//! SSDT generation (Dortania "Getting started with ACPI" / SSDTTime logic).

use crate::domain::model::{AcpiFacts, AcpiPatch, SsdtSource};
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsdtKind {
    /// Fake EC (desktop: disable real EC named EC; laptop: keep real EC) + USBX power properties.
    EcUsbx { laptop: bool },
    /// plugin-type=1 on the first CPU object (SSDT-PLUG / PLUG-ALT for ACPI0007).
    Plug,
    /// AWAC → legacy RTC (STAS=1 or fake RTC0).
    Awac,
    /// PMCR device for NVRAM on 300-series Intel.
    Pmc,
    /// Disable RHUB so macOS rebuilds ports (Comet Lake+/AMD where needed).
    RhubReset,
    /// `_OSI` → XOSI rename + Windows-compatible XOSI method (laptops, I2C).
    Xosi,
    /// Enable GPIO controller for I2C touchpads (GPI0 _STA).
    Gpi0,
    /// Backlight PNLF device; `uid` per iGPU generation (14 SNB/IVB, 15 HSW/BDW, 16 SKL/KBL, 19 CFL+).
    Pnlf { uid: u32 },
    /// Disable unused uncore bridges on X79/X99.
    Unc,
    /// RTC0 with fixed IO ranges for HEDT boards.
    Rtc0Range,
    /// Processor objects for B550/A520 boards declaring CPUs as ACPI0007.
    Cpur,
    /// Fake IMEI for Sandy/Ivy Bridge on 7/6-series mismatched boards.
    Imei,
    /// SBUS / MCHC for SMBus.
    SbusMchc,
    /// Fake ambient light sensor ALS0.
    Als0,
}

impl SsdtKind {
    /// Canonical output file name ("SSDT-EC-USBX.aml").
    pub fn file_name(&self) -> &'static str {
        todo!("file_name {self:?}")
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedSsdt {
    pub file_name: String,
    pub aml: Vec<u8>,
    /// Equivalent ASL source, for display and support.
    pub dsl: String,
    /// ACPI renames this SSDT depends on (e.g. `_OSI` → `XOSI`, `EC` → `EC0`).
    pub patches: Vec<AcpiPatch>,
}

/// Generate one SSDT for this machine. Returns an error when the facts are
/// insufficient (caller then uses `fallback_prebuilt`).
pub fn generate(kind: &SsdtKind, facts: &AcpiFacts) -> Result<GeneratedSsdt, AppError> {
    todo!("generate {kind:?} {:?}", facts.lpc_bridge)
}

/// Prebuilt fallback (OpenCorePkg AcpiSamples or Dortania compiled) used when
/// no DSDT is available. None when no safe generic table exists.
pub fn fallback_prebuilt(kind: &SsdtKind) -> Option<SsdtSource> {
    todo!("fallback_prebuilt {kind:?}")
}
