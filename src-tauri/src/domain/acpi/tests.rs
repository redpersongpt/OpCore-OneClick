//! Scenario tests: synthetic DSDTs/SSDTs (built with the encoder) for the
//! board families the generator targets, checked end to end: facts, then
//! every SSDT kind with and without the tables.

use super::aml::{self, encode_body, FieldUnit, ObjType, Resource, Term};
use super::dsdt::{parse_path, AcpiTable, AcpiTables};
use super::parse::find_all;
use super::ssdt::{
    generate, generate_with_tables, GeneratedSsdt, SsdtKind, ERR_INSUFFICIENT, ERR_NOT_NEEDED,
};
use crate::domain::model::AcpiFacts;

// ── Builders ────────────────────────────────────────────────────────────────

fn table(sig: &[u8; 4], id: &str, terms: &[Term]) -> Vec<u8> {
    aml::table(sig, 2, "OCTEST", id, 1, &encode_body(terms))
}

fn dev(name: &str, body: Vec<Term>) -> Term {
    Term::device(name, body)
}

fn eisa(name: &str, id: &str) -> Term {
    Term::name(name, Term::EisaId(id.into()))
}

fn hid(id: &str) -> Term {
    Term::name("_HID", Term::str(id))
}

fn adr(v: u64) -> Term {
    Term::name("_ADR", Term::int(v))
}

fn uid(v: u64) -> Term {
    Term::name("_UID", Term::int(v))
}

/// `Method (_STA) { If ((VAR == val)) { Return (a) } Return (b) }`
fn sta_on(var: &str, val: u64, a: u64, b: u64) -> Term {
    Term::method(
        "_STA",
        0,
        vec![
            Term::if_then(
                Term::equal(Term::path(var), Term::int(val)),
                vec![Term::ret(Term::int(a))],
            ),
            Term::ret(Term::int(b)),
        ],
    )
}

fn gnvs(units: &[(&str, u32)]) -> Vec<Term> {
    vec![
        Term::OpRegion {
            name: "GNVS".into(),
            space: 0,
            offset: Box::new(Term::int(0x8FF0_0000)),
            len: Box::new(Term::int(0x0100)),
        },
        Term::Field {
            region: "GNVS".into(),
            flags: 0x10,
            units: units
                .iter()
                .map(|(n, b)| FieldUnit {
                    name: Some((*n).into()),
                    bits: *b,
                })
                .collect(),
        },
    ]
}

fn ec_device(name: &str, sta: Option<Term>) -> Term {
    let mut body = vec![
        eisa("_HID", "PNP0C09"),
        Term::name("_UID", Term::int(1)),
        Term::name(
            "_CRS",
            Term::Resources(vec![
                Resource::Io {
                    min: 0x62,
                    max: 0x62,
                    align: 0,
                    len: 1,
                },
                Resource::Io {
                    min: 0x66,
                    max: 0x66,
                    align: 0,
                    len: 1,
                },
            ]),
        ),
        Term::name("_GPE", Term::int(0x17)),
    ];
    body.extend(sta);
    dev(name, body)
}

fn rtc_device(name: &str, sta: Option<Term>) -> Term {
    let mut body = vec![
        eisa("_HID", "PNP0B00"),
        Term::name(
            "_CRS",
            Term::Resources(vec![
                Resource::Io {
                    min: 0x70,
                    max: 0x70,
                    align: 1,
                    len: 2,
                },
                Resource::Io {
                    min: 0x74,
                    max: 0x74,
                    align: 1,
                    len: 4,
                },
                Resource::IrqNoFlags { mask: 1 << 8 },
            ]),
        ),
    ];
    body.extend(sta);
    dev(name, body)
}

fn osi_ini(strings: &[&str]) -> Term {
    Term::method(
        "_INI",
        0,
        strings
            .iter()
            .enumerate()
            .map(|(i, s)| {
                Term::if_then(
                    Term::call("_OSI", vec![Term::str(s)]),
                    vec![Term::store(
                        Term::int(0x07D0 + i as u64),
                        Term::path("OSYS"),
                    )],
                )
            })
            .collect(),
    )
}

/// Coffee Lake Z390 desktop: PCI0/LPCB, real EC named EC with an _STA,
/// AWAC + RTC switched by STAS, Processor objects in \_PR.
fn intel_desktop() -> Vec<Vec<u8>> {
    let mut terms = vec![Term::name("STAS", Term::int(1))];
    terms.extend(gnvs(&[("OSYS", 16), ("ECON", 8)]));
    terms.push(Term::scope(
        "\\_PR",
        (0..4u8)
            .map(|i| Term::Processor {
                name: format!("CPU{i}"),
                id: i + 1,
                pblk: 0x1810,
                pblk_len: 6,
                body: vec![],
            })
            .collect(),
    ));
    terms.push(Term::scope(
        "\\_SB",
        vec![dev(
            "PCI0",
            vec![
                eisa("_HID", "PNP0A08"),
                eisa("_CID", "PNP0A03"),
                osi_ini(&["Windows 2009", "Windows 2015"]),
                dev("GFX0", vec![adr(0x0002_0000)]),
                dev(
                    "XHC",
                    vec![
                        adr(0x0014_0000),
                        dev("RHUB", vec![adr(0), dev("HS01", vec![adr(1)])]),
                    ],
                ),
                dev("HDAS", vec![adr(0x001F_0003)]),
                dev("SBUS", vec![adr(0x001F_0004)]),
                dev(
                    "LPCB",
                    vec![
                        adr(0x001F_0000),
                        ec_device("EC", Some(sta_on("ECON", 1, 0x0F, 0))),
                        dev("AWAC", vec![hid("ACPI000E"), sta_on("STAS", 0, 0x0F, 0)]),
                        rtc_device("RTC", Some(sta_on("STAS", 1, 0x0F, 0))),
                        dev("HPET", vec![eisa("_HID", "PNP0103")]),
                    ],
                ),
            ],
        )],
    ));
    vec![table(b"DSDT", "CFL-Z390", &terms)]
}

/// Alder Lake desktop: PC00/LPCB, ACPI0007 CPUs in \_SB (with a CPU SSDT),
/// AWAC with STAS, no EC.
fn alder_lake() -> Vec<Vec<u8>> {
    let mut terms = vec![Term::name("STAS", Term::int(1))];
    terms.push(Term::scope(
        "\\_SB",
        vec![
            dev(
                "PC00",
                vec![
                    eisa("_HID", "PNP0A08"),
                    eisa("_CID", "PNP0A03"),
                    dev("XHCI", vec![adr(0x0014_0000), dev("RHUB", vec![adr(0)])]),
                    dev("SBUS", vec![adr(0x001F_0004)]),
                    dev("HDAS", vec![adr(0x001F_0003)]),
                    dev(
                        "LPCB",
                        vec![
                            adr(0x001F_0000),
                            dev("AWAC", vec![hid("ACPI000E"), sta_on("STAS", 0, 0x0F, 0)]),
                            rtc_device("RTC", Some(sta_on("STAS", 1, 0x0F, 0))),
                        ],
                    ),
                ],
            ),
            dev("PR00", vec![hid("ACPI0007"), uid(0)]),
            dev("PR01", vec![hid("ACPI0007"), uid(1)]),
            dev("PR02", vec![hid("ACPI0007"), uid(2)]),
            dev("PR03", vec![hid("ACPI0007"), uid(3)]),
        ],
    ));
    let cpu_ssdt = table(
        b"SSDT",
        "CpuSsdt",
        &[
            Term::external("\\_SB.PR00", ObjType::Device),
            Term::scope(
                "\\_SB.PR00",
                vec![Term::method("_PPC", 0, vec![Term::ret(Term::int(0))])],
            ),
        ],
    );
    vec![table(b"DSDT", "ADL-Z690", &terms), cpu_ssdt]
}

