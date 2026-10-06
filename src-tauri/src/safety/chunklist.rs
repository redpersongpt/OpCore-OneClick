//! Apple chunklist (`.chunklist`, magic `CNKL`) parsing and verification, as
//! in OpenCorePkg `macrecovery.py`: a 0x24-byte header, `count` entries of
//! (u32 size, SHA-256) and an RSA-2048 PKCS#1 v1.5 signature over the header
//! and entries, made with Apple EFI ROM key #1.
//!
//! The recovery DMG is served over plain HTTP, so the signed chunklist is
//! what proves the image is Apple's. OpenCore repeats the check at boot with
//! `DmgLoading=Signed`.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::AppError;

const MAGIC: &[u8; 4] = b"CNKL";
const HEADER_SIZE: usize = 0x24;
const ENTRY_SIZE: usize = 0x24;
const RSA_SIGNATURE_SIZE: usize = 256;
const DIGEST_SIGNATURE_SIZE: usize = 32;

/// Apple EFI ROM public key #1 (RSA-2048, e = 65537), big-endian modulus.
const APPLE_KEY_1_MODULUS: &str = "C3E748CAD9CD384329E10E25A91E43E1A762FF529ADE578C935BDDF9B13F2179D4855E6FC89E9E29CA12517D17DFA1EDCE0BEBF0EA7B461FFE61D94E2BDF72C196F89ACD3536B644064014DAE25A15DB6BB0852ECBD120916318D1CCDEA3C84C92ED743FC176D0BACA920D3FCF3158AFF731F88CE0623182A8ED67E650515F75745909F07D415F55FC15A35654D118C55A462D37A3ACDA08612F3F3F6571761EFCCBCC299AEE99B3A4FD6212CCFFF5EF37A2C334E871191F7E1C31960E010A54E86FA3F62E6D6905E1CD57732410A3EB0C6B4DEFDABE9F59BF1618758C751CD56CEF851D1C0EAA1C558E37AC108DA9089863D20E2E7E4BF475EC66FE6B3EFDCF";

