use std::collections::HashSet;

use super::ids::{self, Kind};
use super::*;
use crate::domain::model::{GpuFamily, GpuVendor, MacOsVersion, ProfileGpu};

use GpuFamily::*;
use MacOsVersion::*;

fn from_ids(vendor: &str, device: &str, name: &str) -> ProfileGpu {
    let id = identify(Some(vendor), Some(device), name);
    ProfileGpu {
        name: name.to_string(),
        vendor: id.vendor,
        family: id.family,
        vendor_id: Some(vendor.to_string()),
        device_id: Some(device.to_string()),
        is_igpu: id.is_igpu,
        ..Default::default()
    }
}

fn from_name(name: &str) -> ProfileGpu {
    let id = identify(None, None, name);
    ProfileGpu {
        name: name.to_string(),
        vendor: id.vendor,
        family: id.family,
        is_igpu: id.is_igpu,
        ..Default::default()
    }
}

fn dev(vendor: &str, device: &str) -> ProfileGpu {
    from_ids(vendor, device, "")
}

fn native_range(gpu: &ProfileGpu) -> Vec<MacOsVersion> {
    MacOsVersion::ALL
        .into_iter()
        .filter(|v| natively_supported_on(gpu, *v))
        .collect()
}

fn spoof(id: u16) -> GpuRequirement {
    let [lo, hi] = id.to_le_bytes();
    GpuRequirement::DeviceIdSpoof([lo, hi, 0, 0])
}

// ── Identification by PCI id ────────────────────────────────────────────────

#[test]
fn identifies_by_pci_id() {
    let cases: &[(&str, &str, GpuFamily, bool)] = &[
        // Intel iGPUs
        ("8086", "0046", IntelIronLake, true),
        ("8086", "0042", IntelIronLake, true),
        ("8086", "0102", IntelSandyBridge, true),
        ("8086", "0106", IntelSandyBridge, true),
        ("8086", "010a", IntelSandyBridge, true),
        ("8086", "0112", IntelSandyBridge, true),
        ("8086", "0116", IntelSandyBridge, true),
        ("8086", "0122", IntelSandyBridge, true),
        ("8086", "0126", IntelSandyBridge, true),
        ("8086", "0152", IntelIvyBridge, true),
        ("8086", "0162", IntelIvyBridge, true),
        ("8086", "0166", IntelIvyBridge, true),
        ("8086", "016a", IntelIvyBridge, true),
        ("8086", "0402", IntelHaswell, true),
        ("8086", "0412", IntelHaswell, true),
        ("8086", "0416", IntelHaswell, true),
        ("8086", "0a16", IntelHaswell, true),
        ("8086", "0a2e", IntelHaswell, true),
        ("8086", "0d22", IntelHaswell, true),
        ("8086", "1606", IntelBroadwell, true),
        ("8086", "1616", IntelBroadwell, true),
        ("8086", "1622", IntelBroadwell, true),
        ("8086", "1626", IntelBroadwell, true),
        ("8086", "1902", IntelSkylake, true),
        ("8086", "1912", IntelSkylake, true),
        ("8086", "1916", IntelSkylake, true),
        ("8086", "191e", IntelSkylake, true),
        ("8086", "193b", IntelSkylake, true),
        ("8086", "5902", IntelKabyLake, true),
        ("8086", "5912", IntelKabyLake, true),
        ("8086", "5916", IntelKabyLake, true),
        ("8086", "5917", IntelKabyLake, true),
        ("8086", "591c", IntelKabyLake, true),
        ("8086", "87c0", IntelKabyLake, true),
        ("8086", "3e90", IntelCoffeeLake, true),
        ("8086", "3e92", IntelCoffeeLake, true),
        ("8086", "3e98", IntelCoffeeLake, true),
        ("8086", "3e9b", IntelCoffeeLake, true),
        ("8086", "3ea0", IntelCoffeeLake, true),
        ("8086", "3ea5", IntelCoffeeLake, true),
        ("8086", "9b41", IntelCometLake, true),
        ("8086", "9ba8", IntelCometLake, true),
        ("8086", "9bc5", IntelCometLake, true),
        ("8086", "9bc8", IntelCometLake, true),
        ("8086", "9bca", IntelCometLake, true),
        ("8086", "8a52", IntelIceLake, true),
        ("8086", "8a56", IntelIceLake, true),
        ("8086", "5a84", IntelLowPower, true),
        ("8086", "3184", IntelLowPower, true),
        ("8086", "3185", IntelLowPower, true),
        ("8086", "4e55", IntelLowPower, true),
        ("8086", "46d1", IntelLowPower, true),
        ("8086", "9a49", IntelXe, true),
        ("8086", "4c8a", IntelXe, true),
        ("8086", "4680", IntelXe, true),
        ("8086", "46a6", IntelXe, true),
        ("8086", "a780", IntelXe, true),
        ("8086", "a7a0", IntelXe, true),
        ("8086", "7d55", IntelXe, true),
        ("8086", "64a0", IntelXe, true),
        ("8086", "56a0", IntelArc, false),
        ("8086", "e20b", IntelArc, false),
        ("8086", "2e12", IntelGma, true),
        // AMD dGPUs
        ("1002", "6798", AmdGcn1, false),
        ("1002", "6810", AmdGcn1, false),
        ("1002", "683f", AmdGcn1, false),
        ("1002", "6611", AmdGcn1, false),
        ("1002", "665c", AmdGcn2, false),
        ("1002", "67b0", AmdGcn2, false),
        ("1002", "67b1", AmdGcn2, false),
        ("1002", "6939", AmdGcn3, false),
        ("1002", "7300", AmdGcn3, false),
        ("1002", "6738", AmdTeraScale, false),
        ("1002", "68b8", AmdTeraScale, false),
        ("1002", "9440", AmdTeraScale, false),
        ("1002", "67df", AmdPolaris, false),
        ("1002", "67ef", AmdPolaris, false),
        ("1002", "67ff", AmdPolaris, false),
        ("1002", "6fdf", AmdPolaris, false),
        ("1002", "694c", AmdPolaris, false),
        ("1002", "699f", AmdLexa, false),
        ("1002", "6987", AmdLexa, false),
        ("1002", "6995", AmdLexa, false),
        ("1002", "687f", AmdVega10, false),
        ("1002", "6863", AmdVega10, false),
        ("1002", "69af", AmdVega10, false),
        ("1002", "66af", AmdVega20, false),
        ("1002", "731f", AmdNavi10, false),
        ("1002", "7360", AmdNavi12, false),
        ("1002", "7340", AmdNavi14, false),
        ("1002", "73a2", AmdNavi21, false),
        ("1002", "73a3", AmdNavi21, false),
        ("1002", "73a5", AmdNavi21, false),
        ("1002", "73ab", AmdNavi21, false),
        ("1002", "73af", AmdNavi21, false),
        ("1002", "73bf", AmdNavi21, false),
        ("1002", "73df", AmdNavi22, false),
        ("1002", "73e3", AmdNavi23, false),
        ("1002", "73ef", AmdNavi23, false),
        ("1002", "73ff", AmdNavi23, false),
        ("1002", "743f", AmdNavi24, false),
        ("1002", "73f0", AmdRdna3Plus, false),
        ("1002", "744c", AmdRdna3Plus, false),
        ("1002", "7480", AmdRdna3Plus, false),
        ("1002", "7550", AmdRdna3Plus, false),
        ("1002", "7590", AmdRdna3Plus, false),
        // AMD APUs
        ("1002", "15dd", AmdApuVega, true),
        ("1002", "15d8", AmdApuVega, true),
        ("1002", "1636", AmdApuVega, true),
        ("1002", "1638", AmdApuVega, true),
        ("1002", "164c", AmdApuVega, true),
        ("1002", "15e7", AmdApuVega, true),
        ("1022", "1638", AmdApuVega, true),
        ("1002", "1681", AmdApuRdna, true),
        ("1002", "15bf", AmdApuRdna, true),
        ("1002", "15c8", AmdApuRdna, true),
        ("1002", "164e", AmdApuRdna, true),
        ("1002", "150e", AmdApuRdna, true),
        ("1002", "13c0", AmdApuRdna, true),
        ("1002", "1506", AmdApuRdna, true),
        ("1002", "130f", AmdApuLegacy, true),
        ("1002", "9874", AmdApuLegacy, true),
        ("1002", "990c", AmdApuLegacy, true),
        // NVIDIA
        ("10de", "0622", NvidiaTesla, false),
        ("10de", "0a65", NvidiaTesla, false),
        ("10de", "0866", NvidiaTesla, true),
        ("10de", "06c0", NvidiaFermi, false),
        ("10de", "0f02", NvidiaFermi, false),
        ("10de", "104a", NvidiaFermi, false),
        ("10de", "1080", NvidiaFermi, false),
        ("10de", "1244", NvidiaFermi, false),
        ("10de", "0fc9", NvidiaKepler, false),
        ("10de", "1004", NvidiaKepler, false),
        ("10de", "100c", NvidiaKepler, false),
        ("10de", "1180", NvidiaKepler, false),
        ("10de", "11c6", NvidiaKepler, false),
        ("10de", "1281", NvidiaKepler, false),
        ("10de", "1287", NvidiaKepler, false),
        ("10de", "128b", NvidiaKepler, false),
        ("10de", "1380", NvidiaMaxwell, false),
        ("10de", "13c2", NvidiaMaxwell, false),
        ("10de", "1401", NvidiaMaxwell, false),
        ("10de", "1617", NvidiaMaxwell, false),
        ("10de", "17c8", NvidiaMaxwell, false),
        ("10de", "1b06", NvidiaPascal, false),
        ("10de", "1b80", NvidiaPascal, false),
        ("10de", "1c03", NvidiaPascal, false),
        ("10de", "1d01", NvidiaPascal, false),
        ("10de", "1db4", NvidiaModern, false),
        ("10de", "1e87", NvidiaModern, false),
        ("10de", "1f82", NvidiaModern, false),
        ("10de", "2484", NvidiaModern, false),
        ("10de", "2684", NvidiaModern, false),
        ("10de", "2b85", NvidiaModern, false),
        ("10de", "0391", GpuFamily::Unknown, false),
    ];
    for (vendor, device, family, igpu) in cases {
        let id = identify(Some(vendor), Some(device), "");
        assert_eq!(id.family, *family, "{vendor}:{device}");
        assert_eq!(id.is_igpu, *igpu, "{vendor}:{device} igpu");
        assert!(id.model_name.is_some(), "{vendor}:{device} model name");
        let expected_vendor = match *vendor {
            "8086" => GpuVendor::Intel,
            "10de" => GpuVendor::Nvidia,
            _ => GpuVendor::Amd,
        };
        assert_eq!(id.vendor, expected_vendor, "{vendor}:{device} vendor");
    }
}