/// AMD B550: SBRG at 0x00140003 without EC, ACPI0007 CPUs under \_SB.PLTF,
/// XHCI controllers behind GPP bridges (one named XHC1).
fn amd_b550() -> Vec<Vec<u8>> {
    let cpus: Vec<Term> = (0..16u64)
        .map(|i| dev(&format!("C0{i:02X}"), vec![hid("ACPI0007"), uid(i)]))
        .collect();
    let terms = vec![Term::scope(
        "\\_SB",
        vec![
            dev(
                "PCI0",
                vec![
                    eisa("_HID", "PNP0A08"),
                    eisa("_CID", "PNP0A03"),
                    dev("D00F", vec![adr(0x0002_0000)]),
                    dev("SMBS", vec![adr(0x0014_0000)]),
                    dev(
                        "SBRG",
                        vec![
                            adr(0x0014_0003),
                            rtc_device("RTC", None),
                            dev("HPET", vec![eisa("_HID", "PNP0103")]),
                        ],
                    ),
                    dev(
                        "GP17",
                        vec![
                            adr(0x0008_0001),
                            dev("XHC0", vec![adr(0x0000_0003), dev("RHUB", vec![adr(0)])]),
                            dev("XHC1", vec![adr(0x0000_0004), dev("RHUB", vec![adr(0)])]),
                        ],
                    ),
                ],
            ),
            dev("PLTF", cpus),
        ],
    )];
    vec![table(b"DSDT", "B550", &terms)]
}

/// Kaby Lake-R laptop: EC0 without _STA, GPI0 gated by SBRG/GPEN, I2C
/// touchpad, Windows _OSI checks, OSID method, NBCF.
fn intel_laptop() -> Vec<Vec<u8>> {
    let mut terms = gnvs(&[("OSYS", 16), ("SBRG", 32), ("GPEN", 8)]);
    terms.push(Term::name("NBCF", Term::int(0)));
    terms.push(Term::method("OSID", 0, vec![Term::ret(Term::int(1))]));
    terms.push(Term::scope(
        "\\_SB",
        vec![dev(
            "PCI0",
            vec![
                eisa("_HID", "PNP0A08"),
                eisa("_CID", "PNP0A03"),
                osi_ini(&[
                    "Windows 2009",
                    "Windows 2012",
                    "Windows 2015",
                    "Windows 2017",
                ]),
                dev("GFX0", vec![adr(0x0002_0000)]),
                dev("XHC", vec![adr(0x0014_0000), dev("RHUB", vec![adr(0)])]),
                dev(
                    "GPI0",
                    vec![
                        Term::method("_HID", 0, vec![Term::ret(Term::str("INT344B"))]),
                        Term::method(
                            "_STA",
                            0,
                            vec![
                                Term::if_then(
                                    Term::equal(Term::path("SBRG"), Term::int(0)),
                                    vec![Term::ret(Term::int(0))],
                                ),
                                Term::if_then(
                                    Term::equal(Term::path("GPEN"), Term::int(0)),
                                    vec![Term::ret(Term::int(0))],
                                ),
                                Term::ret(Term::int(0x0F)),
                            ],
                        ),
                    ],
                ),
                dev(
                    "I2C1",
                    vec![
                        adr(0x0015_0001),
                        dev(
                            "TPD0",
                            vec![hid("ELAN0000"), Term::name("_CID", Term::str("PNP0C50"))],
                        ),
                    ],
                ),
                dev("SBUS", vec![adr(0x001F_0004)]),
                dev("HDAS", vec![adr(0x001F_0003)]),
                dev(
                    "LPCB",
                    vec![
                        adr(0x001F_0000),
                        ec_device("EC0", None),
                        rtc_device("RTC", None),
                        dev("HPET", vec![eisa("_HID", "PNP0103")]),
                    ],
                ),
            ],
        )],
    ));
    vec![table(b"DSDT", "KBL-R", &terms)]
}

/// X99 HEDT: LPC0, uncore bridges with PRBM, Processor objects under
/// \_SB.SCK0, EHC1/EHC2 + XHCI, RTC with a partial range.
fn hedt_x99() -> Vec<Vec<u8>> {
    let mut terms = vec![Term::name("PRBM", Term::int(0x0F))];
    let unc0 = dev(
        "UNC0",
        vec![
            eisa("_HID", "PNP0A03"),
            uid(0xFF),
            Term::method(
                "_STA",
                0,
                vec![
                    Term::if_then(
                        Term::equal(Term::path("PRBM"), Term::int(0)),
                        vec![Term::ret(Term::int(0))],
                    ),
                    Term::ret(Term::int(0x0F)),
                ],
            ),
        ],
    );
    terms.push(Term::scope(
        "\\_SB",
        vec![
            unc0,
            dev(
                "PCI0",
                vec![
                    eisa("_HID", "PNP0A08"),
                    eisa("_CID", "PNP0A03"),
                    dev(
                        "BR2A",
                        vec![adr(0x0002_0000), Term::name("_PRT", Term::Package(vec![]))],
                    ),
                    dev("XHCI", vec![adr(0x0014_0000), dev("RHUB", vec![adr(0)])]),
                    dev("EHC1", vec![adr(0x001D_0000), dev("HUBN", vec![adr(0)])]),
                    dev("EHC2", vec![adr(0x001A_0000), dev("HUBN", vec![adr(0)])]),
                    dev("HDEF", vec![adr(0x001B_0000)]),
                    dev("SBUS", vec![adr(0x001F_0003)]),
                    dev("LPC0", vec![adr(0x001F_0000), rtc_device("RTC", None)]),
                ],
            ),
            dev(
                "SCK0",
                (0..4u8)
                    .map(|i| Term::Processor {
                        name: format!("CP0{i}"),
                        id: i,
                        pblk: 0x410,
                        pblk_len: 6,
                        body: vec![],
                    })
                    .collect(),
            ),
        ],
    ));
    vec![table(b"DSDT", "X99", &terms)]
}

const ALL_KINDS: [SsdtKind; 15] = [
    SsdtKind::EcUsbx { laptop: false },
    SsdtKind::EcUsbx { laptop: true },
    SsdtKind::Plug,
    SsdtKind::Awac,
    SsdtKind::Pmc,
    SsdtKind::RhubReset,
    SsdtKind::Xosi,
    SsdtKind::Gpi0,
    SsdtKind::Pnlf { uid: 16 },
    SsdtKind::Unc,
    SsdtKind::Rtc0Range,
    SsdtKind::Cpur,
    SsdtKind::Imei,
    SsdtKind::SbusMchc,
    SsdtKind::Als0,
];