/// DER prefix of a SHA-256 DigestInfo (PKCS#1 v1.5).
const SHA256_DIGEST_INFO: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkEntry {
    pub size: u32,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureMethod {
    /// 256-byte RSA-2048 signature (every Apple recovery chunklist).
    Rsa2048,
    /// 32-byte bare digest: no authenticity, rejected like macrecovery does.
    DigestOnly,
}

#[derive(Debug, Clone)]
pub struct Chunklist {
    pub chunks: Vec<ChunkEntry>,
    pub signature_method: SignatureMethod,
    /// SHA-256 over the header and the chunk table.
    digest: [u8; 32],
    signature: Vec<u8>,
}

fn invalid(reason: impl Into<String>) -> AppError {
    AppError::new("CHUNKLIST_INVALID", format!("Recovery chunklist is invalid: {}", reason.into()))
}

fn le_u32(bytes: &[u8], at: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(b)
}

fn le_u64(bytes: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(b)
}

impl Chunklist {
    /// Parse and structurally validate a chunklist (header layout, offsets,
    /// file length). Does not check the signature.
    pub fn parse(bytes: &[u8]) -> Result<Self, AppError> {
        if bytes.len() < HEADER_SIZE {
            return Err(invalid("file is shorter than the header"));
        }
        if &bytes[0..4] != MAGIC {
            return Err(invalid("missing CNKL magic"));
        }
        let header_size = le_u32(bytes, 4);
        let (file_version, chunk_method, signature_method) = (bytes[8], bytes[9], bytes[10]);
        let count = le_u64(bytes, 12);
        let chunk_offset = le_u64(bytes, 20);
        let signature_offset = le_u64(bytes, 28);
        if header_size as usize != HEADER_SIZE || file_version != 1 || chunk_method != 1 {
            return Err(invalid("unsupported header"));
        }
        let (method, signature_size) = match signature_method {
            1 => (SignatureMethod::Rsa2048, RSA_SIGNATURE_SIZE),
            2 => (SignatureMethod::DigestOnly, DIGEST_SIGNATURE_SIZE),
            other => return Err(invalid(format!("unknown signature method {other}"))),
        };
        if count == 0 || chunk_offset != HEADER_SIZE as u64 {
            return Err(invalid("empty chunk table"));
        }
        let table_end = count
            .checked_mul(ENTRY_SIZE as u64)
            .and_then(|t| t.checked_add(HEADER_SIZE as u64))
            .ok_or_else(|| invalid("chunk count overflows"))?;
        if signature_offset != table_end {
            return Err(invalid("signature offset does not follow the chunk table"));
        }
        let expected_len = table_end + signature_size as u64;
        if bytes.len() as u64 != expected_len {
            return Err(invalid(format!("file is {} bytes, expected {expected_len}", bytes.len())));
        }
        let table_end = table_end as usize;
        let chunks = bytes[HEADER_SIZE..table_end]
            .as_chunks::<ENTRY_SIZE>().0.iter()
            .map(|entry| {
                let mut sha256 = [0u8; 32];
                sha256.copy_from_slice(&entry[4..36]);
                ChunkEntry { size: le_u32(entry, 0), sha256 }
            })
            .collect::<Vec<_>>();
        if chunks.iter().any(|c| c.size == 0) {
            return Err(invalid("zero-sized chunk"));
        }
        let digest: [u8; 32] = Sha256::digest(&bytes[..table_end]).into();
        Ok(Self { chunks, signature_method: method, digest, signature: bytes[table_end..].to_vec() })
    }

    /// Size of the image this chunklist describes.
    pub fn total_size(&self) -> u64 {
        self.chunks.iter().map(|c| u64::from(c.size)).sum()
    }

    /// Verify the signature with Apple's EFI ROM key #1.
    pub fn verify_signature(&self) -> Result<(), AppError> {
        let modulus = hex_decode(APPLE_KEY_1_MODULUS).ok_or_else(|| invalid("bad built-in key"))?;
        self.verify_signature_with(&modulus)
    }

    /// Verify the signature against an RSA modulus (big-endian, e = 65537).
    pub fn verify_signature_with(&self, modulus_be: &[u8]) -> Result<(), AppError> {
        match self.signature_method {
            SignatureMethod::DigestOnly => {
                Err(AppError::new("CHUNKLIST_UNSIGNED", "Recovery chunklist has no digital signature"))
            }
            SignatureMethod::Rsa2048 => {
                // PKCS#1 v1.5 needs room for 0x00 0x01, 8+ bytes of 0xFF, 0x00 and the DigestInfo.
                if modulus_be.len() < SHA256_DIGEST_INFO.len() + 32 + 11 {
                    return Err(AppError::new("CHUNKLIST_SIGNATURE", "The signing key is too short"));
                }
                // The signature is stored as a little-endian integer.
                let signature_be: Vec<u8> = self.signature.iter().rev().copied().collect();
                let recovered = rsa::public_op(&signature_be, modulus_be)
                    .ok_or_else(|| AppError::new("CHUNKLIST_SIGNATURE", "Recovery chunklist signature is invalid"))?;
                if recovered == pkcs1_sha256_encoding(&self.digest, modulus_be.len()) {
                    Ok(())
                } else {
                    Err(AppError::new(
                        "CHUNKLIST_SIGNATURE",
                        "Recovery chunklist is not signed by Apple; the download may have been tampered with",
                    ))
                }
            }
        }
    }

    /// Index and offset of the chunk that starts at `offset`, if any.
    fn chunk_at(&self, offset: u64) -> Option<usize> {
        let mut position = 0u64;
        for (index, chunk) in self.chunks.iter().enumerate() {
            if position == offset {
                return Some(index);
            }
            position += u64::from(chunk.size);
        }
        (position == offset).then_some(self.chunks.len())
    }
}

/// EM = 00 01 FF..FF 00 DigestInfo(SHA-256) digest, `len` bytes.
fn pkcs1_sha256_encoding(digest: &[u8; 32], len: usize) -> Vec<u8> {
    let mut em = vec![0xFF; len];
    let tail = SHA256_DIGEST_INFO.len() + digest.len();
    em[0] = 0x00;
    em[1] = 0x01;
    let separator = len - tail - 1;
    em[separator] = 0x00;
    em[separator + 1..separator + 1 + SHA256_DIGEST_INFO.len()].copy_from_slice(&SHA256_DIGEST_INFO);
    em[len - digest.len()..].copy_from_slice(digest);
    em
}

pub fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Streaming verifier: feed the image bytes in order; fails at the first
/// chunk whose hash does not match. `verified_bytes` is always on a chunk
/// boundary, so a download can resume from it.
pub struct ChunkVerifier<'a> {
    list: &'a Chunklist,
    index: usize,
    in_chunk: u64,
    hasher: Sha256,
    verified: u64,
}