#[test]
fn identifies_virtual_adapters() {
    let cases = [
        ("15ad", "0405"),
        ("1b36", "0100"),
        ("1af4", "1050"),
        ("1234", "1111"),
        ("1414", "5353"),
        ("80ee", "beef"),
        ("1ab8", "4005"),
        ("1013", "00b8"),
    ];
    for (vendor, device) in cases {
        let id = identify(Some(vendor), Some(device), "");
        assert_eq!(id.vendor, GpuVendor::Virtual, "{vendor}:{device}");
        assert_eq!(id.family, VirtualDisplay, "{vendor}:{device}");
        assert!(!id.is_igpu);
    }
    let names = [
        "VMware SVGA 3D",
        "Microsoft Basic Display Adapter",
        "Microsoft Hyper-V Video",
        "VirtualBox Graphics Adapter (WDDM)",
        "Red Hat, Inc. QXL paravirtual graphic card",
        "Virtio 1.0 GPU",
        "QEMU Standard VGA",
        "Parallels Display Adapter (WDDM)",
        "Microsoft Remote Display Adapter",
        "llvmpipe (LLVM 15.0.7, 256 bits)",
    ];
    for name in names {
        let id = identify(None, None, name);
        assert_eq!(
            (id.vendor, id.family),
            (GpuVendor::Virtual, VirtualDisplay),
            "{name}"
        );
    }
}

#[test]
fn pci_id_beats_a_generic_driver_name() {
    // Windows reports the real PCI id even while the GPU runs on the basic driver.
    let id = identify(
        Some("10de"),
        Some("1c03"),
        "Microsoft Basic Display Adapter",
    );
    assert_eq!((id.vendor, id.family), (GpuVendor::Nvidia, NvidiaPascal));
}

#[test]
fn accepts_id_spellings() {
    for (vendor, device) in [
        ("0x8086", "0x3E9B"),
        ("8086", "3E9B"),
        (" 8086 ", " 3e9b"),
        ("0X8086", "3e9b"),
    ] {
        assert_eq!(
            identify(Some(vendor), Some(device), "").family,
            IntelCoffeeLake,
            "{vendor}:{device}"
        );
    }
    // Garbage ids fall back to the name.
    let id = identify(Some("zzzz"), Some("123456"), "Intel(R) UHD Graphics 630");
    assert_eq!((id.vendor, id.family), (GpuVendor::Intel, IntelCoffeeLake));
    let id = identify(Some(""), None, "");
    assert_eq!(
        (id.vendor, id.family),
        (GpuVendor::Unknown, GpuFamily::Unknown)
    );
}

#[test]
fn unknown_device_of_known_vendor_uses_the_name() {
    let id = identify(Some("8086"), Some("ffff"), "Intel(R) Iris(R) Xe Graphics");
    assert_eq!(
        (id.vendor, id.family, id.is_igpu),
        (GpuVendor::Intel, IntelXe, true)
    );
    assert_eq!(id.model_name, None);
    let id = identify(Some("1002"), Some("0001"), "Some card");
    assert_eq!((id.vendor, id.family), (GpuVendor::Amd, GpuFamily::Unknown));
    let id = identify(Some("1a03"), Some("2000"), "ASPEED Graphics Family");
    assert_eq!(
        (id.vendor, id.family),
        (GpuVendor::Unknown, GpuFamily::Unknown)
    );
    assert_eq!(id.model_name.as_deref(), Some("ASPEED BMC graphics"));
}

#[test]
fn navi_revision_and_6750_gre() {
    let id = identify_with_revision(Some("1002"), Some("73ff"), Some("df"), "");
    assert_eq!(id.family, AmdNavi22);
    assert_eq!(id.model_name.as_deref(), Some("Radeon RX 6750 GRE 10GB"));
    let id = identify_with_revision(Some("1002"), Some("73ff"), Some("0xC1"), "");
    assert_eq!(id.family, AmdNavi23);
    assert_eq!(id.model_name.as_deref(), Some("Radeon RX 6600 XT"));
    let id = identify(Some("1002"), Some("73ff"), "AMD Radeon RX 6750 GRE 10GB");
    assert_eq!(id.family, AmdNavi22);
    let id = identify_with_revision(Some("1002"), Some("73bf"), Some("c3"), "");
    assert_eq!(id.model_name.as_deref(), Some("Radeon RX 6800"));
    let id = identify_with_revision(Some("1002"), Some("73df"), Some("c3"), "");
    assert_eq!(
        (id.family, id.model_name.as_deref()),
        (AmdNavi22, Some("Radeon RX 6800M"))
    );
}

// ── Identification by name ──────────────────────────────────────────────────