fn tables(images: Vec<Vec<u8>>) -> AcpiTables {
    AcpiTables::from_images(images).expect("valid images")
}

fn ok(kind: SsdtKind, t: &AcpiTables) -> GeneratedSsdt {
    match generate_with_tables(&kind, t) {
        Ok(g) => g,
        Err(e) => panic!("{kind:?}: {e}"),
    }
}

fn ok_facts(kind: SsdtKind, f: &AcpiFacts) -> GeneratedSsdt {
    match generate(&kind, f) {
        Ok(g) => g,
        Err(e) => panic!("{kind:?}: {e}"),
    }
}

/// Header/checksum validity and the objects the SSDT defines (parsed back).
fn check(g: &GeneratedSsdt) -> AcpiTables {
    assert!(g.file_name.starts_with("SSDT-") && g.file_name.ends_with(".aml"));
    assert_eq!(&g.aml[0..4], b"SSDT");
    assert_eq!(
        u32::from_le_bytes([g.aml[4], g.aml[5], g.aml[6], g.aml[7]]) as usize,
        g.aml.len()
    );
    assert_eq!(aml::byte_sum(&g.aml), 0, "{} checksum", g.file_name);
    assert!(g
        .dsl
        .contains("DefinitionBlock (\"\", \"SSDT\", 2, \"OCLICK\""));
    for p in &g.patches {
        assert_eq!(p.find.len(), p.replace.len());
        assert!(!p.find.is_empty() && p.find.len() % 2 == 0);
        assert!(g.dsl.contains(&p.find), "patch listed in the DSL header");
    }
    let parsed = AcpiTables::new(vec![AcpiTable::parse(g.aml.clone()).expect("parses")]);
    assert!(
        parsed.ns().objects.iter().all(|o| !o.heuristic),
        "{} decodes cleanly",
        g.file_name
    );
    parsed
}

fn has(t: &AcpiTables, path: &str) -> bool {
    parse_path(path).is_some_and(|p| t.exists(&p))
}

/// The patch find pattern occurs exactly once in the target table.
fn unique_in(images: &[Vec<u8>], sig: &[u8; 4], find_hex: &str) -> bool {
    let find: Vec<u8> = (0..find_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&find_hex[i..i + 2], 16).unwrap_or(0))
        .collect();
    let count: usize = images
        .iter()
        .filter(|i| i.starts_with(sig))
        .map(|i| find_all(i, &find).len())
        .sum();
    count == 1
}

// ── Facts ───────────────────────────────────────────────────────────────────

#[test]
fn facts_intel_desktop() {
    let f = tables(intel_desktop()).facts();
    assert_eq!(f.pci_root.as_deref(), Some("\\_SB.PCI0"));
    assert_eq!(f.lpc_bridge.as_deref(), Some("\\_SB.PCI0.LPCB"));
    assert_eq!(f.ec_path.as_deref(), Some("\\_SB.PCI0.LPCB.EC"));
    assert!(f.ec_has_sta);
    assert_eq!(
        f.cpu_paths,
        vec!["\\_PR.CPU0", "\\_PR.CPU1", "\\_PR.CPU2", "\\_PR.CPU3"]
    );
    assert!(!f.cpu_uses_acpi0007);
    assert_eq!(f.awac_path.as_deref(), Some("\\_SB.PCI0.LPCB.AWAC"));
    assert!(f.awac_has_stas);
    assert_eq!(f.rtc_path.as_deref(), Some("\\_SB.PCI0.LPCB.RTC"));
    assert_eq!(f.hpet_path.as_deref(), Some("\\_SB.PCI0.LPCB.HPET"));
    assert_eq!(f.igpu_path.as_deref(), Some("\\_SB.PCI0.GFX0"));
    assert_eq!(f.xhci_paths, vec!["\\_SB.PCI0.XHC"]);
    assert_eq!(f.rhub_paths, vec!["\\_SB.PCI0.XHC.RHUB"]);
    assert_eq!(f.smbus_path.as_deref(), Some("\\_SB.PCI0.SBUS"));
    assert_eq!(f.gpio_path, None);
    assert!(!f.pnlf_exists);
    assert!(f.has_osi_windows);
    assert_eq!(f.dsdt_oem_table_id.as_deref(), Some("CFL-Z390"));
    assert_eq!(f.dsdt_length, Some(intel_desktop()[0].len() as u32));
}

#[test]
fn facts_alder_lake() {
    let f = tables(alder_lake()).facts();
    assert_eq!(f.pci_root.as_deref(), Some("\\_SB.PC00"));
    assert_eq!(f.lpc_bridge.as_deref(), Some("\\_SB.PC00.LPCB"));
    assert_eq!(f.ec_path, None);
    assert!(f.cpu_uses_acpi0007);
    assert_eq!(
        f.cpu_paths,
        vec!["\\_SB.PR00", "\\_SB.PR01", "\\_SB.PR02", "\\_SB.PR03"]
    );
    assert_eq!(f.xhci_paths, vec!["\\_SB.PC00.XHCI"]);
    assert!(f.awac_has_stas);
    assert!(!f.has_osi_windows);
}

#[test]
fn facts_amd_b550() {
    let f = tables(amd_b550()).facts();
    assert_eq!(f.pci_root.as_deref(), Some("\\_SB.PCI0"));
    assert_eq!(f.lpc_bridge.as_deref(), Some("\\_SB.PCI0.SBRG"));
    assert_eq!(f.ec_path, None);
    assert!(f.cpu_uses_acpi0007);
    assert_eq!(f.cpu_paths.len(), 16);
    assert_eq!(f.cpu_paths[0], "\\_SB.PLTF.C000");
    assert_eq!(f.cpu_paths[15], "\\_SB.PLTF.C00F");
    assert_eq!(f.awac_path, None);
    assert_eq!(f.rtc_path.as_deref(), Some("\\_SB.PCI0.SBRG.RTC"));
    assert_eq!(f.smbus_path.as_deref(), Some("\\_SB.PCI0.SMBS"));
    assert_eq!(
        f.xhci_paths,
        vec!["\\_SB.PCI0.GP17.XHC0", "\\_SB.PCI0.GP17.XHC1"]
    );
    assert_eq!(f.igpu_path, None);
}

#[test]
fn facts_intel_laptop() {
    let f = tables(intel_laptop()).facts();
    assert_eq!(f.ec_path.as_deref(), Some("\\_SB.PCI0.LPCB.EC0"));
    assert!(!f.ec_has_sta);
    assert_eq!(f.gpio_path.as_deref(), Some("\\_SB.PCI0.GPI0"));
    assert!(f.has_osi_windows);
    assert_eq!(f.awac_path, None);
    assert_eq!(f.cpu_paths, Vec::<String>::new());
}