impl<'a> ChunkVerifier<'a> {
    pub fn new(list: &'a Chunklist) -> Self {
        Self { list, index: 0, in_chunk: 0, hasher: Sha256::new(), verified: 0 }
    }

    /// Start after an already verified prefix (must be a chunk boundary).
    pub fn resume_at(list: &'a Chunklist, offset: u64) -> Result<Self, AppError> {
        let index = list
            .chunk_at(offset)
            .ok_or_else(|| AppError::new("CHUNKLIST_OFFSET", format!("{offset} is not a chunk boundary")))?;
        Ok(Self { list, index, in_chunk: 0, hasher: Sha256::new(), verified: offset })
    }

    pub fn verified_bytes(&self) -> u64 {
        self.verified
    }

    pub fn is_complete(&self) -> bool {
        self.index == self.list.chunks.len()
    }

    pub fn update(&mut self, mut data: &[u8]) -> Result<(), AppError> {
        while !data.is_empty() {
            let Some(chunk) = self.list.chunks.get(self.index) else {
                return Err(AppError::new("RECOVERY_IMAGE_CORRUPT", "The recovery image is larger than its chunklist"));
            };
            let remaining = u64::from(chunk.size) - self.in_chunk;
            let take = remaining.min(data.len() as u64) as usize;
            self.hasher.update(&data[..take]);
            self.in_chunk += take as u64;
            data = &data[take..];
            if self.in_chunk == u64::from(chunk.size) {
                let digest: [u8; 32] = std::mem::take(&mut self.hasher).finalize().into();
                if digest != chunk.sha256 {
                    return Err(AppError::new(
                        "RECOVERY_IMAGE_CORRUPT",
                        format!("Chunk {} of the recovery image does not match its checksum", self.index + 1),
                    ));
                }
                self.verified += u64::from(chunk.size);
                self.index += 1;
                self.in_chunk = 0;
            }
        }
        Ok(())
    }

    /// The whole image was seen and matched.
    pub fn finish(&self) -> Result<(), AppError> {
        if self.is_complete() && self.in_chunk == 0 {
            Ok(())
        } else {
            Err(AppError::new("RECOVERY_IMAGE_CORRUPT", "The recovery image is shorter than its chunklist"))
        }
    }
}

/// Length of the longest chunk-aligned prefix of `path` that matches the
/// chunklist (0 when the file is missing).
pub fn verified_prefix(path: &Path, list: &Chunklist) -> std::io::Result<u64> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut verifier = ChunkVerifier::new(list);
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 || verifier.update(&buffer[..read]).is_err() {
            break;
        }
    }
    Ok(verifier.verified_bytes())
}

/// Verify a complete image file; `progress` gets the verified byte count.
pub fn verify_file(path: &Path, list: &Chunklist, progress: &dyn Fn(u64)) -> Result<(), AppError> {
    let mut file = std::fs::File::open(path)?;
    let mut verifier = ChunkVerifier::new(list);
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read])?;
        progress(verifier.verified_bytes());
    }
    verifier.finish()
}

/// Minimal fixed-width RSA public operation (s^65537 mod n) with
/// Montgomery multiplication over 32-bit limbs.
mod rsa {
    /// Big-endian bytes → little-endian u32 limbs, `limbs` long.
    fn to_limbs(bytes: &[u8], limbs: usize) -> Option<Vec<u32>> {
        let significant = bytes.iter().skip_while(|b| **b == 0).count();
        if significant > limbs * 4 {
            return None;
        }
        let mut out = vec![0u32; limbs];
        for (i, byte) in bytes.iter().rev().enumerate() {
            if *byte != 0 {
                out[i / 4] |= u32::from(*byte) << ((i % 4) * 8);
            }
        }
        Some(out)
    }

    fn to_bytes(limbs: &[u32], len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        for i in 0..len.min(limbs.len() * 4) {
            out[len - 1 - i] = (limbs[i / 4] >> ((i % 4) * 8)) as u8;
        }
        out
    }

