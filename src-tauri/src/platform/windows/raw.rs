//! Decoders for binary data returned by Win32 APIs (processor topology
//! records, firmware table lists, registry numbers, USB hub IOCTL buffers).
//! No Win32 calls here, so the layouts are unit-tested on every host.

use crate::contracts::UsbPortInfo;

// ─── Processor topology ─────────────────────────────────────────────────────

/// Counts from `GetLogicalProcessorInformationEx(RelationAll)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessorTopology {
    /// Physical cores (RelationProcessorCore records).
    pub cores: u32,
    /// Packages / sockets (RelationProcessorPackage records).
    pub packages: u32,
    /// Logical processors (bits set in the core group masks).
    pub threads: u32,
    /// Distinct core efficiency classes (2 on Intel hybrid P/E designs).
    pub efficiency_classes: u32,
}

const RELATION_PROCESSOR_CORE: u32 = 0;
const RELATION_PROCESSOR_PACKAGE: u32 = 3;
/// `PROCESSOR_RELATIONSHIP` starts after the 8-byte record header; its
/// `GroupCount` sits at offset 22 and the 16-byte `GROUP_AFFINITY` entries
/// start, 8-byte aligned, at offset 24.
const CORE_EFFICIENCY_OFFSET: usize = 9;
const CORE_GROUP_COUNT_OFFSET: usize = 30;
const CORE_GROUP_MASKS_OFFSET: usize = 32;
const GROUP_AFFINITY_SIZE: usize = 16;

