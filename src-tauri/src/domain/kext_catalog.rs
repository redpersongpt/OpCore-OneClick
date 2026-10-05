//! Pinned, tested download manifest for OpenCore, OcBinaryData and every
//! kext the planner can select. Pinned entries download from fixed release
//! URLs (no GitHub API, no rate limit) and are verified by SHA-256. When
//! "use latest releases" is on, `latest_*` describes how to find a newer
//! asset through the GitHub API, falling back to the pin on any failure.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    /// Plain zip with bundles at `bundle_root` inside.
    Zip,
    /// GitHub Actions artifact zip containing an inner `*-RELEASE.zip`.
    NestedZip,
    /// The download is a single zip of one `.kext` folder.
    KextZip,
}

#[derive(Debug, Clone, Copy)]
pub struct Pin {
    pub version: &'static str,
    pub url: &'static str,
    /// Lowercase hex SHA-256 of the downloaded file; None only for moving
    /// nightly artifacts (NootRX), which are then validated structurally.
    pub sha256: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct KextCatalogEntry {
    /// Stable id used by `KextSelection::catalog_id` ("Lilu", "VirtualSMC").
    pub id: &'static str,
    /// GitHub "owner/repo" for latest-release resolution (empty if none).
    pub repo: &'static str,
    pub pin: Pin,
    pub archive: ArchiveKind,
    /// Regex matched against release asset names when resolving "latest"
    /// (must select RELEASE, never DEBUG/RESEARCH_RELEASE).
    pub latest_asset_regex: &'static str,
    /// Top-level bundles the archive provides ("VirtualSMC.kext", "SMCProcessor.kext", ...).
    pub bundles: &'static [&'static str],
    pub description: &'static str,
}

pub fn all() -> &'static [KextCatalogEntry] {
    todo!()
}

pub fn entry(id: &str) -> Option<&'static KextCatalogEntry> {
    all().iter().find(|e| e.id == id)
}

/// OpenCorePkg release pin (RELEASE and DEBUG zips share the version).
pub fn opencore_release() -> Pin {
    todo!()
}

pub fn opencore_debug() -> Pin {
    todo!()
}

/// OcBinaryData archive pinned to a commit (Resources for OpenCanopy, HfsPlus.efi).
pub fn ocbinarydata() -> Pin {
    todo!()
}

/// Raw URL of a prebuilt Dortania SSDT ("SSDT-EC-USBX-DESKTOP.aml").
pub fn dortania_ssdt_url(file_name: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/dortania/Getting-Started-With-ACPI/master/extra-files/compiled/{file_name}"
    )
}