    fn geq(a: &[u32], b: &[u32]) -> bool {
        for i in (0..a.len()).rev() {
            if a[i] != b[i] {
                return a[i] > b[i];
            }
        }
        true
    }

    fn sub_in_place(a: &mut [u32], b: &[u32]) {
        let mut borrow = 0i64;
        for i in 0..a.len() {
            let diff = i64::from(a[i]) - i64::from(b[i]) - borrow;
            if diff < 0 {
                a[i] = (diff + (1i64 << 32)) as u32;
                borrow = 1;
            } else {
                a[i] = diff as u32;
                borrow = 0;
            }
        }
    }

    /// a * b * R^-1 mod n (CIOS). Inputs are < n.
    fn mont_mul(a: &[u32], b: &[u32], n: &[u32], n0inv: u32) -> Vec<u32> {
        let s = n.len();
        let mut t = vec![0u32; s + 2];
        for &bi in b.iter().take(s) {
            let mut carry = 0u64;
            for j in 0..s {
                let x = u64::from(t[j]) + u64::from(a[j]) * u64::from(bi) + carry;
                t[j] = x as u32;
                carry = x >> 32;
            }
            let x = u64::from(t[s]) + carry;
            t[s] = x as u32;
            t[s + 1] = (x >> 32) as u32;

            let m = t[0].wrapping_mul(n0inv);
            let x = u64::from(t[0]) + u64::from(m) * u64::from(n[0]);
            let mut carry = x >> 32;
            for j in 1..s {
                let x = u64::from(t[j]) + u64::from(m) * u64::from(n[j]) + carry;
                t[j - 1] = x as u32;
                carry = x >> 32;
            }
            let x = u64::from(t[s]) + carry;
            t[s - 1] = x as u32;
            t[s] = t[s + 1] + (x >> 32) as u32;
            t[s + 1] = 0;
        }
        let mut result = t[..s].to_vec();
        if t[s] != 0 || geq(&result, n) {
            sub_in_place(&mut result, n);
        }
        result
    }

