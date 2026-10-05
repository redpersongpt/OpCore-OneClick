//! Minimal AML (ACPI Machine Language) encoder for building SSDTs:
//! DefinitionBlock header + checksum, NameString encoding, PkgLength,
//! External, Scope, Device, Name, Method, If/Else, Return, LEqual/LNot/LAnd,
//! Store, integer constants (Zero/One/Ones/Byte/Word/DWord/QWord), String,
//! Buffer, Package, Arg0..6, Local0..7, method invocation (e.g. `_OSI`).
//! Every SSDT produced must disassemble cleanly with `iasl -d`.

/// An AML byte stream builder.
#[derive(Debug, Default, Clone)]
pub struct AmlBuilder {
    pub bytes: Vec<u8>,
}

/// Build a complete SSDT table: 36-byte header (signature "SSDT", revision 2,
/// OEM id, OEM table id, OEM revision, creator "INTL" 0x20200925) + body,
/// with correct length and checksum.
pub fn definition_block(oem_id: &str, oem_table_id: &str, oem_revision: u32, body: &[u8]) -> Vec<u8> {
    todo!("definition_block {oem_id} {oem_table_id} {oem_revision} {}", body.len())
}

/// Encode an ACPI NameString ("\\_SB.PCI0.LPCB", "EC", "^PCI0") with
/// DualNamePrefix/MultiNamePrefix and NameSeg padding with '_'.
pub fn name_string(path: &str) -> Vec<u8> {
    todo!("name_string {path}")
}

/// Encode a PkgLength for a payload of `len` bytes (the encoding includes itself).
pub fn pkg_length(len: usize) -> Vec<u8> {
    todo!("pkg_length {len}")
}
