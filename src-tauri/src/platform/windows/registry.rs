//! Read-only access to `HKEY_LOCAL_MACHINE` (firmware mode, Secure Boot,
//! display memory sizes, ACPI tables saved by the OS).

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, HKEY,
    HKEY_LOCAL_MACHINE, KEY_READ,
};

use super::raw::{registry_number, wide_to_string};

/// Largest value read (ACPI tables are well below this).
const MAX_VALUE_LEN: u32 = 16 * 1024 * 1024;
const MAX_ENTRIES: u32 = 4096;

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// An open registry key, closed on drop.
pub struct RegKey(HKEY);

impl RegKey {
    /// Open `HKLM\<path>` for reading.
    pub fn local_machine(path: &str) -> Option<Self> {
        Self::open(HKEY_LOCAL_MACHINE, path)
    }

    fn open(parent: HKEY, path: &str) -> Option<Self> {
        let wide = to_wide(path);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated and outlives the call; `key` receives the handle.
        let rc = unsafe { RegOpenKeyExW(parent, wide.as_ptr(), 0, KEY_READ, &mut key) };
        (rc == ERROR_SUCCESS && !key.is_null()).then_some(Self(key))
    }

    pub fn subkey(&self, name: &str) -> Option<Self> {
        Self::open(self.0, name)
    }

    pub fn subkey_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for index in 0..MAX_ENTRIES {
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            // SAFETY: `buf` holds `len` UTF-16 units; optional outputs are null.
            let rc = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    buf.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc != ERROR_SUCCESS {
                break;
            }
            names.push(wide_to_string(&buf[..(len as usize).min(buf.len())]));
        }
        names
    }

    /// Value names with their registry type, in enumeration order.
    pub fn value_names(&self) -> Vec<(String, u32)> {
        let mut values = Vec::new();
        let mut buf = vec![0u16; 16_384];
        for index in 0..MAX_ENTRIES {
            let mut len = buf.len() as u32;
            let mut kind = 0u32;
            // SAFETY: `buf` holds `len` UTF-16 units; the data pointers are null
            // so only the name and type are returned.
            let rc = unsafe {
                RegEnumValueW(
                    self.0,
                    index,
                    buf.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    &mut kind,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc != ERROR_SUCCESS {
                break;
            }
            values.push((wide_to_string(&buf[..(len as usize).min(buf.len())]), kind));
        }
        values
    }

    /// Raw data and type of a value.
    pub fn value(&self, name: &str) -> Option<(u32, Vec<u8>)> {
        let wide = to_wide(name);
        let mut kind = 0u32;
        let mut size = 0u32;
        // SAFETY: a null data pointer asks for the size only.
        let rc = unsafe {
            RegQueryValueExW(
                self.0,
                wide.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        // The value can grow between the two calls; retry a few times.
        for _ in 0..4 {
            if size > MAX_VALUE_LEN {
                return None;
            }
            let mut data = vec![0u8; size as usize];
            let mut len = size;
            // SAFETY: `data` is valid for `len` bytes.
            let rc = unsafe {
                RegQueryValueExW(
                    self.0,
                    wide.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut len,
                )
            };
            match rc {
                ERROR_SUCCESS => {
                    data.truncate((len as usize).min(data.len()));
                    return Some((kind, data));
                }
                ERROR_MORE_DATA => size = len.max(size.saturating_mul(2)),
                _ => return None,
            }
        }
        None
    }

    /// DWORD / QWORD (or 4/8-byte binary) value as a number.
    pub fn number(&self, name: &str) -> Option<u64> {
        self.value(name)
            .and_then(|(_, data)| registry_number(&data))
    }
}

impl Drop for RegKey {
    fn drop(&mut self) {
        // SAFETY: the handle came from RegOpenKeyExW and is closed once.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

/// Numeric value of `HKLM\<path>\<name>`.
pub fn local_machine_number(path: &str, name: &str) -> Option<u64> {
    RegKey::local_machine(path)?.number(name)
}