fn u16_at(buf: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        buf.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(buf: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        buf.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn u64_at(buf: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        buf.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

/// Walk a buffer of variable-size `SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX`
/// records. Truncated or malformed records end the walk.
pub fn parse_processor_records(buf: &[u8]) -> ProcessorTopology {
    let mut topology = ProcessorTopology::default();
    let mut classes: Vec<u8> = Vec::new();
    let mut offset = 0usize;
    while let (Some(relationship), Some(size)) = (u32_at(buf, offset), u32_at(buf, offset + 4)) {
        let size = size as usize;
        if size < 8 || offset + size > buf.len() {
            break;
        }
        let record = &buf[offset..offset + size];
        match relationship {
            RELATION_PROCESSOR_CORE => {
                topology.cores += 1;
                if let Some(class) = record.get(CORE_EFFICIENCY_OFFSET) {
                    if !classes.contains(class) {
                        classes.push(*class);
                    }
                }
                let groups = u16_at(record, CORE_GROUP_COUNT_OFFSET).unwrap_or(0) as usize;
                for group in 0..groups {
                    if let Some(mask) = u64_at(
                        record,
                        CORE_GROUP_MASKS_OFFSET + group * GROUP_AFFINITY_SIZE,
                    ) {
                        topology.threads += mask.count_ones();
                    }
                }
            }
            RELATION_PROCESSOR_PACKAGE => topology.packages += 1,
            _ => {}
        }
        offset += size;
    }
    topology.efficiency_classes = classes.len() as u32;
    topology
}

// ─── ACPI firmware tables ───────────────────────────────────────────────────

/// `GetSystemFirmwareTable('ACPI', id)` wants the signature bytes read as a
/// little-endian DWORD ("DSDT" → 'TDSD').
pub fn acpi_table_id(signature: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*signature)
}

/// Signatures from an `EnumSystemFirmwareTables('ACPI')` buffer, in order and
/// without duplicates (multiple SSDTs are listed once each).
pub fn table_signatures(buf: &[u8]) -> Vec<[u8; 4]> {
    let mut out: Vec<[u8; 4]> = Vec::new();
    for chunk in buf.as_chunks::<4>().0 {
        let signature = [chunk[0], chunk[1], chunk[2], chunk[3]];
        let printable = signature
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_');
        if printable && !out.contains(&signature) {
            out.push(signature);
        }
    }
    out
}

/// Registry key under `HKLM\HARDWARE\ACPI` that holds the `instance`-th SSDT:
/// SSDT, SSD1 … SSD9, SSDA … SSDS (as ACPICA's acpidump reads them).
pub fn ssdt_registry_key(instance: u32) -> Option<String> {
    match instance {
        0 => Some("SSDT".to_string()),
        1..=9 => Some(format!("SSD{instance}")),
        10..=28 => char::from_u32(u32::from(b'A') + instance - 10).map(|c| format!("SSD{c}")),
        _ => None,
    }
}

// ─── Registry values ────────────────────────────────────────────────────────

/// REG_DWORD / REG_QWORD / 4- or 8-byte REG_BINARY data as a number
/// (display adapters store `HardwareInformation.MemorySize` in all of these).
pub fn registry_number(data: &[u8]) -> Option<u64> {
    match data.len() {
        4 => u32_at(data, 0).map(u64::from),
        8 => u64_at(data, 0),
        _ => None,
    }
}

/// UTF-16 text up to the first NUL.
pub fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

// ─── USB hub IOCTLs ─────────────────────────────────────────────────────────

/// `USB_NODE_INFORMATION` (packed): NodeType (4 bytes), then the hub
/// descriptor whose third byte is `bNumberOfPorts`.
pub fn hub_port_count(node_information: &[u8]) -> Option<u32> {
    node_information
        .get(6)
        .map(|n| u32::from(*n))
        .filter(|n| *n > 0)
}

/// Input buffer for IOCTL_USB_GET_PORT_CONNECTOR_PROPERTIES.
pub fn connector_properties_request(port: u32, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len.max(18)];
    buf[..4].copy_from_slice(&port.to_le_bytes());
    buf
}

/// `USB_PORT_CONNECTOR_PROPERTIES` → (UsbPortProperties, CompanionPortNumber).
pub fn decode_connector_properties(buf: &[u8]) -> Option<(u32, u16)> {
    Some((u32_at(buf, 8)?, u16_at(buf, 14)?))
}

/// Input buffer for IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2: the
/// caller announces the protocols it understands (USB 1.1, 2.0 and 3.x).
pub fn connection_v2_request(port: u32) -> [u8; 16] {
    let mut buf = [0u8; 16];
    buf[..4].copy_from_slice(&port.to_le_bytes());
    buf[4..8].copy_from_slice(&16u32.to_le_bytes());
    buf[8..12].copy_from_slice(&0b111u32.to_le_bytes());
    buf
}

/// `USB_NODE_CONNECTION_INFORMATION_EX_V2.SupportedUsbProtocols`.
pub fn decode_connection_v2(buf: &[u8]) -> Option<u32> {
    u32_at(buf, 8)
}

const PORT_USER_CONNECTABLE: u32 = 1 << 0;
const PORT_TYPE_C: u32 = 1 << 3;
const PROTOCOL_USB300: u32 = 1 << 2;

/// One root hub port from the two IOCTL answers (either may be missing).
/// Connector guesses follow the AppleUSB `UsbConnector` values USB maps use:
/// 0 USB 2 Type-A, 3 USB 3 Type-A, 9 Type-C, 255 internal.
pub fn usb_port_info(
    index: u32,
    connector: Option<(u32, u16)>,
    protocols: Option<u32>,
) -> UsbPortInfo {
    let usb3 = protocols.is_some_and(|p| p & PROTOCOL_USB300 != 0);
    let companion = connector.map(|(_, c)| u32::from(c)).filter(|c| *c > 0);
    let user_connectable = connector.map(|(props, _)| props & PORT_USER_CONNECTABLE != 0);
    let connector_type = connector.map(|(props, _)| {
        if props & PORT_USER_CONNECTABLE == 0 {
            255
        } else if props & PORT_TYPE_C != 0 {
            9
        } else if usb3 || companion.is_some() {
            3
        } else {
            0
        }
    });
    UsbPortInfo {
        index,
        name: None,
        speed_class: if usb3 { "usb3" } else { "usb2" }.to_string(),
        connector: connector_type,
        user_connectable,
        companion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_record(efficiency: u8, masks: &[u64]) -> Vec<u8> {
        let size = CORE_GROUP_MASKS_OFFSET + masks.len() * GROUP_AFFINITY_SIZE;
        let mut r = vec![0u8; size];
        r[..4].copy_from_slice(&RELATION_PROCESSOR_CORE.to_le_bytes());
        r[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        r[8] = 1; // LTP_PC_SMT
        r[CORE_EFFICIENCY_OFFSET] = efficiency;
        r[CORE_GROUP_COUNT_OFFSET..CORE_GROUP_COUNT_OFFSET + 2]
            .copy_from_slice(&(masks.len() as u16).to_le_bytes());
        for (i, mask) in masks.iter().enumerate() {
            let at = CORE_GROUP_MASKS_OFFSET + i * GROUP_AFFINITY_SIZE;
            r[at..at + 8].copy_from_slice(&mask.to_le_bytes());
        }
        r
    }

    fn other_record(relationship: u32, size: usize) -> Vec<u8> {
        let mut r = vec![0u8; size];
        r[..4].copy_from_slice(&relationship.to_le_bytes());
        r[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        r
    }

    #[test]
    fn hybrid_topology() {
        // i5-12600K-like: 6 P-cores with SMT (class 1) + 4 E-cores (class 0).
        let mut buf = Vec::new();
        for core in 0..6u32 {
            buf.extend(core_record(1, &[0b11 << (core * 2)]));
        }
        for core in 0..4u32 {
            buf.extend(core_record(0, &[1 << (12 + core)]));
        }
        buf.extend(other_record(RELATION_PROCESSOR_PACKAGE, 80));
        buf.extend(other_record(2, 56)); // cache
        let t = parse_processor_records(&buf);
        assert_eq!(
            t,
            ProcessorTopology {
                cores: 10,
                packages: 1,
                threads: 16,
                efficiency_classes: 2
            }
        );
    }

    #[test]
    fn truncated_records_stop_the_walk() {
        let mut buf = core_record(0, &[0b11]);
        buf.extend(core_record(0, &[0b1100]));
        buf.truncate(buf.len() - 4);
        let t = parse_processor_records(&buf);
        assert_eq!((t.cores, t.threads), (1, 2));
        assert_eq!(parse_processor_records(&[]), ProcessorTopology::default());
        assert_eq!(
            parse_processor_records(&[0, 0, 0, 0, 0, 0, 0, 0]),
            ProcessorTopology::default()
        );
    }

    #[test]
    fn firmware_table_ids() {
        // ACPICA requests the DSDT as 'TDSD' (a C multi-character constant).
        let tdsd = (u32::from(b'T') << 24)
            | (u32::from(b'D') << 16)
            | (u32::from(b'S') << 8)
            | u32::from(b'D');
        assert_eq!(acpi_table_id(b"DSDT"), tdsd);
        let mut list = Vec::new();
        for sig in [b"FACP", b"APIC", b"SSDT", b"SSDT", b"MCFG"] {
            list.extend_from_slice(&acpi_table_id(sig).to_le_bytes());
        }
        list.extend_from_slice(&[0xff, 0, 1]);
        assert_eq!(
            table_signatures(&list),
            [*b"FACP", *b"APIC", *b"SSDT", *b"MCFG"]
        );
        assert_eq!(ssdt_registry_key(0).as_deref(), Some("SSDT"));
        assert_eq!(ssdt_registry_key(7).as_deref(), Some("SSD7"));
        assert_eq!(ssdt_registry_key(10).as_deref(), Some("SSDA"));
        assert_eq!(ssdt_registry_key(28).as_deref(), Some("SSDS"));
        assert_eq!(ssdt_registry_key(29), None);
    }

    #[test]
    fn registry_numbers_and_text() {
        assert_eq!(
            registry_number(&0x2000_0000u32.to_le_bytes()),
            Some(512 << 20)
        );
        assert_eq!(registry_number(&(8u64 << 30).to_le_bytes()), Some(8 << 30));
        assert_eq!(registry_number(&[1, 2, 3]), None);
        let wide: Vec<u16> = "\\\\?\\usb#root_hub30\0junk".encode_utf16().collect();
        assert_eq!(wide_to_string(&wide), "\\\\?\\usb#root_hub30");
    }

    #[test]
    fn usb_hub_buffers() {
        let mut node = vec![0u8; 76];
        node[6] = 26;
        assert_eq!(hub_port_count(&node), Some(26));
        assert_eq!(hub_port_count(&node[..4]), None);

        let request = connector_properties_request(5, 0);
        assert_eq!(request.len(), 18);
        assert_eq!(&request[..4], &5u32.to_le_bytes());
        let mut answer = request.clone();
        answer[8..12].copy_from_slice(&(PORT_USER_CONNECTABLE | PORT_TYPE_C).to_le_bytes());
        answer[14..16].copy_from_slice(&21u16.to_le_bytes());
        assert_eq!(decode_connector_properties(&answer), Some((0b1001, 21)));

        let v2 = connection_v2_request(3);
        assert_eq!(&v2[4..8], &16u32.to_le_bytes());
        assert_eq!(decode_connection_v2(&v2), Some(0b111));
    }

    #[test]
    fn port_classification() {
        let ss = usb_port_info(21, Some((PORT_USER_CONNECTABLE, 1)), Some(PROTOCOL_USB300));
        assert_eq!(
            (ss.speed_class.as_str(), ss.connector, ss.companion),
            ("usb3", Some(3), Some(1))
        );
        let hs = usb_port_info(1, Some((PORT_USER_CONNECTABLE, 21)), Some(0b011));
        assert_eq!((hs.speed_class.as_str(), hs.connector), ("usb2", Some(3)));
        let usb2_only = usb_port_info(2, Some((PORT_USER_CONNECTABLE, 0)), Some(0b011));
        assert_eq!((usb2_only.connector, usb2_only.companion), (Some(0), None));
        let internal = usb_port_info(14, Some((0, 0)), Some(0b011));
        assert_eq!(
            (internal.connector, internal.user_connectable),
            (Some(255), Some(false))
        );
        let type_c = usb_port_info(
            5,
            Some((PORT_USER_CONNECTABLE | PORT_TYPE_C, 17)),
            Some(0b011),
        );
        assert_eq!(type_c.connector, Some(9));
        let unknown = usb_port_info(9, None, None);
        assert_eq!(
            (
                unknown.connector,
                unknown.user_connectable,
                unknown.speed_class.as_str()
            ),
            (None, None, "usb2")
        );
    }
}
