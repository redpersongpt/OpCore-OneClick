//! Apple model database (subset relevant to Hackintosh SMBIOS choices),
//! from acidanthera `AppleModels/DataBase` (master 2026-09-30).

use super::model::MacOsVersion;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacKind {
    Desktop,
    Laptop,
    Workstation,
}

#[derive(Debug, Clone, Copy)]
pub struct SmbiosModel {
    pub model: &'static str,
    pub board_id: &'static str,
    /// Oldest macOS (from High Sierra on) this model can install; None = older than 10.13.
    pub min_os: Option<MacOsVersion>,
    /// Newest macOS this model is supported by.
    pub max_os: MacOsVersion,
    /// `AppleModelId` (T2 Secure Boot model, e.g. "j185"); None = x86legacy.
    pub secure_boot_model: Option<&'static str>,
    pub kind: MacKind,
    /// Short description of the real hardware ("iMac 27\" 2020, Comet Lake").
    pub description: &'static str,
}

/// Every model the planner may choose or the UI may offer.
pub fn all() -> &'static [SmbiosModel] {
    todo!()
}

pub fn find(model: &str) -> Option<&'static SmbiosModel> {
    all().iter().find(|m| m.model.eq_ignore_ascii_case(model))
}

/// True when Apple supports `version` on `model` (no board-id skip needed).
pub fn supports(model: &str, version: MacOsVersion) -> bool {
    find(model).is_some_and(|m| m.max_os >= version && m.min_os.is_none_or(|min| min <= version))
}