#[test]
fn identifies_by_name() {
    use GpuVendor::{Amd, Intel, Nvidia};
    let cases: &[(&str, GpuVendor, GpuFamily)] = &[
        // AMD
        ("RX 580", Amd, AmdPolaris),
        ("AMD Radeon RX 580 2048SP", Amd, AmdPolaris),
        ("Radeon RX 6600 XT", Amd, AmdNavi23),
        ("AMD Radeon RX 6700 XT", Amd, AmdNavi22),
        ("AMD Radeon RX 6800M", Amd, AmdNavi22),
        ("AMD Radeon RX 6850M XT", Amd, AmdNavi22),
        ("AMD Radeon RX 6800S", Amd, AmdNavi23),
        ("AMD Radeon RX 6650M XT", Amd, AmdNavi23),
        ("AMD Radeon RX 6800 XT", Amd, AmdNavi21),
        ("AMD Radeon RX 6950 XT", Amd, AmdNavi21),
        ("AMD Radeon RX 6750 GRE 10GB", Amd, AmdNavi22),
        ("AMD Radeon RX 6500 XT", Amd, AmdNavi24),
        ("AMD Radeon RX 7900 XTX", Amd, AmdRdna3Plus),
        ("AMD Radeon RX 9070 XT", Amd, AmdRdna3Plus),
        ("AMD Radeon RX 5700 XT", Amd, AmdNavi10),
        ("AMD Radeon RX 5500 XT", Amd, AmdNavi14),
        ("AMD Radeon Pro 5600M", Amd, AmdNavi12),
        ("AMD Radeon Pro 5500M", Amd, AmdNavi14),
        ("Radeon RX Vega 64", Amd, AmdVega10),
        ("AMD Radeon RX Vega", Amd, AmdVega10),
        ("AMD Radeon VII", Amd, AmdVega20),
        ("Radeon Vega 8 Graphics", Amd, AmdApuVega),
        ("AMD Radeon(TM) Vega 8 Graphics", Amd, AmdApuVega),
        ("AMD Radeon RX Vega 11 Graphics", Amd, AmdApuVega),
        ("Advanced Micro Devices, Inc. [AMD/ATI] Vega 10 XL/XT [Radeon RX Vega 56/64]", Amd, AmdVega10),
        (
            "Advanced Micro Devices, Inc. [AMD/ATI] Renoir [Radeon Vega Series / Radeon Vega Mobile Series]",
            Amd,
            AmdApuVega,
        ),
        ("AMD Radeon 780M Graphics", Amd, AmdApuRdna),
        ("AMD Radeon RX Vega M GH Graphics", Amd, AmdPolaris),
        ("AMD Radeon RX 550", Amd, AmdLexa),
        ("AMD Radeon Pro WX 3200", Amd, AmdLexa),
        ("Radeon Pro WX 7100", Amd, AmdPolaris),
        ("AMD Radeon Pro W6800", Amd, AmdNavi21),
        ("AMD Radeon Pro W6600", Amd, AmdNavi23),
        ("AMD Radeon Pro W5700", Amd, AmdNavi10),
        ("AMD Radeon Pro W7800", Amd, AmdRdna3Plus),
        ("AMD Radeon Pro Vega II", Amd, AmdVega20),
        ("Radeon Pro Vega 56", Amd, AmdVega10),
        ("AMD Radeon R9 290X", Amd, AmdGcn2),
        ("AMD Radeon R9 390", Amd, AmdGcn2),
        ("AMD Radeon R9 380", Amd, AmdGcn3),
        ("AMD Radeon R9 280X", Amd, AmdGcn1),
        ("AMD Radeon R7 370", Amd, AmdGcn1),
        ("AMD Radeon R9 Fury X", Amd, AmdGcn3),
        ("AMD Radeon HD 7970", Amd, AmdGcn1),
        ("AMD Radeon HD 7790", Amd, AmdGcn2),
        ("ATI Radeon HD 6870", Amd, AmdTeraScale),
        ("ATI Radeon HD 5770", Amd, AmdTeraScale),
        ("AMD Radeon HD 7470", Amd, AmdTeraScale),
        ("AMD Radeon HD 8570D", Amd, AmdApuLegacy),
        ("AMD Radeon R7 Graphics", Amd, AmdApuLegacy),
        ("AMD FirePro W9100", Amd, AmdGcn2),
        ("AMD FirePro D700", Amd, AmdGcn1),
        ("Advanced Micro Devices, Inc. [AMD/ATI] Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]", Amd, AmdNavi21),
        ("Advanced Micro Devices, Inc. [AMD/ATI] Ellesmere [Radeon RX 470/480/570/570X/580/580X/590]", Amd, AmdPolaris),
        // NVIDIA
        ("NVIDIA GeForce GTX 1060 6GB", Nvidia, NvidiaPascal),
        ("GTX 1060", Nvidia, NvidiaPascal),
        ("NVIDIA GeForce GT 710", Nvidia, NvidiaKepler),
        ("NVIDIA GeForce GT 730", Nvidia, NvidiaKepler),
        ("NVIDIA Corporation GF108 [GeForce GT 730]", Nvidia, NvidiaFermi),
        ("NVIDIA Corporation GK208B [GeForce GT 730]", Nvidia, NvidiaKepler),
        ("NVIDIA GeForce GTX 680", Nvidia, NvidiaKepler),
        ("NVIDIA GeForce GTX 770", Nvidia, NvidiaKepler),
        ("NVIDIA GeForce GTX 750 Ti", Nvidia, NvidiaMaxwell),
        ("NVIDIA GeForce GTX 960", Nvidia, NvidiaMaxwell),
        ("NVIDIA GeForce GTX 1650", Nvidia, NvidiaModern),
        ("NVIDIA GeForce RTX 3070", Nvidia, NvidiaModern),
        ("GeForce RTX 4090", Nvidia, NvidiaModern),
        ("NVIDIA GeForce GTX TITAN Black", Nvidia, NvidiaKepler),
        ("NVIDIA GeForce GTX TITAN X", Nvidia, NvidiaMaxwell),
        ("NVIDIA TITAN Xp", Nvidia, NvidiaPascal),
        ("NVIDIA TITAN V", Nvidia, NvidiaModern),
        ("NVIDIA GeForce GT 1030", Nvidia, NvidiaPascal),
        ("NVIDIA GeForce GTX 580", Nvidia, NvidiaFermi),
        ("NVIDIA GeForce GTS 450", Nvidia, NvidiaFermi),
        ("NVIDIA GeForce 210", Nvidia, NvidiaTesla),
        ("NVIDIA GeForce 9800 GT", Nvidia, NvidiaTesla),
        ("NVIDIA GeForce GT 640", Nvidia, NvidiaKepler),
        ("NVIDIA GeForce GT 630", Nvidia, NvidiaFermi),
        ("NVIDIA Quadro K4000", Nvidia, NvidiaKepler),
        ("NVIDIA Quadro K2200", Nvidia, NvidiaMaxwell),
        ("NVIDIA Quadro P2000", Nvidia, NvidiaPascal),
        ("NVIDIA Quadro RTX 4000", Nvidia, NvidiaModern),
        ("NVIDIA Quadro 4000", Nvidia, NvidiaFermi),
        ("NVIDIA Quadro FX 3800", Nvidia, NvidiaTesla),
        ("NVIDIA Tesla K80", Nvidia, NvidiaKepler),
        ("NVIDIA GeForce MX150", Nvidia, NvidiaPascal),
        ("NVIDIA GeForce MX130", Nvidia, NvidiaMaxwell),
        ("NVIDIA GeForce GTX 1050 Ti with Max-Q Design", Nvidia, NvidiaPascal),
        ("NVIDIA GeForce GTX 670M", Nvidia, NvidiaFermi),
        ("NVIDIA GeForce GTX 675MX", Nvidia, NvidiaKepler),
        ("NVIDIA GeForce 940MX", Nvidia, NvidiaMaxwell),
        // Intel
        ("Intel(R) UHD Graphics 630", Intel, IntelCoffeeLake),
        ("UHD 630", Intel, IntelCoffeeLake),
        ("Intel(R) HD Graphics 630", Intel, IntelKabyLake),
        ("Intel(R) HD Graphics 530", Intel, IntelSkylake),
        ("Intel(R) HD Graphics 520", Intel, IntelSkylake),
        ("Intel(R) HD Graphics 515", Intel, IntelSkylake),
        ("Intel(R) HD Graphics P530", Intel, IntelSkylake),
        ("Intel(R) HD Graphics 4600", Intel, IntelHaswell),
        ("Intel(R) HD Graphics 5000", Intel, IntelHaswell),
        ("Intel(R) HD Graphics 5500", Intel, IntelBroadwell),
        ("Intel(R) HD Graphics 6000", Intel, IntelBroadwell),
        ("Intel(R) HD Graphics 4000", Intel, IntelIvyBridge),
        ("Intel(R) HD Graphics 3000", Intel, IntelSandyBridge),
        ("Intel(R) HD Graphics 2000", Intel, IntelSandyBridge),
        ("Intel(R) HD Graphics 610", Intel, IntelKabyLake),
        ("Intel(R) UHD Graphics 610", Intel, IntelCoffeeLake),
        ("Intel(R) UHD Graphics 620", Intel, IntelKabyLake),
        ("Intel(R) UHD Graphics 617", Intel, IntelKabyLake),
        ("Intel(R) HD Graphics 505", Intel, IntelLowPower),
        ("Intel(R) Iris(R) Plus Graphics", Intel, IntelIceLake),
        ("Intel(R) Iris(R) Plus Graphics 655", Intel, IntelCoffeeLake),
        ("Intel(R) Iris(R) Plus Graphics 640", Intel, IntelKabyLake),
        ("Intel(R) Iris(TM) Pro Graphics 5200", Intel, IntelHaswell),
        ("Intel(R) Iris(R) Pro Graphics 6200", Intel, IntelBroadwell),
        ("Intel(R) Iris(R) Graphics 540", Intel, IntelSkylake),
        ("Intel(R) Iris(R) Xe Graphics", Intel, IntelXe),
        ("Intel(R) UHD Graphics 770", Intel, IntelXe),
        ("Intel(R) UHD Graphics 750", Intel, IntelXe),
        ("Intel(R) UHD Graphics 600", Intel, IntelLowPower),
        ("Intel(R) UHD Graphics", Intel, GpuFamily::Unknown),
        ("Intel(R) Arc(TM) A770 Graphics", Intel, IntelArc),
        ("Intel(R) Arc(TM) Graphics", Intel, IntelXe),
        ("Intel(R) Graphics", Intel, IntelXe),
        ("Intel Corporation CometLake-S GT2 [UHD Graphics 630]", Intel, IntelCometLake),
        (
            "Intel Corporation Xeon E3-1200 v3/4th Gen Core Processor Integrated Graphics Controller",
            Intel,
            IntelHaswell,
        ),
        ("Intel Corporation 2nd Generation Core Processor Family Integrated Graphics Controller", Intel, IntelSandyBridge),
        ("Intel Corporation 3rd Gen Core processor Graphics Controller", Intel, IntelIvyBridge),
        ("Intel Corporation Alder Lake-N [UHD Graphics]", Intel, IntelLowPower),
        ("Intel Corporation Raptor Lake-S GT1 [UHD Graphics 770]", Intel, IntelXe),
        ("Intel Corporation Kaby Lake-R GT2 [UHD Graphics 620]", Intel, IntelKabyLake),
        ("Intel Corporation WhiskeyLake-U GT2 [UHD Graphics 620]", Intel, IntelCoffeeLake),
        ("Intel Corporation Skylake GT2 [HD Graphics 520]", Intel, IntelSkylake),
        ("Intel(R) Graphics Media Accelerator HD", Intel, IntelIronLake),
        ("Mobile Intel(R) 4 Series Express Chipset Family", Intel, IntelGma),
    ];
    for (name, vendor, family) in cases {
        let id = identify(None, None, name);
        assert_eq!((id.vendor, id.family), (*vendor, *family), "{name}");
        assert_eq!(id.model_name, None, "{name}");
    }
}

