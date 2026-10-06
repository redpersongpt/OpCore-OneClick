//! Flash confirmation tokens.
//!
//! Token format: `flashconf.<session_id>.<base64url claims>.<base64url HMAC-SHA256>`.
//! A token binds the canonical device path, the disk fingerprint, the EFI
//! folder and its content hash, and the recovery image choice. It is single
//! use and expires after five minutes of monotonic time (wall-clock changes
//! do not extend it).
//!
//! The token protects against stale or raced confirmations (the disk or the
//! EFI changed after the user confirmed, a second click, a replay); it does
//! not protect against a compromised renderer, which could request its own
//! token.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tracing::{info, warn};

use super::disk_identity::{compare_fingerprints, DiskIdentityFingerprint};
use crate::error::AppError;

type HmacSha256 = Hmac<Sha256>;

/// Lifetime of a confirmation token.
pub const FLASH_CONFIRMATION_TTL: Duration = Duration::from_secs(5 * 60);

/// Current token version.
pub const FLASH_CONFIRMATION_TOKEN_VERSION: u32 = 3;

const TOKEN_PREFIX: &str = "flashconf";

/// What a confirmation is about. Built once when the user confirms and again
/// right before the flash; both must be equal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashBinding {
    /// Canonical device path from the disk listing.
    pub device: String,
    pub disk_fingerprint: DiskIdentityFingerprint,
    /// Canonical directory that contains `EFI`.
    pub efi_path: String,
    /// Hash over every file of the EFI folder.
    pub efi_state_hash: String,
    /// Recovery release id ("15", "26"), if a recovery image is written.
    pub recovery: Option<String>,
    /// Identity of the verified recovery files, if any.
    pub payload_state_hash: Option<String>,
}

/// Claims embedded in a flash confirmation token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlashConfirmationClaims {
    pub version: u32,
    pub session_id: String,
    pub nonce: String,
    /// Unix milliseconds (informational; expiry is enforced with `Instant`).
    pub issued_at: i64,
    pub expires_at: i64,
    pub device: String,
    pub disk_fingerprint: DiskIdentityFingerprint,
    pub efi_path: String,
    pub efi_state_hash: String,
    pub recovery: Option<String>,
    pub payload_state_hash: Option<String>,
}

impl FlashConfirmationClaims {
    /// Check that the confirmed state equals the current state.
    pub fn check_binding(&self, current: &FlashBinding) -> Result<(), AppError> {
        if self.device != current.device {
            return Err(AppError::new("CONFIRMATION_DEVICE_MISMATCH", "The confirmation was issued for a different disk")
                .with_suggestion("Select the disk again and confirm."));
        }
        let comparison = compare_fingerprints(&self.disk_fingerprint, &current.disk_fingerprint);
        if !comparison.matches {
            return Err(AppError::new(
                "DISK_IDENTITY_CHANGED",
                format!(
                    "The disk at {} is not the one you confirmed (changed: {})",
                    current.device,
                    comparison.mismatched_fields.join(", ")
                ),
            )
            .with_suggestion("Re-select the USB drive and confirm again."));
        }
        if self.efi_path != current.efi_path || self.efi_state_hash != current.efi_state_hash {
            return Err(AppError::new("EFI_CHANGED", "The EFI folder changed after you confirmed the flash")
                .with_suggestion("Review the build and confirm again."));
        }
        if self.recovery != current.recovery || self.payload_state_hash != current.payload_state_hash {
            return Err(AppError::new("RECOVERY_CHANGED", "The recovery image changed after you confirmed the flash")
                .with_suggestion("Confirm the flash again."));
        }
        Ok(())
    }
}

/// A freshly issued token.
#[derive(Debug, Clone)]
pub struct IssuedToken {
    pub token: String,
    /// Unix milliseconds, for display.
    pub expires_at: i64,
}

/// Holds the secret key and session ID for flash token operations.
/// One instance per app lifetime.
pub struct FlashSecurityContext {
    secret: [u8; 32],
    session_id: String,
    /// Outstanding nonces and when they were issued. A nonce is removed when
    /// redeemed, so a token works once.
    issued: Mutex<HashMap<String, Instant>>,
}