    /// x * 2 mod n for x < n.
    fn double_mod(x: &mut [u32], n: &[u32]) {
        let mut carry = 0u32;
        for limb in x.iter_mut() {
            let next = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
        if carry != 0 || geq(x, n) {
            sub_in_place(x, n);
        }
    }

    /// base^exponent mod modulus. All values big-endian; the result has the
    /// modulus' length. `None` for an even/zero modulus or base >= modulus.
    pub fn mod_pow(base: &[u8], exponent: &[u8], modulus: &[u8]) -> Option<Vec<u8>> {
        let len = modulus.len();
        let limbs = len.div_ceil(4);
        let n = to_limbs(modulus, limbs)?;
        if n[0] & 1 == 0 || n.iter().all(|l| *l == 0) {
            return None;
        }
        let b = to_limbs(base, limbs)?;
        if geq(&b, &n) {
            return None;
        }
        // n0inv = -n^-1 mod 2^32 (Newton iteration).
        let mut inv: u32 = 1;
        for _ in 0..5 {
            inv = inv.wrapping_mul(2u32.wrapping_sub(n[0].wrapping_mul(inv)));
        }
        let n0inv = inv.wrapping_neg();
        // R^2 mod n by doubling 1 (2 * 32 * limbs) times.
        let mut r2 = vec![0u32; limbs];
        r2[0] = 1;
        for _ in 0..(64 * limbs) {
            double_mod(&mut r2, &n);
        }
        let mut one = vec![0u32; limbs];
        one[0] = 1;
        let base_m = mont_mul(&b, &r2, &n, n0inv);
        let mut acc = mont_mul(&one, &r2, &n, n0inv);
        for byte in exponent {
            for bit in (0..8).rev() {
                acc = mont_mul(&acc, &acc, &n, n0inv);
                if (byte >> bit) & 1 == 1 {
                    acc = mont_mul(&acc, &base_m, &n, n0inv);
                }
            }
        }
        let result = mont_mul(&acc, &one, &n, n0inv);
        Some(to_bytes(&result, len))
    }

    /// RSA verification primitive with e = 65537.
    pub fn public_op(signature: &[u8], modulus: &[u8]) -> Option<Vec<u8>> {
        mod_pow(signature, &[0x01, 0x00, 0x01], modulus)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// RSA-2048 key made for these tests only (e = 65537).
    const TEST_MODULUS: &str = "c0665fa5b3cbc5c02d628b692039ce97c5c199ecd7362849210ae132f0e85621e1a9718cd07528b265762d61c88a2bb97ea50fe1e63f1527eb1a0bb939b0c187\
        470de91bbe1d733da3074e84a646eab569e42e936329d35f14696e139b6ade6237445732e769b7bddfbd05b509cf78f432acb1458a517d6186cc7cd3a214efb1\
        d94c2ddc34a6ffd0094fbded5cfb01ddc7a5359197540991fb5f60366097a5f3e548bbda4d4313e7b73396f4d24522038c0bb561b33db568f182de2aa36004fd\
        19396f214af774dd176c77a98beb6a46faf896a72bb745542a7e1d580435ad81b2a593a356731cc1ad03a6d5a3763d35ae735800a3afdd37a0f2d594edb33609";
    const TEST_PRIVATE_EXPONENT: &str = "a6ad49791c8c90910f004af3d4961fb27e005d5fbf854c5b2603edda1ab7bc3e77e739d69a9494a00fa3d466dcbb4e6bd11a1feb3c7333d9b423893a7a8ef4e9\
        4395fa772d390827c27f46f745b1340ddb617133fff931033284af76cef2431b64f3907329e4fce7c1d75805612d5a847b0dfe38d73e0757a0d6afe10b8e05a3\
        52ab3f28db2322c7384f650fd3402224318c99bcfbc79dffca258d50437906241a83d96533f4acd1276731cb98e3b37b9c16f5a78bfe189363494b0a026f14be\
        9c1bc722b0e5165862a6aa98808f80aa4192849f56b849ee5901e4cabaae875a66ff3cf1e6c06e236858e36b363b42a0ea0324671f3b12767231a88384a38f8d";

    fn hex(text: &str) -> Vec<u8> {
        let compact: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        hex_decode(&compact).unwrap()
    }

    /// Build a chunklist for `image` split into `chunk` sized pieces, signed
    /// with the test key (method 1) or carrying a bare digest (method 2).
    pub(crate) fn build_chunklist(image: &[u8], chunk: usize, method: u8) -> Vec<u8> {
        let pieces: Vec<&[u8]> = image.chunks(chunk).collect();
        let count = pieces.len() as u64;
        let mut out = Vec::new();
        out.extend_from_slice(b"CNKL");
        out.extend_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        out.extend_from_slice(&[1, 1, method, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&(HEADER_SIZE as u64).to_le_bytes());
        out.extend_from_slice(&(HEADER_SIZE as u64 + ENTRY_SIZE as u64 * count).to_le_bytes());
        for piece in pieces {
            out.extend_from_slice(&(piece.len() as u32).to_le_bytes());
            out.extend_from_slice(&Sha256::digest(piece));
        }
        let digest: [u8; 32] = Sha256::digest(&out).into();
        if method == 1 {
            let modulus = hex(TEST_MODULUS);
            let em = pkcs1_sha256_encoding(&digest, modulus.len());
            let signature_be = rsa::mod_pow(&em, &hex(TEST_PRIVATE_EXPONENT), &modulus).unwrap();
            out.extend(signature_be.iter().rev());
        } else {
            out.extend_from_slice(&digest);
        }
        out
    }

    pub(crate) fn test_modulus() -> Vec<u8> {
        hex(TEST_MODULUS)
    }

    fn sample_image(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn mod_pow_matches_reference_values() {
        // Small numbers against u128 arithmetic.
        let modulus: u128 = 0xF123_4567_89AB_CDEF;
        for (base, exp) in [(2u128, 10u32), (0x1234_5678, 65537), (modulus - 1, 3)] {
            let mut expected = 1u128;
            for _ in 0..exp {
                expected = expected * base % modulus;
            }
            let got = rsa::mod_pow(&base.to_be_bytes()[8..], &exp.to_be_bytes(), &modulus.to_be_bytes()[8..]).unwrap();
            let mut wide = [0u8; 16];
            wide[8..].copy_from_slice(&got);
            assert_eq!(u128::from_be_bytes(wide), expected);
        }
        // Apple's key against a value computed with Python's pow().
        let mut base = Vec::new();
        let mut i = 0;
        while base.len() < 256 {
            base.extend_from_slice(&Sha256::digest(format!("opcore-{i}").as_bytes()));
            i += 1;
        }
        base[0] = 0x12;
        let modulus = hex_decode(APPLE_KEY_1_MODULUS).unwrap();
        let expected = hex(
            "260843da43a53facf375b421cb5355758632c82c0f56cc877dd73bdce687abeb69f473a1f61021db358d916d59d738f0b6ee119fad549ad2d2861bf55009cd0e\
             4a1585c9dd9356591a3109747181451c8ca282cfe8aed3f8266e2df0719fc20ea6e83cefdb625cb76f1abb177a9b09837336888f57fa7bdf7216cf8da0d433bb\
             49a89f9d6ab9c7493943a53fb120a38c214dd8c0571098ab8536af9e82407768c0e5f1910f64ed9c75ccc14e8045a110cb494dbfedc9a2c3d266a06e909e59d6\
             896506860f9f0ec957ae55489079553eb8d8b925476110485faae0ffc2aedf5dd7b38f2bdbd602458f1fe4e8f81a168b3d1456c97469ae79136f276a430b92ba",
        );
        assert_eq!(rsa::public_op(&base, &modulus).unwrap(), expected);
        // Values outside the group are rejected.
        assert!(rsa::public_op(&[0xFF; 256], &modulus).is_none());
        assert!(rsa::mod_pow(&[1], &[1], &[0x10]).is_none());
    }

    #[test]
    fn parses_and_verifies_a_signed_chunklist() {
        let image = sample_image(10 * 1024 + 123);
        let bytes = build_chunklist(&image, 1024, 1);
        assert_eq!(bytes.len(), 36 + 36 * 11 + 256);
        let list = Chunklist::parse(&bytes).unwrap();
        assert_eq!(list.chunks.len(), 11);
        assert_eq!(list.total_size(), image.len() as u64);
        assert_eq!(list.chunks[10].size, 123);
        list.verify_signature_with(&test_modulus()).unwrap();
        // Signed by a different key than Apple's.
        assert_eq!(list.verify_signature().unwrap_err().code, "CHUNKLIST_SIGNATURE");
        assert_eq!(list.verify_signature_with(&[0xC3; 16]).unwrap_err().code, "CHUNKLIST_SIGNATURE");
    }

    #[test]
    fn tampering_breaks_the_signature() {
        let image = sample_image(4096);
        let mut bytes = build_chunklist(&image, 1024, 1);
        bytes[40] ^= 0x01; // inside the first chunk hash
        let list = Chunklist::parse(&bytes).unwrap();
        assert_eq!(list.verify_signature_with(&test_modulus()).unwrap_err().code, "CHUNKLIST_SIGNATURE");
        let mut bytes = build_chunklist(&image, 1024, 1);
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        let list = Chunklist::parse(&bytes).unwrap();
        assert!(list.verify_signature_with(&test_modulus()).is_err());
    }

    #[test]
    fn unsigned_chunklists_are_rejected() {
        let image = sample_image(3000);
        let bytes = build_chunklist(&image, 1024, 2);
        let list = Chunklist::parse(&bytes).unwrap();
        assert_eq!(list.signature_method, SignatureMethod::DigestOnly);
        assert_eq!(list.verify_signature().unwrap_err().code, "CHUNKLIST_UNSIGNED");
    }

    #[test]
    fn malformed_headers_are_rejected() {
        let good = build_chunklist(&sample_image(2048), 1024, 2);
        assert!(Chunklist::parse(&good).is_ok());
        assert!(Chunklist::parse(&good[..20]).is_err());
        let mut bad = good.clone();
        bad[0] = b'X';
        assert!(Chunklist::parse(&bad).is_err());
        let mut bad = good.clone();
        bad[8] = 2; // file version
        assert!(Chunklist::parse(&bad).is_err());
        let mut bad = good.clone();
        bad[10] = 3; // signature method
        assert!(Chunklist::parse(&bad).is_err());
        let mut bad = good.clone();
        bad[12..20].copy_from_slice(&u64::MAX.to_le_bytes()); // count overflow
        assert!(Chunklist::parse(&bad).is_err());
        let mut bad = good.clone();
        bad.push(0); // trailing data
        assert!(Chunklist::parse(&bad).is_err());
        let mut bad = good;
        bad[36..40].copy_from_slice(&0u32.to_le_bytes()); // zero-sized chunk
        assert!(Chunklist::parse(&bad).is_err());
    }

    #[test]
    fn streaming_verifier_accepts_any_split() {
        let image = sample_image(5000);
        let list = Chunklist::parse(&build_chunklist(&image, 1000, 2)).unwrap();
        for step in [1usize, 7, 999, 1000, 1001, 5000] {
            let mut verifier = ChunkVerifier::new(&list);
            for piece in image.chunks(step) {
                verifier.update(piece).unwrap();
            }
            verifier.finish().unwrap();
            assert_eq!(verifier.verified_bytes(), 5000);
        }
    }

    #[test]
    fn streaming_verifier_reports_corruption_and_size_errors() {
        let image = sample_image(5000);
        let list = Chunklist::parse(&build_chunklist(&image, 1000, 2)).unwrap();
        let mut corrupt = image.clone();
        corrupt[2500] ^= 0xFF;
        let mut verifier = ChunkVerifier::new(&list);
        let err = verifier.update(&corrupt).unwrap_err();
        assert_eq!(err.code, "RECOVERY_IMAGE_CORRUPT");
        assert_eq!(verifier.verified_bytes(), 2000);

        let mut verifier = ChunkVerifier::new(&list);
        verifier.update(&image[..4500]).unwrap();
        assert!(verifier.finish().is_err());

        let mut longer = image.clone();
        longer.push(0);
        let mut verifier = ChunkVerifier::new(&list);
        assert!(verifier.update(&longer).is_err());
    }

    #[test]
    fn resume_requires_a_chunk_boundary() {
        let image = sample_image(5000);
        let list = Chunklist::parse(&build_chunklist(&image, 1000, 2)).unwrap();
        let mut verifier = ChunkVerifier::resume_at(&list, 3000).unwrap();
        verifier.update(&image[3000..]).unwrap();
        verifier.finish().unwrap();
        assert!(ChunkVerifier::resume_at(&list, 3001).is_err());
        assert!(ChunkVerifier::resume_at(&list, 5000).unwrap().is_complete());
    }

    #[test]
    fn file_prefix_and_full_verification() {
        let dir = std::env::temp_dir().join(format!("opcore-cnk-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = sample_image(5000);
        let list = Chunklist::parse(&build_chunklist(&image, 1000, 2)).unwrap();
        let path = dir.join("BaseSystem.dmg");
        assert_eq!(verified_prefix(&path, &list).unwrap(), 0);

        let mut partial = image[..3500].to_vec();
        partial[3200] ^= 1; // torn tail inside the 4th chunk
        std::fs::write(&path, &partial).unwrap();
        assert_eq!(verified_prefix(&path, &list).unwrap(), 3000);

        std::fs::write(&path, &image).unwrap();
        let seen = std::sync::atomic::AtomicU64::new(0);
        verify_file(&path, &list, &|n| seen.store(n, std::sync::atomic::Ordering::Relaxed)).unwrap();
        assert_eq!(seen.load(std::sync::atomic::Ordering::Relaxed), 5000);

        std::fs::write(&path, &image[..4999]).unwrap();
        assert!(verify_file(&path, &list, &|_| {}).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Manual check against a chunklist downloaded from Apple:
    /// `OPCORE_CHUNKLIST=/path/BaseSystem.chunklist cargo test -- --ignored real_apple_chunklist`
    #[test]
    #[ignore]
    fn real_apple_chunklist() {
        let bytes = std::fs::read(std::env::var("OPCORE_CHUNKLIST").unwrap()).unwrap();
        let list = Chunklist::parse(&bytes).unwrap();
        assert!(list.total_size() > 0);
        list.verify_signature().unwrap();
        let mut tampered = bytes.clone();
        tampered[100] ^= 1;
        assert!(Chunklist::parse(&tampered).unwrap().verify_signature().is_err());
    }

    #[test]
    fn hex_helpers() {
        assert_eq!(hex_decode("00ff10").unwrap(), vec![0, 255, 16]);
        assert!(hex_decode("0").is_none());
        assert!(hex_decode("zz").is_none());
        assert_eq!(hex_encode(&[0xAB, 0x01]), "ab01");
    }
}