#[test]
fn name_igpu_flags() {
    assert!(identify(None, None, "Intel(R) UHD Graphics 630").is_igpu);
    assert!(identify(None, None, "Intel(R) UHD Graphics").is_igpu);
    assert!(!identify(None, None, "Intel(R) Arc(TM) A770 Graphics").is_igpu);
    assert!(identify(None, None, "AMD Radeon(TM) Vega 8 Graphics").is_igpu);
    let bare = identify(None, None, "AMD Radeon(TM) Graphics");
    assert_eq!(
        (bare.vendor, bare.family, bare.is_igpu),
        (GpuVendor::Amd, GpuFamily::Unknown, true)
    );
    assert!(!identify(None, None, "AMD Radeon RX 6600 XT").is_igpu);
    assert!(!identify(None, None, "NVIDIA GeForce GTX 1060").is_igpu);
}

#[test]
fn unknown_names_stay_unknown() {
    for name in ["", "   ", "Foo Bar 9000", "Generic PnP Monitor"] {
        let id = identify(None, None, name);
        assert_eq!(
            (id.vendor, id.family),
            (GpuVendor::Unknown, GpuFamily::Unknown),
            "{name:?}"
        );
    }
}

// ── Support matrix ──────────────────────────────────────────────────────────

#[test]
fn nvidia_support_matrix() {
    let kepler = dev("10de", "1287");
    assert_eq!(
        native_range(&kepler),
        vec![HighSierra, Mojave, Catalina, BigSur]
    );
    let s = support(&kepler);
    assert_eq!(s.max_with_root_patch, Some(Tahoe));
    assert_eq!(s.requirement, GpuRequirement::Standard);

    let fermi = dev("10de", "0f02");
    assert_eq!(native_range(&fermi), vec![HighSierra]);

    for maxwell_or_pascal in [dev("10de", "1380"), dev("10de", "1c03")] {
        assert_eq!(native_range(&maxwell_or_pascal), vec![HighSierra]);
        assert!(support(&maxwell_or_pascal)
            .boot_args
            .contains(&"nvda_drv_vrl=1"));
    }

    for modern in [
        dev("10de", "1f82"),
        dev("10de", "2484"),
        dev("10de", "2684"),
    ] {
        let s = support(&modern);
        assert!(!s.display_capable);
        assert!(native_range(&modern).is_empty());
        assert_eq!(s.max_with_root_patch, None);
    }
    assert!(!support(&dev("10de", "0391")).display_capable);
}

#[test]
fn amd_legacy_support_matrix() {
    let hawaii = dev("1002", "67b0");
    assert_eq!(
        native_range(&hawaii),
        vec![HighSierra, Mojave, Catalina, BigSur, Monterey]
    );
    assert_eq!(support(&hawaii).max_with_root_patch, Some(Tahoe));
    let r9_290 = support(&dev("1002", "67b1"));
    assert_eq!(r9_290.requirement, spoof(0x67B0));
    assert!(r9_290.display_capable);
    let cape_verde = support(&dev("1002", "683f"));
    assert!(cape_verde.boot_args.contains(&"radpg=15"));
    let oland = support(&dev("1002", "6611"));
    assert!(!oland.display_capable);
    let terascale = dev("1002", "6738");
    assert_eq!(native_range(&terascale), vec![HighSierra]);
    assert_eq!(support(&terascale).max_with_root_patch, None);
}

#[test]
fn amd_polaris_vega_support_matrix() {
    let rx580 = dev("1002", "67df");
    assert_eq!(native_range(&rx580), MacOsVersion::ALL.to_vec());
    let s = support(&rx580);
    assert_eq!(s.requirement, GpuRequirement::Standard);
    assert!(!s.boot_args.contains(&"agdpmod=pikera"));

    let rx580_2048 = support(&dev("1002", "6fdf"));
    assert!(!rx580_2048.display_capable);
    let vega_m = support(&dev("1002", "694c"));
    assert!(!vega_m.display_capable);

    let lexa = dev("1002", "699f");
    let s = support(&lexa);
    assert_eq!(s.requirement, spoof(0x67FF));
    assert_eq!(
        s.requirement,
        GpuRequirement::DeviceIdSpoof([0xFF, 0x67, 0x00, 0x00])
    );
    assert!(s.boot_args.contains(&"-radcodec"));
    assert!(natively_supported_on(&lexa, Tahoe));
    assert_eq!(support(&dev("1002", "6995")).requirement, spoof(0x67E3));

    let vega64 = dev("1002", "687f");
    assert_eq!(native_range(&vega64), MacOsVersion::ALL.to_vec());
    let radeon_vii = dev("1002", "66af");
    assert!(!natively_supported_on(&radeon_vii, HighSierra));
    assert!(natively_supported_on(&radeon_vii, Mojave));
    assert!(natively_supported_on(&radeon_vii, Tahoe));
    let pro_vega_20 = dev("1002", "69af");
    assert!(!natively_supported_on(&pro_vega_20, HighSierra));
    assert!(natively_supported_on(&pro_vega_20, Mojave));
}

#[test]
fn amd_navi_support_matrix() {
    for id in ["731f", "7340"] {
        let gpu = dev("1002", id);
        assert!(!natively_supported_on(&gpu, Mojave), "{id}");
        assert!(natively_supported_on(&gpu, Catalina), "{id}");
        assert!(natively_supported_on(&gpu, Tahoe), "{id}");
        assert_eq!(support(&gpu).boot_args, vec!["agdpmod=pikera"], "{id}");
    }

    let rx6800 = dev("1002", "73bf");
    assert!(!natively_supported_on(&rx6800, Catalina));
    assert!(natively_supported_on(&rx6800, BigSur));
    assert!(natively_supported_on(&rx6800, Tahoe));
    let s = support(&rx6800);
    assert_eq!(s.requirement, GpuRequirement::Standard);
    assert!(s.boot_args.contains(&"agdpmod=pikera"));
    assert!(s.notes.iter().any(|n| n.contains("agdpmod=ignore")));
    assert_eq!(weg_spoof_alternative(&rx6800), None);

    for id in ["73af", "73a5"] {
        let gpu = dev("1002", id);
        let s = support(&gpu);
        assert_eq!(s.requirement, GpuRequirement::NootRx, "{id}");
        assert!(s.boot_args.is_empty(), "{id}");
        assert!(natively_supported_on(&gpu, BigSur), "{id}");
        assert!(natively_supported_on(&gpu, Tahoe), "{id}");
        assert_eq!(
            weg_spoof_alternative(&gpu),
            Some([0xBF, 0x73, 0, 0]),
            "{id}"
        );
    }

    let rx6700xt = dev("1002", "73df");
    let s = support(&rx6700xt);
    assert_eq!(s.requirement, GpuRequirement::NootRx);
    assert_eq!(
        native_range(&rx6700xt),
        vec![Monterey, Ventura, Sonoma, Sequoia, Tahoe]
    );
    assert_eq!(weg_spoof_alternative(&rx6700xt), None);

    let rx6600 = dev("1002", "73ff");
    assert!(!natively_supported_on(&rx6600, BigSur));
    assert!(natively_supported_on(&rx6600, Monterey));
    assert!(natively_supported_on(&rx6600, Tahoe));
    assert_eq!(support(&rx6600).requirement, GpuRequirement::Standard);

    let rx6650xt = dev("1002", "73ef");
    assert_eq!(support(&rx6650xt).requirement, GpuRequirement::NootRx);
    assert_eq!(weg_spoof_alternative(&rx6650xt), Some([0xFF, 0x73, 0, 0]));
    assert!(!natively_supported_on(&rx6650xt, BigSur));

    // RX 6750 GRE 10GB sits on the Navi 23 id with revision 0xDF.
    let id = identify_with_revision(Some("1002"), Some("73ff"), Some("df"), "");
    let gre = ProfileGpu {
        vendor: id.vendor,
        family: id.family,
        vendor_id: Some("1002".into()),
        device_id: Some("73ff".into()),
        ..Default::default()
    };
    assert_eq!(support(&gre).requirement, GpuRequirement::NootRx);

    for id in ["743f", "744c", "7480", "7550", "73f0"] {
        let gpu = dev("1002", id);
        assert!(!support(&gpu).display_capable, "{id}");
        assert!(native_range(&gpu).is_empty(), "{id}");
    }
}

#[test]
fn amd_apu_support_matrix() {
    let raven = dev("1002", "15dd");
    let s = support(&raven);
    assert_eq!(s.requirement, GpuRequirement::NootedRed);
    assert_eq!(
        native_range(&raven),
        vec![Catalina, BigSur, Monterey, Ventura, Sonoma, Sequoia, Tahoe]
    );
    for id in ["1681", "15bf", "164e", "150e"] {
        assert!(!support(&dev("1002", id)).display_capable, "{id}");
    }
    assert!(!support(&dev("1002", "130f")).display_capable);
}