impl FlashSecurityContext {
    /// Create a new security context with a random 32-byte secret.
    pub fn new(session_id: String) -> Arc<Self> {
        let mut secret = [0u8; 32];
        rand::rng().fill(&mut secret[..]);
        info!(session_id = %session_id, "Flash confirmation context ready");
        Arc::new(Self { secret, session_id, issued: Mutex::new(HashMap::new()) })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Issue a single-use token for `binding`.
    pub fn issue_token(&self, binding: &FlashBinding) -> Result<IssuedToken, AppError> {
        self.issue_at(binding, Instant::now())
    }

    fn issue_at(&self, binding: &FlashBinding, now: Instant) -> Result<IssuedToken, AppError> {
        let wall = chrono::Utc::now().timestamp_millis();
        let expires_at = wall + FLASH_CONFIRMATION_TTL.as_millis() as i64;
        let nonce = generate_nonce();
        let claims = FlashConfirmationClaims {
            version: FLASH_CONFIRMATION_TOKEN_VERSION,
            session_id: self.session_id.clone(),
            nonce: nonce.clone(),
            issued_at: wall,
            expires_at,
            device: binding.device.clone(),
            disk_fingerprint: binding.disk_fingerprint.clone(),
            efi_path: binding.efi_path.clone(),
            efi_state_hash: binding.efi_state_hash.clone(),
            recovery: binding.recovery.clone(),
            payload_state_hash: binding.payload_state_hash.clone(),
        };
        let token = sign_claims(&self.secret, &claims)?;
        let mut issued = self.issued.lock().map_err(|_| poisoned())?;
        issued.retain(|_, at| now.saturating_duration_since(*at) < FLASH_CONFIRMATION_TTL);
        issued.insert(nonce, now);
        info!(device = %binding.device, "Flash confirmation token issued");
        Ok(IssuedToken { token, expires_at })
    }

    /// Verify a token (format, signature, session, version) and consume it.
    /// Fails if it was already used or is older than the TTL.
    pub fn redeem_token(&self, token: &str) -> Result<FlashConfirmationClaims, AppError> {
        self.redeem_at(token, Instant::now())
    }

    fn redeem_at(&self, token: &str, now: Instant) -> Result<FlashConfirmationClaims, AppError> {
        let claims = verify_token(token, &self.secret, &self.session_id)?;
        let mut issued = self.issued.lock().map_err(|_| poisoned())?;
        let Some(issued_at) = issued.remove(&claims.nonce) else {
            warn!("Flash token replayed or unknown");
            return Err(AppError::new("CONFIRMATION_CONSUMED", "This confirmation was already used")
                .with_suggestion("Confirm the flash again."));
        };
        if now.saturating_duration_since(issued_at) >= FLASH_CONFIRMATION_TTL {
            return Err(AppError::new("CONFIRMATION_EXPIRED", "The confirmation expired")
                .recoverable()
                .with_suggestion("Confirm the flash again."));
        }
        info!(device = %claims.device, "Flash confirmation token redeemed");
        Ok(claims)
    }
}

fn poisoned() -> AppError {
    AppError::new("INTERNAL_ERROR", "Flash confirmation state is unavailable")
}

/// 24 hex characters.
fn generate_nonce() -> String {
    let mut bytes = [0u8; 12];
    rand::rng().fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Claims as JSON with sorted keys.
fn serialize_claims(claims: &FlashConfirmationClaims) -> Result<String, AppError> {
    let value = serde_json::to_value(claims)?;
    let sorted: BTreeMap<String, serde_json::Value> = match value {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => return Err(AppError::new("SERIALIZE_ERROR", "claims are not an object")),
    };
    Ok(serde_json::to_string(&sorted)?)
}

fn mac_for(secret: &[u8], payload: &str) -> Result<HmacSha256, AppError> {
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|e| AppError::new("HMAC_ERROR", e.to_string()))?;
    mac.update(payload.as_bytes());
    Ok(mac)
}

fn sign_claims(secret: &[u8], claims: &FlashConfirmationClaims) -> Result<String, AppError> {
    let payload = URL_SAFE_NO_PAD.encode(serialize_claims(claims)?.as_bytes());
    let signature = mac_for(secret, &payload)?.finalize().into_bytes();
    Ok(format!("{TOKEN_PREFIX}.{}.{payload}.{}", claims.session_id, URL_SAFE_NO_PAD.encode(signature)))
}

fn malformed() -> AppError {
    AppError::new("CONFIRMATION_MALFORMED", "The flash confirmation is malformed")
}

/// Structure, signature (constant time), session and version.
fn verify_token(token: &str, secret: &[u8], session_id: &str) -> Result<FlashConfirmationClaims, AppError> {
    let parts: Vec<&str> = token.split('.').collect();
    let [prefix, header_session, payload, signature] = parts.as_slice() else { return Err(malformed()) };
    if *prefix != TOKEN_PREFIX {
        return Err(malformed());
    }
    if *header_session != session_id {
        return Err(AppError::new("CONFIRMATION_SESSION_CHANGED", "The confirmation belongs to an earlier app session")
            .with_suggestion("Confirm the flash again."));
    }
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| malformed())?;
    mac_for(secret, payload)?
        .verify_slice(&signature)
        .map_err(|_| AppError::new("CONFIRMATION_SIGNATURE_INVALID", "The flash confirmation was altered"))?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| malformed())?;
    let claims: FlashConfirmationClaims = serde_json::from_slice(&bytes).map_err(|_| malformed())?;
    if claims.session_id != session_id {
        return Err(AppError::new("CONFIRMATION_SESSION_CHANGED", "The confirmation belongs to an earlier app session"));
    }
    if claims.version != FLASH_CONFIRMATION_TOKEN_VERSION || claims.nonce.len() < 16 {
        return Err(malformed());
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> FlashBinding {
        FlashBinding {
            device: "/dev/sdb".into(),
            disk_fingerprint: DiskIdentityFingerprint {
                serial_number: Some("4c53".into()),
                device_path: Some("/dev/sdb".into()),
                size_bytes: Some(32_000_000_000),
                model: Some("ultra".into()),
                removable: Some(true),
                ..Default::default()
            },
            efi_path: "/data/builds/abc".into(),
            efi_state_hash: "aa".repeat(32),
            recovery: Some("26".into()),
            payload_state_hash: Some("bb".repeat(32)),
        }
    }

    #[test]
    fn issue_and_redeem_once() {
        let ctx = FlashSecurityContext::new("session-1".into());
        let issued = ctx.issue_token(&binding()).unwrap();
        assert!(issued.token.starts_with("flashconf.session-1."));
        let claims = ctx.redeem_token(&issued.token).unwrap();
        claims.check_binding(&binding()).unwrap();
        assert_eq!(claims.recovery.as_deref(), Some("26"));
        assert_eq!(ctx.redeem_token(&issued.token).unwrap_err().code, "CONFIRMATION_CONSUMED");
    }

    #[test]
    fn expiry_uses_monotonic_time() {
        let ctx = FlashSecurityContext::new("s".into());
        let start = Instant::now();
        let issued = ctx.issue_at(&binding(), start).unwrap();
        let late = start + FLASH_CONFIRMATION_TTL + Duration::from_secs(1);
        assert_eq!(ctx.redeem_at(&issued.token, late).unwrap_err().code, "CONFIRMATION_EXPIRED");
        // An expired token is gone, not reusable.
        assert_eq!(ctx.redeem_at(&issued.token, start).unwrap_err().code, "CONFIRMATION_CONSUMED");
    }

    #[test]
    fn tampering_and_foreign_tokens_are_rejected() {
        let ctx = FlashSecurityContext::new("s".into());
        let issued = ctx.issue_token(&binding()).unwrap();
        let parts: Vec<&str> = issued.token.split('.').collect();

        // Swap in claims for another device, keep the old signature.
        let mut claims: FlashConfirmationClaims =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
        claims.device = "/dev/sda".into();
        let forged_payload = URL_SAFE_NO_PAD.encode(serialize_claims(&claims).unwrap());
        let forged = format!("{}.{}.{}.{}", parts[0], parts[1], forged_payload, parts[3]);
        assert_eq!(ctx.redeem_token(&forged).unwrap_err().code, "CONFIRMATION_SIGNATURE_INVALID");

        assert_eq!(ctx.redeem_token("garbage").unwrap_err().code, "CONFIRMATION_MALFORMED");
        assert_eq!(ctx.redeem_token("flashconf.s.x.y").unwrap_err().code, "CONFIRMATION_MALFORMED");

        let other = FlashSecurityContext::new("other".into());
        assert_eq!(other.redeem_token(&issued.token).unwrap_err().code, "CONFIRMATION_SESSION_CHANGED");
        // Same session id but a different secret (app restarted with a reused id).
        let restarted = FlashSecurityContext::new("s".into());
        assert_eq!(restarted.redeem_token(&issued.token).unwrap_err().code, "CONFIRMATION_SIGNATURE_INVALID");
    }

    #[test]
    fn binding_checks_every_part() {
        let ctx = FlashSecurityContext::new("s".into());
        let claims = ctx.redeem_token(&ctx.issue_token(&binding()).unwrap().token).unwrap();

        let mut changed = binding();
        changed.device = "/dev/sdc".into();
        assert_eq!(claims.check_binding(&changed).unwrap_err().code, "CONFIRMATION_DEVICE_MISMATCH");

        let mut changed = binding();
        changed.disk_fingerprint.serial_number = None;
        assert_eq!(claims.check_binding(&changed).unwrap_err().code, "DISK_IDENTITY_CHANGED");

        let mut changed = binding();
        changed.efi_state_hash = "cc".repeat(32);
        assert_eq!(claims.check_binding(&changed).unwrap_err().code, "EFI_CHANGED");

        let mut changed = binding();
        changed.efi_path = "/data/builds/other".into();
        assert_eq!(claims.check_binding(&changed).unwrap_err().code, "EFI_CHANGED");

        let mut changed = binding();
        changed.recovery = None;
        changed.payload_state_hash = None;
        assert_eq!(claims.check_binding(&changed).unwrap_err().code, "RECOVERY_CHANGED");

        let mut changed = binding();
        changed.payload_state_hash = Some("dd".repeat(32));
        assert_eq!(claims.check_binding(&changed).unwrap_err().code, "RECOVERY_CHANGED");
    }

    #[test]
    fn claims_serialize_with_sorted_keys() {
        let ctx = FlashSecurityContext::new("s".into());
        let issued = ctx.issue_token(&binding()).unwrap();
        let payload = issued.token.split('.').nth(2).unwrap();
        let json = String::from_utf8(URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        let device = json.find("\"device\"").unwrap();
        let version = json.find("\"version\"").unwrap();
        assert!(device < version);
    }
}