#[test]
fn facts_hedt_x99() {
    let t = tables(hedt_x99());
    let f = t.facts();
    assert_eq!(f.pci_root.as_deref(), Some("\\_SB.PCI0"));
    assert_eq!(f.lpc_bridge.as_deref(), Some("\\_SB.PCI0.LPC0"));
    assert_eq!(f.igpu_path, None, "0x00020000 is a PCIe root port on HEDT");
    assert_eq!(f.cpu_paths[0], "\\_SB.SCK0.CP00");
    assert_eq!(
        f.smbus_path.as_deref(),
        Some("\\_SB.PCI0.SBUS"),
        "0x001F0003 with HDA at 0x001B0000"
    );
    assert_eq!(
        f.rhub_paths,
        vec![
            "\\_SB.PCI0.XHCI.RHUB",
            "\\_SB.PCI0.EHC1.HUBN",
            "\\_SB.PCI0.EHC2.HUBN"
        ]
    );
    assert_eq!(f.xhci_paths, vec!["\\_SB.PCI0.XHCI"]);
}

#[test]
fn gpio_controller_found_by_id() {
    // Ryzen laptop: \_SB.GPIO (AMDI0030) next to an LPC0 holding EC0.
    let terms = vec![Term::scope(
        "\\_SB",
        vec![
            dev(
                "PCI0",
                vec![
                    eisa("_HID", "PNP0A08"),
                    dev("LPC0", vec![adr(0x0014_0003), ec_device("EC0", None)]),
                ],
            ),
            dev(
                "GPIO",
                vec![hid("AMDI0030"), Term::name("_CID", Term::str("AMDI0030"))],
            ),
        ],
    )];
    let f = tables(vec![table(b"DSDT", "RENOIR", &terms)]).facts();
    assert_eq!(f.gpio_path.as_deref(), Some("\\_SB.GPIO"));
    assert_eq!(f.lpc_bridge.as_deref(), Some("\\_SB.PCI0.LPC0"));
    assert_eq!(f.ec_path.as_deref(), Some("\\_SB.PCI0.LPC0.EC0"));
}

#[test]
fn osi_strings_found() {
    let t = tables(intel_laptop());
    assert_eq!(
        t.osi_strings(),
        vec![
            "Windows 2009",
            "Windows 2012",
            "Windows 2015",
            "Windows 2017"
        ]
    );
}