#[test]
fn intel_old_generations_support_matrix() {
    let hd3000 = dev("8086", "0126");
    assert_eq!(native_range(&hd3000), vec![HighSierra]);
    // Non-Metal: OCLP is only mentioned, never offered as a path.
    assert_eq!(support(&hd3000).max_with_root_patch, None);
    assert!(support(&hd3000)
        .notes
        .iter()
        .any(|n| n.contains("Non-Metal")));
    assert_eq!(support(&dev("8086", "0112")).requirement, spoof(0x0126));
    assert_eq!(support(&dev("8086", "0122")).requirement, spoof(0x0126));
    assert!(!support(&dev("8086", "0102")).display_capable);
    assert!(!support(&dev("8086", "010a")).display_capable);
    // Mobile GT1 is in Apple's match list, but only with a GT1 warning.
    let snb_gt1 = support(&dev("8086", "0106"));
    assert!(snb_gt1.display_capable);
    assert!(snb_gt1.notes.iter().any(|n| n.contains("GT1")));

    let hd4000 = dev("8086", "0162");
    assert_eq!(
        native_range(&hd4000),
        vec![HighSierra, Mojave, Catalina, BigSur]
    );
    assert_eq!(support(&hd4000).max_with_root_patch, Some(Tahoe));
    let hd2500 = support(&dev("8086", "0152"));
    assert!(!hd2500.display_capable);
    assert_eq!(hd2500.max_native, Some(BigSur));

    let hd4600 = dev("8086", "0412");
    assert_eq!(
        native_range(&hd4600),
        vec![HighSierra, Mojave, Catalina, BigSur, Monterey]
    );
    assert_eq!(support(&hd4600).requirement, GpuRequirement::Standard);
    assert_eq!(support(&hd4600).max_with_root_patch, Some(Tahoe));
    assert_eq!(support(&dev("8086", "0416")).requirement, spoof(0x0412));
    assert_eq!(support(&dev("8086", "0a16")).requirement, spoof(0x0412));
    assert_eq!(
        support(&dev("8086", "0a26")).requirement,
        GpuRequirement::Standard
    );
    assert_eq!(
        support(&dev("8086", "0d26")).requirement,
        GpuRequirement::Standard
    );
    assert!(!support(&dev("8086", "0402")).display_capable);

    let hd5500 = dev("8086", "1616");
    assert!(natively_supported_on(&hd5500, Monterey));
    assert!(!natively_supported_on(&hd5500, Ventura));
    assert_eq!(support(&dev("8086", "1612")).requirement, spoof(0x1626));
    assert!(!support(&dev("8086", "0042")).display_capable);
    assert!(natively_supported_on(&dev("8086", "0046"), HighSierra));
}

#[test]
fn intel_skylake_support_matrix() {
    let hd530 = dev("8086", "1912");
    let s = support(&hd530);
    assert_eq!(s.requirement, spoof(0x5912));
    assert_eq!(s.max_native, None);
    assert_eq!(native_range(&hd530), MacOsVersion::ALL.to_vec());
    assert_eq!(device_id_for(&hd530, Monterey), None);
    assert_eq!(device_id_for(&hd530, Ventura), Some([0x12, 0x59, 0, 0]));
    assert_eq!(device_id_for(&hd530, Tahoe), Some([0x12, 0x59, 0, 0]));

    let p530 = dev("8086", "191d");
    assert_eq!(device_id_for(&p530, Catalina), Some([0x1B, 0x19, 0, 0]));
    assert_eq!(device_id_for(&p530, Sonoma), Some([0x1B, 0x59, 0, 0]));

    let hd520 = dev("8086", "1916");
    assert_eq!(support(&hd520).requirement, spoof(0x5916));

    let iris_pro_580 = dev("8086", "193b");
    let s = support(&iris_pro_580);
    assert_eq!(s.max_native, Some(Monterey));
    assert_eq!(s.max_with_root_patch, Some(Tahoe));
    assert!(!natively_supported_on(&iris_pro_580, Ventura));

    let hd510_laptop = dev("8086", "1906");
    let s = support(&hd510_laptop);
    assert!(s.display_capable);
    assert_eq!(s.max_native, Some(Monterey));
    assert_eq!(
        device_id_for(&hd510_laptop, Monterey),
        Some([0x02, 0x19, 0, 0])
    );
    assert!(s.notes.iter().any(|n| n.contains("GT1")));
}

#[test]
fn intel_kaby_coffee_comet_ice_support_matrix() {
    let hd630 = dev("8086", "5912");
    assert_eq!(native_range(&hd630), MacOsVersion::ALL.to_vec());
    assert_eq!(support(&dev("8086", "5917")).requirement, spoof(0x5916));
    let uhd617 = dev("8086", "87c0");
    assert!(!natively_supported_on(&uhd617, HighSierra));
    assert!(natively_supported_on(&uhd617, Mojave));
    let hd610 = support(&dev("8086", "5902"));
    assert!(hd610.display_capable);
    assert_eq!(hd610.requirement, spoof(0x5912));

    let uhd630_desktop = dev("8086", "3e92");
    assert!(!natively_supported_on(&uhd630_desktop, HighSierra));
    assert!(natively_supported_on(&uhd630_desktop, Mojave));
    assert!(natively_supported_on(&uhd630_desktop, Tahoe));
    let uhd630_laptop = dev("8086", "3e9b");
    assert!(natively_supported_on(&uhd630_laptop, HighSierra));
    assert_eq!(support(&dev("8086", "3ea0")).requirement, spoof(0x3E9B));
    assert_eq!(support(&dev("8086", "3ea9")).requirement, spoof(0x3E9B));
    assert_eq!(
        support(&dev("8086", "3ea5")).requirement,
        GpuRequirement::Standard
    );
    assert_eq!(support(&dev("8086", "3e90")).requirement, spoof(0x3E92));
    assert_eq!(support(&dev("8086", "3e96")).requirement, spoof(0x3E92));

    let cml = dev("8086", "9bc8");
    assert_eq!(
        native_range(&cml),
        vec![Catalina, BigSur, Monterey, Ventura, Sonoma, Sequoia, Tahoe]
    );
    assert_eq!(support(&cml).requirement, GpuRequirement::Standard);
    assert_eq!(
        support(&dev("8086", "9bc5")).requirement,
        GpuRequirement::Standard
    );
    assert_eq!(support(&dev("8086", "9b41")).requirement, spoof(0x3E9B));
    assert_eq!(support(&dev("8086", "9bca")).requirement, spoof(0x3E9B));
    assert_eq!(support(&dev("8086", "9be6")).requirement, spoof(0x9BC5));
    // 0x9B21/0x9BAA/0x9BAC ship on UHD 620 parts (i5-10210U) too.
    assert_eq!(support(&dev("8086", "9b21")).requirement, spoof(0x3E9B));
    assert_eq!(support(&dev("8086", "9bac")).requirement, spoof(0x3E9B));
    assert!(support(&dev("8086", "9b21"))
        .notes
        .iter()
        .any(|n| n.contains("GT1")));
    assert_eq!(support(&dev("8086", "9bc2")).requirement, spoof(0x9BC4));
    assert!(!support(&dev("8086", "9ba4")).display_capable);
    // Xeon E-2100M P630 is faked to the desktop id, which needs 10.14.
    let p630_mobile = dev("8086", "3e94");
    assert_eq!(support(&p630_mobile).requirement, spoof(0x3E92));
    assert!(!natively_supported_on(&p630_mobile, HighSierra));
    assert!(natively_supported_on(&p630_mobile, Mojave));

    let icl = dev("8086", "8a52");
    assert!(!natively_supported_on(&icl, Mojave));
    assert!(natively_supported_on(&icl, Catalina));
    assert!(natively_supported_on(&icl, Tahoe));
    assert_eq!(support(&icl).boot_args, vec!["-igfxcdc", "-igfxdvmt"]);
    assert!(!support(&dev("8086", "8a56")).display_capable);
}

#[test]
fn modern_intel_is_unsupported() {
    for id in [
        "9a49", "4c8a", "4680", "a780", "7d55", "64a0", "3185", "5a85", "46d1", "56a0", "e20b",
    ] {
        let gpu = dev("8086", id);
        assert!(!support(&gpu).display_capable, "{id}");
        assert!(native_range(&gpu).is_empty(), "{id}");
    }
}

#[test]
fn virtual_display_is_supported_everywhere() {
    for gpu in [
        dev("15ad", "0405"),
        from_name("Microsoft Basic Display Adapter"),
    ] {
        let s = support(&gpu);
        assert!(s.display_capable);
        assert_eq!((s.min_native, s.max_native), (None, None));
        assert_eq!(native_range(&gpu), MacOsVersion::ALL.to_vec());
    }
}

#[test]
fn unknown_is_not_display_capable() {
    let gpu = ProfileGpu::default();
    assert!(!support(&gpu).display_capable);
    assert!(native_range(&gpu).is_empty());
}

