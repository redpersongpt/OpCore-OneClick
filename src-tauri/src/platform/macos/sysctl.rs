//! In-process `sysctlbyname` reads (no child process).

use std::ffi::CString;

use crate::platform::common::clean_text;

/// String value ("machdep.cpu.brand_string").
pub fn string(name: &str) -> Option<String> {
    let cname = CString::new(name).ok()?;
    let mut len: libc::size_t = 0;
    // SAFETY: a null output buffer asks for the value size only.
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len == 0 || len > 64 * 1024 {
        return None;
    }
    let mut buf = vec![0u8; len];
    // SAFETY: `buf` is valid for `len` bytes and `len` is updated to the bytes written.
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(len);
    clean_text(&String::from_utf8_lossy(&buf))
}

/// Integer value (32 or 64 bit, "hw.memsize", "hw.physicalcpu").
pub fn number(name: &str) -> Option<u64> {
    let cname = CString::new(name).ok()?;
    let mut buf = [0u8; 8];
    let mut len: libc::size_t = buf.len();
    // SAFETY: `buf` is valid for 8 bytes; the kernel writes at most `len` bytes.
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    match len {
        4 => Some(u64::from(u32::from_ne_bytes([
            buf[0], buf[1], buf[2], buf[3],
        ]))),
        8 => Some(u64::from_ne_bytes(buf)),
        _ => None,
    }
}

/// Feature flags (same names as the CPUID decoder) from the Intel-only
/// `machdep.cpu.features` / `leaf7_features` / `extfeatures` word lists.
pub fn features_from_words(words: &[&str]) -> Vec<String> {
    const MAP: &[(&str, &str)] = &[
        ("SSE3", "sse3"),
        ("SSSE3", "ssse3"),
        ("SSE4.1", "sse4_1"),
        ("SSE4.2", "sse4_2"),
        ("AVX1.0", "avx"),
        ("AVX2", "avx2"),
        ("AVX512F", "avx512f"),
        ("VMX", "vmx"),
        ("VMM", "hypervisor"),
    ];
    let present: Vec<&str> = words.iter().flat_map(|w| w.split_whitespace()).collect();
    MAP.iter()
        .filter(|(word, _)| present.iter().any(|p| p.eq_ignore_ascii_case(word)))
        .map(|(_, name)| (*name).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intel_feature_words() {
        let features = "FPU VME SSE SSE2 SSE3 PCLMULQDQ VMX SSSE3 FMA SSE4.1 SSE4.2 POPCNT AVX1.0 RDRAND F16C VMM";
        let leaf7 = "RDWRFSGS BMI1 AVX2 SMEP BMI2 ERMS";
        assert_eq!(
            features_from_words(&[features, leaf7, "SYSCALL XD EM64T"]),
            [
                "sse3",
                "ssse3",
                "sse4_1",
                "sse4_2",
                "avx",
                "avx2",
                "vmx",
                "hypervisor"
            ]
        );
        assert!(features_from_words(&["FPU SSE SSE2"]).is_empty());
    }

    #[test]
    fn reads_basic_values() {
        assert!(number("hw.memsize").unwrap_or(0) > 0);
        assert!(number("hw.logicalcpu").unwrap_or(0) > 0);
        assert!(string("kern.ostype").is_some());
        assert_eq!(number("no.such.sysctl"), None);
        assert_eq!(string("no.such.sysctl"), None);
    }
}