#[test]
fn parse_dsdt_and_dir() {
    let images = alder_lake();
    let f = super::parse_dsdt(&images[0]).expect("dsdt");
    assert_eq!(f.lpc_bridge.as_deref(), Some("\\_SB.PC00.LPCB"));
    assert!(
        super::parse_dsdt(&images[1]).is_err(),
        "an SSDT is not a DSDT"
    );

    let dir = std::env::temp_dir().join(format!("oneclick-acpi-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("DSDT.aml"), &images[0]).expect("write");
    std::fs::write(dir.join("SSDT1.aml"), &images[1]).expect("write");
    std::fs::write(dir.join("ssdt1.dat"), &images[1]).expect("write");
    std::fs::write(dir.join("notes.txt"), b"ignored").expect("write");
    std::fs::write(dir.join("SSDT2.aml"), b"SSDT garbage").expect("write");
    let f = super::parse_tables(&dir).expect("parse dir");
    assert!(f.cpu_uses_acpi0007);
    assert_eq!(f.cpu_paths.len(), 4);
    let t = super::load_tables(&dir).expect("load dir");
    assert_eq!(t.tables().len(), 2, "the copy of SSDT1 is dropped");
    std::fs::remove_file(dir.join("DSDT.aml")).expect("remove");
    let err = super::parse_tables(&dir).expect_err("no DSDT");
    assert_eq!(err.code, "ACPI_DSDT_MISSING");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── SSDTs from tables ───────────────────────────────────────────────────────

#[test]
fn ec_desktop_renames_and_disables_real_ec() {
    let images = intel_desktop();
    let t = tables(images.clone());
    let g = ok(SsdtKind::EcUsbx { laptop: false }, &t);
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PCI0.LPCB.EC"), "fake EC under LPCB");
    assert!(
        has(&parsed, "\\_SB.PCI0.LPCB.EC0._STA"),
        "real EC (renamed) disabled"
    );
    assert!(has(&parsed, "\\_SB.USBX._DSM"));
    assert_eq!(g.patches.len(), 2);
    assert_eq!(
        (g.patches[0].find.as_str(), g.patches[0].replace.as_str()),
        ("45435F5F", "4543305F")
    );
    assert_eq!(g.patches[0].table_signature, None);
    let xsta = &g.patches[1];
    assert_eq!(xsta.table_signature.as_deref(), Some("DSDT"));
    assert!(xsta.find.contains("5F535441") && xsta.replace.contains("58535441"));
    // Unique once EC is renamed to EC0.
    let renamed: Vec<Vec<u8>> = images
        .iter()
        .map(|i| {
            let mut d = i.clone();
            for p in find_all(i, b"EC__") {
                d[p..p + 4].copy_from_slice(b"EC0_");
            }
            d
        })
        .collect();
    assert!(unique_in(&renamed, b"DSDT", &xsta.find));
    assert!(g.dsl.contains("Return (\\_SB.PCI0.LPCB.EC0.XSTA)"));
}

#[test]
fn ec_laptop_keeps_real_ec() {
    let t = tables(intel_laptop());
    let g = ok(SsdtKind::EcUsbx { laptop: true }, &t);
    let parsed = check(&g);
    assert!(
        has(&parsed, "\\_SB.PCI0.LPCB.EC"),
        "fake EC since the real one is EC0"
    );
    assert!(!has(&parsed, "\\_SB.PCI0.LPCB.EC0._STA"));
    assert!(g.patches.is_empty());
    assert!(has(&parsed, "\\_SB.USBX"));
}

#[test]
fn ec_without_any_ec_amd() {
    let t = tables(amd_b550());
    let g = ok(SsdtKind::EcUsbx { laptop: false }, &t);
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PCI0.SBRG.EC"));
    assert!(g.patches.is_empty());
}

#[test]
fn plug_variants() {
    let g = ok(SsdtKind::Plug, &tables(intel_desktop()));
    assert_eq!(g.file_name, "SSDT-PLUG.aml");
    let parsed = check(&g);
    assert!(has(&parsed, "\\_PR.CPU0._DSM"));

    let g = ok(SsdtKind::Plug, &tables(alder_lake()));
    assert_eq!(g.file_name, "SSDT-PLUG-ALT.aml");
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.C000._DSM"));
    assert!(has(&parsed, "\\_SB.C003._UID"));
    assert!(!has(&parsed, "\\_SB.C001._DSM"));

    let g = ok(SsdtKind::Plug, &tables(hedt_x99()));
    assert!(g.dsl.contains("External (\\_SB.SCK0.CP00, ProcessorObj)"));

    // Apple firmware already has CPU0._DSM; a second one cannot be added.
    let terms = vec![Term::scope(
        "\\_PR",
        vec![Term::Processor {
            name: "CPU0".into(),
            id: 0,
            pblk: 0x410,
            pblk_len: 6,
            body: vec![Term::method("_DSM", 4, vec![Term::ret(Term::int(0))])],
        }],
    )];
    let err = generate_with_tables(
        &SsdtKind::Plug,
        &tables(vec![table(b"DSDT", "MAC", &terms)]),
    )
    .expect_err("CPU0 has a _DSM");
    assert_eq!(err.code, ERR_NOT_NEEDED);
}

#[test]
fn cpur_for_amd_only_with_acpi0007() {
    let g = ok(SsdtKind::Cpur, &tables(amd_b550()));
    assert_eq!(g.file_name, "SSDT-CPUR.aml");
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.C000"));
    assert!(has(&parsed, "\\_SB.C00F._UID"));
    assert!(!has(&parsed, "\\_SB.C000._DSM"), "no plugin-type on AMD");
    let err = generate_with_tables(&SsdtKind::Cpur, &tables(intel_desktop()))
        .expect_err("Processor objects exist");
    assert_eq!(err.code, ERR_NOT_NEEDED);
}

#[test]
fn awac_uses_stas() {
    let g = ok(SsdtKind::Awac, &tables(intel_desktop()));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_INI"));
    assert!(g.patches.is_empty());
    assert!(g.dsl.contains("\\STAS = One"));
    let err = generate_with_tables(&SsdtKind::Awac, &tables(amd_b550())).expect_err("plain RTC");
    assert_eq!(err.code, ERR_NOT_NEEDED);
}

#[test]
fn awac_without_stas_overrides_sta() {
    // AWAC whose _STA does not use STAS, and no RTC at all.
    let terms = vec![Term::scope(
        "\\_SB",
        vec![dev(
            "PCI0",
            vec![
                eisa("_HID", "PNP0A08"),
                dev(
                    "LPCB",
                    vec![
                        adr(0x001F_0000),
                        dev(
                            "AWAC",
                            vec![
                                hid("ACPI000E"),
                                Term::method("_STA", 0, vec![Term::ret(Term::int(0x0F))]),
                            ],
                        ),
                    ],
                ),
            ],
        )],
    )];
    let images = vec![table(b"DSDT", "AWACONLY", &terms)];
    let g = ok(SsdtKind::Awac, &tables(images.clone()));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PCI0.LPCB.AWAC._STA"));
    assert!(has(&parsed, "\\_SB.PCI0.LPCB.RTC0._HID"), "fake RTC0");
    assert_eq!(g.patches.len(), 1);
    assert!(unique_in(&images, b"DSDT", &g.patches[0].find));
}

/// AWAC switched by STAS next to an RTC with the given `_STA`.
fn awac_board(rtc_sta: Option<Term>, root_ini: bool) -> Vec<Vec<u8>> {
    let mut terms = vec![Term::name("STAS", Term::int(0))];
    if root_ini {
        terms.push(Term::method("_INI", 0, vec![]));
    }
    terms.push(Term::scope(
        "\\_SB",
        vec![dev(
            "PC00",
            vec![
                eisa("_HID", "PNP0A08"),
                dev(
                    "LPCB",
                    vec![
                        adr(0x001F_0000),
                        dev("AWAC", vec![hid("ACPI000E"), sta_on("STAS", 0, 0x0F, 0)]),
                        rtc_device("RTC", rtc_sta),
                    ],
                ),
            ],
        )],
    ));
    vec![table(b"DSDT", "AWACRTC", &terms)]
}

#[test]
fn awac_leaves_an_always_present_rtc_alone() {
    let always = Term::method("_STA", 0, vec![Term::ret(Term::int(0x0F))]);
    let g = ok(SsdtKind::Awac, &tables(awac_board(Some(always), false)));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_INI"));
    assert!(g.patches.is_empty(), "no _STA rename for the RTC");
    assert!(!has(&parsed, "\\_SB.PC00.LPCB.RTC._STA"));

    // An RTC whose _STA reports it absent is forced on in macOS.
    let off = Term::method("_STA", 0, vec![Term::ret(Term::int(0))]);
    let g = ok(SsdtKind::Awac, &tables(awac_board(Some(off), false)));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PC00.LPCB.RTC._STA"));
    assert_eq!(g.patches.len(), 1);
}

#[test]
fn awac_avoids_a_second_root_ini() {
    let g = ok(SsdtKind::Awac, &tables(awac_board(None, true)));
    let parsed = check(&g);
    assert!(!has(&parsed, "\\_INI"), "module-level code instead");
    assert!(g.dsl.contains("Scope (\\_SB)") && g.dsl.contains("\\STAS = One"));

    // From facts alone a root _INI cannot be ruled out.
    let f = tables(awac_board(None, false)).facts();
    let g = ok_facts(SsdtKind::Awac, &f);
    let parsed = check(&g);
    assert!(!has(&parsed, "\\_INI"));
    assert!(g.dsl.contains("\\STAS = One"));
}

#[test]
fn rtc_range_disables_the_original_rtc_unless_awac_mode_does() {
    // AWAC board, RTC without _STA: the old RTC would stay enabled.
    let g = ok(SsdtKind::Rtc0Range, &tables(awac_board(None, false)));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PC00.LPCB.RTC._STA"));
    assert!(has(&parsed, "\\_SB.PC00.LPCB.RTC0._CRS"));
    assert!(g.patches.is_empty());
    assert!(g.dsl.contains("instead of SSDT-AWAC"));

    // AWAC board, RTC _STA follows STAS: AWAC mode already turns it off.
    let g = ok(
        SsdtKind::Rtc0Range,
        &tables(awac_board(Some(sta_on("STAS", 1, 0x0F, 0)), false)),
    );
    let parsed = check(&g);
    assert!(!has(&parsed, "\\_SB.PC00.LPCB.RTC._STA"));
    assert!(g.patches.is_empty());

    // AWAC board, RTC _STA that ignores STAS: renamed and disabled.
    let always = Term::method("_STA", 0, vec![Term::ret(Term::int(0x0F))]);
    let images = awac_board(Some(always), false);
    let g = ok(SsdtKind::Rtc0Range, &tables(images.clone()));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PC00.LPCB.RTC._STA"));
    assert_eq!(g.patches.len(), 1);
    assert!(unique_in(&images, b"DSDT", &g.patches[0].find));

    // From facts, the RTC is disabled only when it has no _STA of its own.
    let f = tables(awac_board(None, false)).facts();
    let g = ok_facts(SsdtKind::Rtc0Range, &f);
    check(&g);
    assert!(g.dsl.contains("If (!CondRefOf (\\_SB.PC00.LPCB.RTC._STA))"));
}

#[test]
fn pmc_under_lpc() {
    let g = ok(SsdtKind::Pmc, &tables(intel_desktop()));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PCI0.LPCB.PMCR._CRS"));
    let g = ok(SsdtKind::Pmc, &tables(hedt_x99()));
    assert!(has(&check(&g), "\\_SB.PCI0.LPC0.PMCR"));
}

#[test]
fn rhub_reset_with_illegal_names() {
    let g = ok(SsdtKind::RhubReset, &tables(amd_b550()));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PCI0.GP17.XHC0.RHUB._STA"));
    assert!(has(&parsed, "\\_SB.PCI0.GP17.XHC1._STA"), "XHC1 disabled");
    assert!(
        has(&parsed, "\\_SB.PCI0.GP17.XHC2._ADR"),
        "replacement controller"
    );
    assert!(g.dsl.contains("Name (_ADR, 0x04)"));

    let g = ok(SsdtKind::RhubReset, &tables(hedt_x99()));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PCI0.XHCI.RHUB._STA"));
    assert!(has(&parsed, "\\_SB.PCI0.EH01"));
    assert!(has(&parsed, "\\_SB.PCI0.EH02"));
    assert!(has(&parsed, "\\_SB.PCI0.EHC1._STA"));
}

#[test]
fn xosi_with_detected_strings_and_osid() {
    let g = ok(SsdtKind::Xosi, &tables(intel_laptop()));
    let parsed = check(&g);
    assert!(has(&parsed, "\\XOSI"));
    assert!(g.dsl.contains("\"Windows 2017\""));
    assert!(
        !g.dsl.contains("\"Windows 2018\""),
        "stops at the newest string the firmware checks"
    );
    let finds: Vec<&str> = g.patches.iter().map(|p| p.find.as_str()).collect();
    assert_eq!(finds, vec!["4F534944", "5F4F5349"], "OSID before _OSI");
}

#[test]
fn gpi0_override() {
    let images = intel_laptop();
    let g = ok(SsdtKind::Gpi0, &tables(images.clone()));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PCI0.GPI0._STA"));
    assert_eq!(g.patches.len(), 1);
    assert!(unique_in(&images, b"DSDT", &g.patches[0].find));
    let err = generate_with_tables(&SsdtKind::Gpi0, &tables(intel_desktop())).expect_err("no GPIO");
    assert_eq!(err.code, ERR_NOT_NEEDED);
}

#[test]
fn pnlf_and_nbcf() {
    let g = ok(SsdtKind::Pnlf { uid: 16 }, &tables(intel_laptop()));
    let parsed = check(&g);
    assert!(has(&parsed, "\\PNLF._UID"));
    assert!(g.dsl.contains("Name (_UID, 0x10)"));
    assert!(g
        .patches
        .iter()
        .any(|p| p.find == "084E42434600" && !p.enabled));
    assert!(!g.patches.iter().any(|p| p.find == "504E4C46"));
}

#[test]
fn unc_and_rtc_range_for_hedt() {
    let t = tables(hedt_x99());
    let g = ok(SsdtKind::Unc, &t);
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.UNC0._INI"));
    assert!(g.dsl.contains("\\PRBM = Zero"));

    let g = ok(SsdtKind::Rtc0Range, &t);
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PCI0.LPC0.RTC0._CRS"));
    assert!(
        has(&parsed, "\\_SB.PCI0.LPC0.RTC._STA"),
        "original RTC disabled (no AWAC)"
    );

    let err = generate_with_tables(&SsdtKind::Unc, &tables(intel_desktop())).expect_err("no UNC0");
    assert_eq!(err.code, ERR_NOT_NEEDED);
}

fn rtc_board(rtc: Term) -> Vec<Vec<u8>> {
    let terms = vec![Term::scope(
        "\\_SB",
        vec![dev(
            "PC00",
            vec![
                eisa("_HID", "PNP0A08"),
                dev("LPC0", vec![adr(0x001F_0000), rtc]),
            ],
        )],
    )];
    vec![table(b"DSDT", "X299", &terms)]
}

#[test]
fn rtc_range_follows_the_firmware_ranges() {
    // 0x70/2 + 0x74/4 + 0x78/2 with IRQ (Edge...) {8}: both gaps closed, the IRQ kept.
    let rtc = dev(
        "RTC",
        vec![
            eisa("_HID", "PNP0B00"),
            Term::name(
                "_CRS",
                Term::Resources(vec![
                    Resource::Io {
                        min: 0x70,
                        max: 0x70,
                        align: 1,
                        len: 2,
                    },
                    Resource::Io {
                        min: 0x74,
                        max: 0x74,
                        align: 1,
                        len: 2,
                    },
                    Resource::Io {
                        min: 0x78,
                        max: 0x78,
                        align: 1,
                        len: 2,
                    },
                    Resource::Irq {
                        mask: 1 << 8,
                        flags: 0x01,
                    },
                ]),
            ),
        ],
    );
    let g = ok(SsdtKind::Rtc0Range, &tables(rtc_board(rtc)));
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PC00.LPC0.RTC0._CRS"));
    assert!(has(&parsed, "\\_SB.PC00.LPC0.RTC._STA"));
    let crs = Term::Resources(vec![
        Resource::Io {
            min: 0x70,
            max: 0x70,
            align: 1,
            len: 4,
        },
        Resource::Io {
            min: 0x74,
            max: 0x74,
            align: 1,
            len: 4,
        },
        Resource::Io {
            min: 0x78,
            max: 0x78,
            align: 1,
            len: 2,
        },
        Resource::Irq {
            mask: 1 << 8,
            flags: 0x01,
        },
    ]);
    let mut expected = Vec::new();
    crs.encode(&mut expected);
    assert!(
        !find_all(&g.aml, &expected).is_empty(),
        "closed ranges and the original IRQ"
    );
    assert!(g.dsl.contains("IRQ (Edge, ActiveHigh, Exclusive, )"));

    // A contiguous range needs nothing.
    let rtc = dev(
        "RTC",
        vec![
            eisa("_HID", "PNP0B00"),
            Term::name(
                "_CRS",
                Term::Resources(vec![
                    Resource::Io {
                        min: 0x70,
                        max: 0x70,
                        align: 1,
                        len: 8,
                    },
                    Resource::IrqNoFlags { mask: 1 << 8 },
                ]),
            ),
        ],
    );
    let err =
        generate_with_tables(&SsdtKind::Rtc0Range, &tables(rtc_board(rtc))).expect_err("no gap");
    assert_eq!(err.code, ERR_NOT_NEEDED);

    // A _CRS method cannot be checked: the OpenCore sample ranges are used.
    let rtc = dev(
        "RTC",
        vec![
            eisa("_HID", "PNP0B00"),
            Term::method(
                "_CRS",
                0,
                vec![Term::ret(Term::Resources(vec![Resource::IrqNoFlags {
                    mask: 1 << 8,
                }]))],
            ),
        ],
    );
    let g = ok(SsdtKind::Rtc0Range, &tables(rtc_board(rtc)));
    check(&g);
    assert!(g.dsl.contains("0x0074,             // Range Minimum"));
}

#[test]
fn xosi_detects_strings_passed_through_helpers() {
    let terms = vec![
        Term::method(
            "OSIF",
            1,
            vec![Term::ret(Term::call("_OSI", vec![Term::Arg(0)]))],
        ),
        Term::scope(
            "\\_SB",
            vec![Term::method(
                "_INI",
                0,
                vec![
                    Term::if_then(Term::call("OSIF", vec![Term::str("Windows 2019")]), vec![]),
                    Term::if_then(
                        Term::call("OSIF", vec![Term::str("Windows 2001 SP2")]),
                        vec![],
                    ),
                    Term::if_then(Term::call("OSIF", vec![Term::str("Windows NT")]), vec![]),
                    Term::if_then(Term::call("OSIF", vec![Term::str("Windows 20")]), vec![]),
                    Term::if_then(
                        Term::call("OSIF", vec![Term::str("Windows 2015\u{7f}")]),
                        vec![],
                    ),
                ],
            )],
        ),
    ];
    let t = tables(vec![table(b"DSDT", "OSIF", &terms)]);
    assert_eq!(t.osi_strings(), vec!["Windows 2019", "Windows 2001 SP2"]);
    assert!(t.facts().has_osi_windows);
    let g = ok(SsdtKind::Xosi, &t);
    assert!(g.dsl.contains("\"Windows 2019\"") && !g.dsl.contains("\"Windows 2020\""));
    let finds: Vec<&str> = g.patches.iter().map(|p| p.find.as_str()).collect();
    assert_eq!(
        finds,
        vec!["5F4F5349"],
        "OSIF is never preceded by a NameSeg ending in _"
    );

    // \_SB.PCI0.EC0_.OSIS: "EC0_OSIS" contains a false "_OSI".
    let terms = vec![
        Term::scope(
            "\\_SB",
            vec![dev(
                "EC0",
                vec![Term::method("OSIS", 0, vec![Term::ret(Term::int(1))])],
            )],
        ),
        Term::method(
            "TEST",
            0,
            vec![Term::ret(Term::call("\\_SB.EC0.OSIS", vec![]))],
        ),
    ];
    let t = tables(vec![table(b"DSDT", "OSIS", &terms)]);
    let g = ok(SsdtKind::Xosi, &t);
    let finds: Vec<&str> = g.patches.iter().map(|p| p.find.as_str()).collect();
    assert_eq!(finds, vec!["4F534953", "5F4F5349"], "OSIS before _OSI");
    assert_eq!(g.patches[0].replace, "58534953");

    // XSIS is taken, so OSIS becomes YSIS.
    let mut taken = terms.clone();
    taken.push(Term::name("XSIS", Term::int(0)));
    let g = ok(
        SsdtKind::Xosi,
        &tables(vec![table(b"DSDT", "OSIS", &taken)]),
    );
    assert_eq!(
        (g.patches[0].find.as_str(), g.patches[0].replace.as_str()),
        ("4F534953", "59534953")
    );
}

#[test]
fn ec_laptop_not_needed_when_ec_and_usbx_exist() {
    let terms = vec![Term::scope(
        "\\_SB",
        vec![
            dev(
                "PCI0",
                vec![
                    eisa("_HID", "PNP0A08"),
                    dev("LPCB", vec![adr(0x001F_0000), ec_device("EC", None)]),
                ],
            ),
            dev("USBX", vec![adr(0)]),
        ],
    )];
    let t = tables(vec![table(b"DSDT", "MAC", &terms)]);
    let err =
        generate_with_tables(&SsdtKind::EcUsbx { laptop: true }, &t).expect_err("nothing to add");
    assert_eq!(err.code, ERR_NOT_NEEDED);
    // A desktop still renames and disables it.
    let g = ok(SsdtKind::EcUsbx { laptop: false }, &t);
    assert!(has(&check(&g), "\\_SB.PCI0.LPCB.EC0._STA"));
}

#[test]
fn imei_sbus_als() {
    let t = tables(intel_desktop());
    let g = ok(SsdtKind::Imei, &t);
    assert!(has(&check(&g), "\\_SB.PCI0.IMEI._ADR"));

    let g = ok(SsdtKind::SbusMchc, &t);
    let parsed = check(&g);
    assert!(has(&parsed, "\\_SB.PCI0.MCHC"));
    assert!(has(&parsed, "\\_SB.PCI0.SBUS.BUS0._CID"));

    let g = ok(SsdtKind::Als0, &t);
    assert!(has(&check(&g), "\\_SB.ALS0._ALR"));
}

// ── SSDTs from facts only ───────────────────────────────────────────────────

#[test]
fn every_kind_from_facts() {
    for images in [
        intel_desktop(),
        alder_lake(),
        amd_b550(),
        intel_laptop(),
        hedt_x99(),
    ] {
        let f = tables(images).facts();
        for kind in &ALL_KINDS {
            match generate(kind, &f) {
                Ok(g) => {
                    check(&g);
                }
                Err(e) => assert!(
                    e.code == ERR_NOT_NEEDED || e.code == ERR_INSUFFICIENT,
                    "{kind:?}: unexpected error {e}"
                ),
            }
        }
    }
}

#[test]
fn facts_only_desktop_ec_avoids_renames() {
    let f = tables(intel_desktop()).facts();
    let g = ok_facts(SsdtKind::EcUsbx { laptop: false }, &f);
    let parsed = check(&g);
    assert!(g.patches.is_empty(), "no EC0 rename without the tables");
    assert!(
        has(&parsed, "\\_SB.EC"),
        "fake EC beside the real EC named EC"
    );
    assert!(g.dsl.contains("stays enabled"));
}

#[test]
fn facts_only_guards() {
    let f = tables(intel_laptop()).facts();
    let g = ok_facts(SsdtKind::Gpi0, &f);
    check(&g);
    assert!(g.dsl.contains("If (CondRefOf (\\GPEN))"));
    assert!(g.patches.is_empty());

    let g = ok_facts(SsdtKind::RhubReset, &f);
    assert!(g.dsl.contains("If (!CondRefOf (\\_SB.PCI0.XHC.RHUB._STA))"));

    let g = ok_facts(SsdtKind::Xosi, &f);
    assert!(g.dsl.contains("\"Windows 2022\""));
    assert_eq!(g.patches.len(), 1);

    let g = ok_facts(SsdtKind::EcUsbx { laptop: true }, &f);
    check(&g);
    assert!(
        g.dsl.contains("If (!CondRefOf (\\_SB.PCI0.LPCB.EC))"),
        "fake EC only when none exists"
    );

    let f = tables(alder_lake()).facts();
    let g = ok_facts(SsdtKind::Plug, &f);
    assert_eq!(g.file_name, "SSDT-PLUG-ALT.aml");
    let g = ok_facts(SsdtKind::Awac, &f);
    assert!(g.dsl.contains("\\STAS = One"));
}

#[test]
fn bad_facts_are_rejected_not_panicking() {
    let f = AcpiFacts {
        lpc_bridge: Some("\\_SB.PCI0.LP\"C".into()),
        pci_root: Some("garbage path".into()),
        cpu_paths: vec!["\\_PR.CPU0.TOOLONGNAME".into()],
        rhub_paths: vec!["".into()],
        smbus_path: Some("\\".into()),
        ..AcpiFacts::default()
    };
    for kind in [
        SsdtKind::Pmc,
        SsdtKind::Plug,
        SsdtKind::RhubReset,
        SsdtKind::SbusMchc,
        SsdtKind::Imei,
    ] {
        let err = generate(&kind, &f).expect_err("rejected");
        assert_eq!(err.code, ERR_INSUFFICIENT, "{kind:?}");
    }
    // Always buildable: EC (fake under \_SB), XOSI, PNLF, ALS0.
    let g = ok_facts(SsdtKind::EcUsbx { laptop: false }, &f);
    assert!(has(&check(&g), "\\_SB.EC"));
}

/// Writes every SSDT (AML + DSL) for the synthetic boards into
/// `$ACPI_DUMP_DIR` for checking with iasl.
#[test]
#[ignore]
fn dump_generated_tables() {
    let Ok(dir) = std::env::var("ACPI_DUMP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("dump dir");
    let boards: [(&str, Vec<Vec<u8>>); 5] = [
        ("desktop", intel_desktop()),
        ("adl", alder_lake()),
        ("b550", amd_b550()),
        ("laptop", intel_laptop()),
        ("x99", hedt_x99()),
    ];
    for (board, images) in boards {
        for (i, img) in images.iter().enumerate() {
            std::fs::write(dir.join(format!("{board}-input{i}.aml")), img).expect("write");
        }
        let t = tables(images);
        let f = t.facts();
        for kind in &ALL_KINDS {
            for (mode, result) in [
                ("tables", generate_with_tables(kind, &t)),
                ("facts", generate(kind, &f)),
            ] {
                if let Ok(g) = result {
                    let variant = if *kind == (SsdtKind::EcUsbx { laptop: true }) {
                        "-LAPTOP"
                    } else {
                        ""
                    };
                    let stem = format!(
                        "{board}-{mode}-{}{variant}",
                        g.file_name.trim_end_matches(".aml")
                    );
                    std::fs::write(dir.join(format!("{stem}.aml")), &g.aml).expect("write");
                    std::fs::write(dir.join(format!("{stem}.dsl")), &g.dsl).expect("write");
                }
            }
        }
    }
}

/// Reads every machine directory in `$ACPI_REAL_DIR` (one dumped table set
/// per directory), writes the recovered namespace and facts to
/// `$ACPI_REAL_OUT/<machine>.ours.json` and every SSDT kind into
/// `$ACPI_REAL_OUT/ssdt/` for comparison with iasl.
#[test]
#[ignore]
fn real_dumps() {
    use super::namespace::NsKind;
    let (Ok(dir), Ok(out)) = (
        std::env::var("ACPI_REAL_DIR"),
        std::env::var("ACPI_REAL_OUT"),
    ) else {
        return;
    };
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(out.join("ssdt")).expect("out dir");
    let mut machines: Vec<_> = std::fs::read_dir(&dir)
        .expect("dir")
        .flatten()
        .map(|e| e.path())
        .collect();
    machines.sort();
    for m in machines {
        let name = m
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let t = match super::load_tables(&m) {
            Ok(t) => t,
            Err(e) => {
                println!("{name}: {e}");
                continue;
            }
        };
        let mut objs = Vec::new();
        for o in &t.ns().objects {
            let kind = match &o.kind {
                NsKind::Device => "Device",
                NsKind::Processor { .. } => "Processor",
                NsKind::Method { .. } => "Method",
                NsKind::ThermalZone => "ThermalZone",
                NsKind::PowerResource => "PowerResource",
                NsKind::Name(_) => "Name",
                _ => continue,
            };
            let segs: Vec<String> = o
                .path
                .iter()
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect();
            objs.push(serde_json::json!([
                t.table(o.table).map(|x| x.signature.clone()),
                kind,
                segs.join("."),
                o.heuristic
            ]));
        }
        let facts = t.facts();
        let doc = serde_json::json!({ "objects": objs, "facts": facts });
        std::fs::write(
            out.join(format!("{name}.ours.json")),
            serde_json::to_vec_pretty(&doc).expect("json"),
        )
        .expect("write");
        let heuristic = t.ns().objects.iter().filter(|o| o.heuristic).count();
        println!(
            "{name}: {} objects, {heuristic} heuristic",
            t.ns().objects.len()
        );
        for kind in &ALL_KINDS {
            for (mode, result) in [
                ("tables", generate_with_tables(kind, &t)),
                ("facts", generate(kind, &facts)),
            ] {
                match result {
                    Ok(g) => {
                        let variant = if *kind == (SsdtKind::EcUsbx { laptop: true }) {
                            "-LAPTOP"
                        } else {
                            ""
                        };
                        let stem = format!(
                            "{name}-{mode}-{}{variant}",
                            g.file_name.trim_end_matches(".aml")
                        );
                        std::fs::write(out.join("ssdt").join(format!("{stem}.aml")), &g.aml)
                            .expect("write");
                        std::fs::write(out.join("ssdt").join(format!("{stem}.dsl")), &g.dsl)
                            .expect("write");
                        let patches: Vec<_> = g
                            .patches
                            .iter()
                            .map(|p| {
                                (
                                    p.comment.clone(),
                                    p.find.clone(),
                                    p.replace.clone(),
                                    p.table_signature.clone(),
                                    p.oem_table_id.clone(),
                                    p.enabled,
                                )
                            })
                            .collect();
                        std::fs::write(
                            out.join("ssdt").join(format!("{stem}.patches.json")),
                            serde_json::to_vec_pretty(&patches).expect("json"),
                        )
                        .expect("write");
                    }
                    Err(e) => println!("   {mode} {kind:?}: {} {}", e.code, e.message),
                }
            }
        }
    }
}

#[test]
fn corrupted_tables_never_panic() {
    for images in [intel_desktop(), amd_b550(), intel_laptop(), hedt_x99()] {
        let image = &images[0];
        for pos in (aml::HEADER_LEN..image.len()).step_by(23) {
            for value in [0x00, 0xFF, 0x5B] {
                let mut bad = image.clone();
                bad[pos] = value;
                let t = AcpiTables::new(vec![AcpiTable::parse(bad).expect("header intact")]);
                let facts = t.facts();
                for kind in &ALL_KINDS {
                    if let Ok(g) = generate_with_tables(kind, &t) {
                        assert_eq!(aml::byte_sum(&g.aml), 0);
                    }
                    let _ = generate(kind, &facts);
                }
            }
        }
    }
}

#[test]
fn patch_comments_name_their_tables() {
    let mut c = String::from("EC0 _STA to XSTA rename");
    super::name_required_table(&mut c, "SSDT-EC.aml");
    assert_eq!(c, "EC0 _STA to XSTA rename (SSDT-EC.aml)");
    super::name_required_table(&mut c, "SSDT-EC.aml");
    super::name_required_table(&mut c, "SSDT-USBX.aml");
    assert_eq!(c, "EC0 _STA to XSTA rename (SSDT-EC.aml, SSDT-USBX.aml)");
    assert_eq!(super::tables_named_in(&c), ["SSDT-EC.aml", "SSDT-USBX.aml"]);

    let mut xosi = String::from("_OSI to XOSI rename - requires SSDT-XOSI.aml");
    super::name_required_table(&mut xosi, "SSDT-XOSI.aml");
    assert_eq!(xosi, "_OSI to XOSI rename - requires SSDT-XOSI.aml");
    super::name_required_table(&mut xosi, "SSDT-GPI0.aml");
    assert_eq!(xosi, "_OSI to XOSI rename - requires SSDT-XOSI.aml (SSDT-GPI0.aml)");

    let mut text = String::from("Rename (to make room)");
    super::name_required_table(&mut text, "SSDT-AWAC.aml");
    assert_eq!(text, "Rename (to make room) (SSDT-AWAC.aml)");
    assert!(super::tables_named_in("no table here").is_empty());
}