#[test]
fn name_only_support_hints() {
    assert!(!support(&from_name("AMD Radeon RX 580 2048SP")).display_capable);
    assert!(support(&from_name("AMD Radeon RX 580")).display_capable);
    assert!(!support(&from_name("AMD Radeon RX Vega M GH Graphics")).display_capable);

    let rx6950 = from_name("AMD Radeon RX 6950 XT");
    assert_eq!(support(&rx6950).requirement, GpuRequirement::NootRx);
    assert_eq!(weg_spoof_alternative(&rx6950), Some([0xBF, 0x73, 0, 0]));
    assert_eq!(
        support(&from_name("AMD Radeon RX 6800 XT")).requirement,
        GpuRequirement::Standard
    );
    assert_eq!(
        support(&from_name("AMD Radeon RX 6650 XT")).requirement,
        GpuRequirement::NootRx
    );
    assert_eq!(
        support(&from_name("AMD Radeon RX 6700 XT")).requirement,
        GpuRequirement::NootRx
    );

    assert_eq!(
        support(&from_name("Intel(R) UHD Graphics 620")).requirement,
        spoof(0x5916)
    );
    assert_eq!(
        support(&from_name("Intel(R) HD Graphics 530")).requirement,
        spoof(0x5912)
    );
    assert_eq!(
        support(&from_name("Intel(R) HD Graphics 4400")).requirement,
        spoof(0x0412)
    );
    assert!(!support(&from_name("Intel(R) HD Graphics 2000")).display_capable);
    assert!(!support(&from_name("Intel(R) HD Graphics 2500")).display_capable);
    assert_eq!(
        support(&from_name("Intel(R) UHD Graphics 610")).requirement,
        spoof(0x3E92)
    );
    assert_eq!(
        support(&from_name("Intel(R) Iris(R) Pro Graphics 580")).max_native,
        Some(Monterey)
    );

    let gt730 = support(&from_name("NVIDIA GeForce GT 730"));
    assert!(gt730.notes.iter().any(|n| n.contains("0x1287")));
    assert!(!support(&from_name("NVIDIA Tesla K80")).display_capable);
    assert!(!support(&from_name("AMD Instinct MI50")).display_capable);
}

#[test]
fn manual_family_overrides_the_id_table() {
    // The user marked an RX 6800 XT id as Navi 22: family facts win.
    let gpu = ProfileGpu {
        vendor: GpuVendor::Amd,
        family: AmdNavi22,
        vendor_id: Some("1002".into()),
        device_id: Some("73bf".into()),
        ..Default::default()
    };
    assert_eq!(support(&gpu).requirement, GpuRequirement::NootRx);
    // Device id without vendor id still resolves through the vendor field.
    let gpu = ProfileGpu {
        vendor: GpuVendor::Intel,
        family: IntelCometLake,
        device_id: Some("9b41".into()),
        ..Default::default()
    };
    assert_eq!(support(&gpu).requirement, spoof(0x3E9B));
}

#[test]
fn support_family_matches_input() {
    for (family, _) in all_families() {
        let gpu = ProfileGpu {
            family: *family,
            ..Default::default()
        };
        assert_eq!(support(&gpu).family, *family);
    }
}

// ── Tables and labels ───────────────────────────────────────────────────────

const EVERY_FAMILY: [GpuFamily; 41] = [
    IntelGma,
    IntelIronLake,
    IntelSandyBridge,
    IntelIvyBridge,
    IntelHaswell,
    IntelBroadwell,
    IntelSkylake,
    IntelKabyLake,
    IntelCoffeeLake,
    IntelCometLake,
    IntelIceLake,
    IntelLowPower,
    IntelXe,
    IntelArc,
    AmdTeraScale,
    AmdGcn1,
    AmdGcn2,
    AmdGcn3,
    AmdPolaris,
    AmdLexa,
    AmdVega10,
    AmdVega20,
    AmdNavi10,
    AmdNavi12,
    AmdNavi14,
    AmdNavi21,
    AmdNavi22,
    AmdNavi23,
    AmdNavi24,
    AmdRdna3Plus,
    AmdApuVega,
    AmdApuRdna,
    AmdApuLegacy,
    NvidiaTesla,
    NvidiaFermi,
    NvidiaKepler,
    NvidiaMaxwell,
    NvidiaPascal,
    NvidiaModern,
    VirtualDisplay,
    GpuFamily::Unknown,
];

#[test]
fn all_families_lists_every_family_once() {
    let listed: Vec<GpuFamily> = all_families().iter().map(|(f, _)| *f).collect();
    assert_eq!(listed, EVERY_FAMILY.to_vec());
    assert!(all_families().iter().all(|(_, label)| !label.is_empty()));
    assert!(family_label(AmdNavi22).contains("Navi 22"));
    assert_eq!(family_label(GpuFamily::Unknown), "Unknown");
}

#[test]
fn device_tables_have_unique_ids() {
    for table in [ids::INTEL_DEVICES, ids::AMD_DEVICES, ids::NVIDIA_DEVICES] {
        let mut seen = HashSet::new();
        for d in table {
            assert!(seen.insert(d.id), "duplicate id 0x{:04X}", d.id);
        }
    }
    for table in [ids::INTEL_RANGES, ids::AMD_RANGES, ids::NVIDIA_RANGES] {
        for (i, a) in table.iter().enumerate() {
            assert!(a.lo <= a.hi, "range 0x{:04X}", a.lo);
            for b in &table[i + 1..] {
                assert!(
                    a.hi < b.lo || b.hi < a.lo,
                    "overlap 0x{:04X} / 0x{:04X}",
                    a.lo,
                    b.lo
                );
            }
        }
    }
}

#[test]
fn spoof_targets_are_native_ids() {
    let native = |table: &[ids::Device], id: u16| {
        table
            .iter()
            .any(|d| d.id == id && matches!(d.kind, Kind::Family | Kind::Gt1(None)))
    };
    for (table, name) in [(ids::INTEL_DEVICES, "intel"), (ids::AMD_DEVICES, "amd")] {
        for d in table {
            let target = match d.kind {
                Kind::Fake(t) | Kind::FakeGuess(t) | Kind::Gt1(Some(t)) | Kind::NootRx(Some(t)) => {
                    t
                }
                _ => continue,
            };
            assert!(
                native(table, target),
                "{name} 0x{:04X} -> 0x{target:04X}",
                d.id
            );
        }
    }
    for d in ids::INTEL_DEVICES
        .iter()
        .filter(|d| d.family == IntelSkylake)
    {
        if let Some(kaby) = ids::skylake_to_kaby(d.id) {
            let target = ids::INTEL_DEVICES.iter().find(|k| k.id == kaby);
            assert!(
                target.is_some_and(|k| k.family == IntelKabyLake && k.kind == Kind::Family),
                "0x{:04X} -> 0x{kaby:04X}",
                d.id
            );
        }
    }
}

#[test]
fn every_table_entry_is_identified_consistently() {
    for (vendor, table) in [
        ("8086", ids::INTEL_DEVICES),
        ("1002", ids::AMD_DEVICES),
        ("10de", ids::NVIDIA_DEVICES),
    ] {
        for d in table {
            let device = format!("{:04x}", d.id);
            let id = identify(Some(vendor), Some(&device), "");
            assert_eq!(id.family, d.family, "{vendor}:{device}");
            assert_eq!(id.model_name.as_deref(), Some(d.name), "{vendor}:{device}");
            // Support never panics and keeps the family.
            assert_eq!(support(&from_ids(vendor, &device, "")).family, d.family);
        }
    }
}

#[test]
fn every_family_has_notes() {
    for family in EVERY_FAMILY {
        let s = support(&ProfileGpu {
            family,
            ..Default::default()
        });
        assert!(!s.notes.is_empty(), "{family:?}");
        assert!(s.notes.iter().all(|n| !n.trim().is_empty()), "{family:?}");
    }
}

#[test]
fn identifies_short_and_glued_names() {
    use GpuVendor::{Amd, Intel, Nvidia};
    let cases: &[(&str, GpuVendor, GpuFamily)] = &[
        ("RX580", Amd, AmdPolaris),
        ("Radeon RX 590", Amd, AmdPolaris),
        ("AMD Radeon RX 560X", Amd, AmdPolaris),
        ("AMD Radeon Pro 580X", Amd, AmdPolaris),
        ("AMD Radeon RX 640", Amd, AmdLexa),
        ("Radeon 540X", Amd, AmdLexa),
        ("Vega 8", Amd, AmdApuVega),
        ("AMD Radeon Vega Mobile Gfx", Amd, AmdApuVega),
        ("Radeon RX 6600M", Amd, AmdNavi23),
        ("AMD Radeon RX 6700S", Amd, AmdNavi23),
        ("AMD Radeon RX 6550M", Amd, AmdNavi24),
        ("AMD Radeon RX 7600M XT", Amd, AmdRdna3Plus),
        ("AMD Radeon 610M", Amd, AmdApuRdna),
        ("AMD Radeon HD 7660G", Amd, AmdApuLegacy),
        ("AMD Radeon R5 230", Amd, AmdTeraScale),
        ("AMD Radeon R9 270X", Amd, AmdGcn1),
        ("AMD FirePro W7100", Amd, AmdGcn3),
        ("GTX1080Ti", Nvidia, NvidiaPascal),
        ("RTX 3060 Ti", Nvidia, NvidiaModern),
        ("NVIDIA GeForce GTX 1660 SUPER", Nvidia, NvidiaModern),
        ("NVIDIA GeForce GT 750M", Nvidia, NvidiaKepler),
        ("NVIDIA GeForce GTX 965M", Nvidia, NvidiaMaxwell),
        ("NVIDIA NVS 510", Nvidia, NvidiaKepler),
        ("NVIDIA Tesla V100-PCIE-16GB", Nvidia, NvidiaModern),
        ("GeForce 7300 GT", Nvidia, GpuFamily::Unknown),
        ("HD4000", Intel, IntelIvyBridge),
        ("UHD630", Intel, IntelCoffeeLake),
        ("Intel(R) HD Graphics Family", Intel, GpuFamily::Unknown),
        (
            "Intel(R) Q45/Q43 Express Chipset (Microsoft Corporation - WDDM 1.1)",
            Intel,
            IntelGma,
        ),
        ("Intel(R) Iris(R) Plus Graphics G7", Intel, IntelIceLake),
    ];
    for (name, vendor, family) in cases {
        let id = identify(None, None, name);
        assert_eq!((id.vendor, id.family), (*vendor, *family), "{name}");
    }
}

#[test]
fn name_patterns_compile() {
    // Forces every lazily built pattern, so a typo fails here and never at runtime.
    for pattern in super::names::all_patterns() {
        assert!(!pattern.as_str().is_empty());
    }
}

/// First and last release a GPU runs on without root patches.
type Span = Option<(MacOsVersion, MacOsVersion)>;

fn native_span(gpu: &ProfileGpu) -> Span {
    let range = native_range(gpu);
    Some((*range.first()?, *range.last()?))
}

#[test]
fn version_matrix_for_common_gpus() {
    let cases: &[(&str, &str, Span)] = &[
        // Intel
        ("8086", "0046", Some((HighSierra, HighSierra))),
        ("8086", "0116", Some((HighSierra, HighSierra))),
        ("8086", "0166", Some((HighSierra, BigSur))),
        ("8086", "0152", None),
        ("8086", "0412", Some((HighSierra, Monterey))),
        ("8086", "0a2e", Some((HighSierra, Monterey))),
        ("8086", "1626", Some((HighSierra, Monterey))),
        ("8086", "1916", Some((HighSierra, Tahoe))),
        ("8086", "1932", Some((HighSierra, Monterey))),
        ("8086", "591b", Some((HighSierra, Tahoe))),
        ("8086", "87c0", Some((Mojave, Tahoe))),
        ("8086", "3e91", Some((Mojave, Tahoe))),
        ("8086", "3ea5", Some((HighSierra, Tahoe))),
        ("8086", "9bc5", Some((Catalina, Tahoe))),
        ("8086", "9b41", Some((Catalina, Tahoe))),
        ("8086", "8a5c", Some((Catalina, Tahoe))),
        ("8086", "8a56", None),
        ("8086", "9a49", None),
        ("8086", "a780", None),
        ("8086", "56a0", None),
        // AMD
        ("1002", "6798", Some((HighSierra, Monterey))),
        ("1002", "6939", Some((HighSierra, Monterey))),
        ("1002", "67df", Some((HighSierra, Tahoe))),
        ("1002", "67ef", Some((HighSierra, Tahoe))),
        ("1002", "699f", Some((HighSierra, Tahoe))),
        ("1002", "687f", Some((HighSierra, Tahoe))),
        ("1002", "66af", Some((Mojave, Tahoe))),
        ("1002", "731f", Some((Catalina, Tahoe))),
        ("1002", "7340", Some((Catalina, Tahoe))),
        ("1002", "7360", Some((Catalina, Tahoe))),
        ("1002", "73bf", Some((BigSur, Tahoe))),
        ("1002", "73af", Some((BigSur, Tahoe))),
        ("1002", "73df", Some((Monterey, Tahoe))),
        ("1002", "73ff", Some((Monterey, Tahoe))),
        ("1002", "73ef", Some((Monterey, Tahoe))),
        ("1002", "743f", None),
        ("1002", "744c", None),
        ("1002", "7550", None),
        ("1002", "6fdf", None),
        ("1002", "15dd", Some((Catalina, Tahoe))),
        ("1002", "1638", Some((Catalina, Tahoe))),
        ("1002", "1681", None),
        ("1002", "15bf", None),
        // NVIDIA
        ("10de", "0a65", Some((HighSierra, HighSierra))),
        ("10de", "0f02", Some((HighSierra, HighSierra))),
        ("10de", "1287", Some((HighSierra, BigSur))),
        ("10de", "1004", Some((HighSierra, BigSur))),
        ("10de", "13c2", Some((HighSierra, HighSierra))),
        ("10de", "1b81", Some((HighSierra, HighSierra))),
        ("10de", "1f08", None),
        ("10de", "2204", None),
        // Virtual machines
        ("15ad", "0405", Some((HighSierra, Tahoe))),
        ("1af4", "1050", Some((HighSierra, Tahoe))),
    ];
    for (vendor, device, expected) in cases {
        let gpu = dev(vendor, device);
        assert_eq!(native_span(&gpu), *expected, "{vendor}:{device}");
        // The native range never has holes.
        let range = native_range(&gpu);
        if let (Some(first), Some(last)) = (range.first(), range.last()) {
            let span = MacOsVersion::ALL
                .iter()
                .filter(|v| (first..=last).contains(v))
                .count();
            assert_eq!(span, range.len(), "{vendor}:{device}");
        }
    }
}

#[test]
fn root_patch_paths_follow_oclp_metal_classes() {
    // Metal 3802 / 31001 classes reach macOS 26 with OCLP 3.0.
    for (vendor, device) in [
        ("8086", "0166"),
        ("8086", "0412"),
        ("8086", "1616"),
        ("8086", "193b"),
        ("1002", "6798"),
        ("1002", "665c"),
        ("1002", "7300"),
        ("10de", "1287"),
    ] {
        let s = support(&dev(vendor, device));
        assert_eq!(s.max_with_root_patch, Some(Tahoe), "{vendor}:{device}");
        assert!(
            s.notes.iter().any(|n| n.contains("OCLP")),
            "{vendor}:{device}"
        );
    }
    // Non-Metal classes and natively supported GPUs have no patch path.
    for (vendor, device) in [
        ("8086", "0046"),
        ("8086", "0126"),
        ("1002", "68b8"),
        ("10de", "0622"),
        ("10de", "06c0"),
        ("10de", "1380"),
        ("10de", "1c82"),
        ("8086", "3e9b"),
        ("8086", "1912"),
        ("1002", "67df"),
        ("1002", "73bf"),
        ("1002", "15d8"),
        ("10de", "2684"),
    ] {
        assert_eq!(
            support(&dev(vendor, device)).max_with_root_patch,
            None,
            "{vendor}:{device}"
        );
    }
}

#[test]
fn requirements_and_boot_args() {
    for id in ["15dd", "15d8", "1636", "164c", "1638", "15e7"] {
        let s = support(&dev("1002", id));
        assert_eq!(s.requirement, GpuRequirement::NootedRed, "{id}");
        assert!(s.boot_args.is_empty(), "{id}");
    }
    for id in ["73df", "73a5", "73af", "73ef", "73e1"] {
        let s = support(&dev("1002", id));
        assert_eq!(s.requirement, GpuRequirement::NootRx, "{id}");
        assert!(!s.boot_args.contains(&"agdpmod=pikera"), "{id}");
        assert!(s.notes.iter().any(|n| n.contains("NootRX")), "{id}");
    }
    assert_eq!(
        weg_spoof_alternative(&dev("1002", "73e1")),
        Some([0xE3, 0x73, 0, 0])
    );
    for id in ["731f", "7340", "73bf", "73a3", "73ff", "73e3"] {
        let s = support(&dev("1002", id));
        assert_eq!(s.requirement, GpuRequirement::Standard, "{id}");
        assert!(s.boot_args.contains(&"agdpmod=pikera"), "{id}");
        assert!(!s.boot_args.contains(&"-radcodec"), "{id}");
    }
    for id in ["67df", "67ff", "687f", "66af", "7360"] {
        let s = support(&dev("1002", id));
        assert!(!s.boot_args.contains(&"agdpmod=pikera"), "{id}");
        assert!(
            s.notes.iter().any(|n| n.contains("WhateverGreen 1.7.1")),
            "{id}"
        );
    }
    for (id, target) in [
        ("6981", 0x67FF),
        ("6985", 0x67FF),
        ("6987", 0x67FF),
        ("67b1", 0x67B0),
    ] {
        let s = support(&dev("1002", id));
        assert_eq!(s.requirement, spoof(target), "{id}");
        assert!(s.boot_args.contains(&"-radcodec"), "{id}");
    }
    let navi21 = support(&dev("1002", "73bf"));
    assert!(navi21.notes.iter().any(|n| n.contains("11.4")));
    let navi23 = support(&dev("1002", "73ff"));
    assert!(navi23.notes.iter().any(|n| n.contains("12.1")));
    // Release-independent ids keep their spoof on every target.
    let cml_u = dev("8086", "9b41");
    for version in MacOsVersion::ALL {
        assert_eq!(
            device_id_for(&cml_u, version),
            Some([0x9B, 0x3E, 0, 0]),
            "{version:?}"
        );
    }
    assert_eq!(device_id_for(&dev("8086", "3e9b"), Tahoe), None);
    assert_eq!(device_id_for(&dev("1002", "67df"), Tahoe), None);
}

#[test]
fn skylake_needs_its_model_for_the_kaby_lake_spoof() {
    // Family only: nothing tells which Kaby Lake id to use.
    let family_only = ProfileGpu {
        vendor: GpuVendor::Intel,
        family: IntelSkylake,
        is_igpu: true,
        ..Default::default()
    };
    let s = support(&family_only);
    assert_eq!(s.requirement, GpuRequirement::Standard);
    assert_eq!(
        (s.max_native, s.max_with_root_patch),
        (Some(Monterey), Some(Tahoe))
    );
    assert!(!natively_supported_on(&family_only, Ventura));

    let by_name = from_name("Intel(R) HD Graphics 520");
    let s = support(&by_name);
    assert_eq!(s.requirement, spoof(0x5916));
    assert_eq!((s.max_native, s.max_with_root_patch), (None, None));
    assert!(natively_supported_on(&by_name, Tahoe));

    for (device, kaby) in [
        ("1912", 0x5912),
        ("191b", 0x591B),
        ("191d", 0x591B),
        ("1916", 0x5916),
        ("191e", 0x591E),
    ] {
        let gpu = dev("8086", device);
        assert_eq!(support(&gpu).requirement, spoof(kaby), "{device}");
        assert_eq!(device_id_for(&gpu, Ventura), Some(le(kaby)), "{device}");
    }

    // GT4 has no Kaby Lake counterpart.
    let gt4 = dev("8086", "193a");
    let s = support(&gt4);
    assert_eq!(s.max_native, Some(Monterey));
    assert_eq!(s.max_with_root_patch, Some(Tahoe));
    assert_eq!(s.requirement, spoof(0x193B));
    assert_eq!(device_id_for(&gt4, Catalina), Some(le(0x193B)));
    assert!(s.notes.iter().any(|n| n.contains("OCLP Skylake")));

    // 0x192A is a GT3 server part (Linux pciids.h), not GT4.
    let gt3 = dev("8086", "192a");
    let s = support(&gt3);
    assert_eq!(s.requirement, spoof(0x5927));
    assert_eq!(s.max_native, None);
    assert!(s.notes.iter().any(|n| n.contains("inferred")));
    assert_eq!(device_id_for(&gt3, Monterey), Some(le(0x1927)));
}

#[test]
fn more_name_hints() {
    let cml_u = from_name("Intel Corporation CometLake-U GT2 [UHD Graphics 620]");
    assert_eq!(cml_u.family, IntelCometLake);
    assert_eq!(support(&cml_u).requirement, spoof(0x3E9B));
    let cml_gt1 = support(&from_name(
        "Intel Corporation CometLake-S GT1 [UHD Graphics 610]",
    ));
    assert!(cml_gt1.notes.iter().any(|n| n.contains("GT1")));
    assert_eq!(
        support(&from_name("Intel(R) HD Graphics P630")).requirement,
        spoof(0x591B)
    );
    assert_eq!(
        support(&from_name("Intel(R) UHD Graphics P630")).requirement,
        spoof(0x3E92)
    );

    for name in [
        "AMD Radeon R7 240",
        "AMD Radeon R5 340",
        "AMD Radeon HD 8570",
    ] {
        let gpu = from_name(name);
        assert_eq!(gpu.family, AmdGcn1, "{name}");
        assert!(!support(&gpu).display_capable, "{name}");
    }
    for name in ["AMD Radeon HD 7750", "AMD Radeon R7 250X"] {
        assert!(
            support(&from_name(name)).boot_args.contains(&"radpg=15"),
            "{name}"
        );
    }
    assert!(support(&from_name("AMD Radeon HD 7970")).display_capable);

    assert_eq!(from_name("NVIDIA NVS 300").family, NvidiaTesla);
    assert_eq!(from_name("NVIDIA NVS 315").family, NvidiaFermi);
    assert_eq!(from_name("NVIDIA Quadro K620").family, NvidiaMaxwell);
    assert_eq!(from_name("NVIDIA Quadro M4000").family, NvidiaMaxwell);

    // A data-centre board keeps its family but cannot drive a display.
    let k80 = from_name("NVIDIA Tesla K80");
    assert_eq!(k80.family, NvidiaKepler);
    assert!(!support(&k80).display_capable);
}

#[test]
fn identify_reports_model_names() {
    let cases = [
        ("1002", "73bf", "Radeon RX 6800 / 6800 XT / 6900 XT"),
        ("1002", "67df", "Radeon RX 470/480/570/580/590"),
        ("10de", "1287", "GeForce GT 730 (GK208)"),
        ("10de", "0f02", "GeForce GT 730 (GF108)"),
        ("8086", "3e92", "UHD Graphics 630"),
        ("8086", "9bc8", "UHD Graphics 630"),
        ("15ad", "0405", "VMware SVGA"),
        ("1414", "5353", "Hyper-V video"),
    ];
    for (vendor, device, name) in cases {
        assert_eq!(
            identify(Some(vendor), Some(device), "")
                .model_name
                .as_deref(),
            Some(name),
            "{vendor}:{device}"
        );
    }
}

#[test]
fn board_brand_words_do_not_shadow_model_numbers() {
    use GpuVendor::Amd;
    let cases: &[(&str, GpuVendor, GpuFamily)] = &[
        ("ASUS ROG Strix RX 580", Amd, AmdPolaris),
        ("ASUS Phoenix RX 550", Amd, AmdLexa),
        ("ROG Strix Radeon RX 6700 XT", Amd, AmdNavi22),
        (
            "Advanced Micro Devices, Inc. [AMD/ATI] Strix Halo [Radeon Graphics / Radeon 8050S / 8060S Graphics]",
            Amd,
            AmdApuRdna,
        ),
        ("AMD Strix Point", Amd, AmdApuRdna),
        (
            "Advanced Micro Devices, Inc. [AMD/ATI] Ellesmere [Radeon RX 470/480/570/570X/580/580X/590]",
            Amd,
            AmdPolaris,
        ),
        (
            "Advanced Micro Devices, Inc. [AMD/ATI] Lexa PRO [Radeon 540/540X/550/550X / RX 540X/550/550X]",
            Amd,
            AmdLexa,
        ),
        ("Bonaire [Radeon RX 455 OEM]", Amd, AmdGcn2),
    ];
    for (name, vendor, family) in cases {
        let id = identify(None, None, name);
        assert_eq!((id.vendor, id.family), (*vendor, *family), "{name}");
    }
}

#[test]
fn bare_hd_numbers_pick_the_right_vendor() {
    for (name, vendor, family) in [
        ("HD 7970", GpuVendor::Amd, AmdGcn1),
        ("HD7750", GpuVendor::Amd, AmdGcn1),
        ("HD 6870", GpuVendor::Amd, AmdTeraScale),
        ("HD 4000", GpuVendor::Intel, IntelIvyBridge),
        ("HD 4600", GpuVendor::Intel, IntelHaswell),
        ("HD 3000", GpuVendor::Intel, IntelSandyBridge),
        ("HD 530", GpuVendor::Intel, IntelSkylake),
        ("AMD Radeon HD 6310 Graphics", GpuVendor::Amd, AmdApuLegacy),
    ] {
        let id = identify(None, None, name);
        assert_eq!((id.vendor, id.family), (vendor, family), "{name}");
    }
    assert!(!support(&from_name("AMD Radeon HD 6310 Graphics")).display_capable);
}

#[test]
fn cape_verde_radpg_only_on_desktop_boards() {
    for id in ["683f", "683d", "6837", "682b"] {
        assert!(
            support(&dev("1002", id)).boot_args.contains(&"radpg=15"),
            "{id}"
        );
    }
    // Mobile Cape Verde (Venus/Heathrow/Chelsea) and other GCN 1 chips.
    for id in ["6821", "6825", "682f", "6798", "6818"] {
        assert!(
            !support(&dev("1002", id)).boot_args.contains(&"radpg=15"),
            "{id}"
        );
    }
}

#[test]
fn newer_amd_apus_are_unsupported() {
    for id in ["1114", "1902", "1586", "13c0"] {
        let gpu = dev("1002", id);
        assert_eq!((gpu.family, gpu.is_igpu), (AmdApuRdna, true), "{id}");
        assert!(!support(&gpu).display_capable, "{id}");
    }
}

#[test]
fn clarkdale_desktop_cannot_drive_a_display() {
    let s = support(&dev("8086", "0042"));
    assert!(!s.display_capable);
    assert!(s.notes.iter().any(|n| n.contains("LVDS")));
    let arrandale = support(&dev("8086", "0046"));
    assert!(arrandale.display_capable);
    assert_eq!(arrandale.max_native, Some(HighSierra));
    assert_eq!(arrandale.max_with_root_patch, None);
}
