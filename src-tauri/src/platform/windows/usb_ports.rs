//! Root hub ports of every USB host controller (port count, USB 2/3
//! protocol, user-connectable / Type-C flags, companion port), read with the
//! hub IOCTLs the way USBView does. Best effort: a hub that cannot be opened
//! simply reports no ports.

use std::collections::HashMap;
use std::mem::{offset_of, size_of};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_IDW, CM_Get_Parent, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW, CR_SUCCESS, DIGCF_DEVICEINTERFACE,
    DIGCF_PRESENT, HDEVINFO, MAX_DEVICE_ID_LEN, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
};
use windows_sys::Win32::Devices::Usb::{
    GUID_DEVINTERFACE_USB_HUB, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2,
    IOCTL_USB_GET_NODE_INFORMATION, IOCTL_USB_GET_PORT_CONNECTOR_PROPERTIES, USB_NODE_INFORMATION,
    USB_PORT_CONNECTOR_PROPERTIES,
};
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::IO::DeviceIoControl;

use crate::contracts::UsbPortInfo;
use crate::error::AppError;

use super::raw::{
    connection_v2_request, connector_properties_request, decode_connection_v2,
    decode_connector_properties, hub_port_count, usb_port_info, wide_to_string,
};

const MAX_HUBS: u32 = 512;

struct DeviceInfoSet(HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        // SAFETY: the set came from SetupDiGetClassDevsW and is destroyed once.
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateFileW and is closed once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct HubInterface {
    path: String,
    devinst: u32,
}

/// Root hub ports keyed by the upper-case instance id of the host
/// controller that owns the root hub.
pub fn root_hub_ports() -> Result<HashMap<String, Vec<UsbPortInfo>>, AppError> {
    // SAFETY: plain call with a static GUID; the result is checked below.
    let set = unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVINTERFACE_USB_HUB,
            std::ptr::null(),
            std::ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == INVALID_HANDLE_VALUE as HDEVINFO || set == 0 {
        return Err(AppError::new(
            "SCAN_USB",
            "USB hubs could not be enumerated",
        ));
    }
    let set = DeviceInfoSet(set);
    let mut out = HashMap::new();
    for index in 0..MAX_HUBS {
        let Some(hub) = hub_interface(&set, index) else {
            break;
        };
        let is_root = device_id(hub.devinst)
            .is_some_and(|id| id.to_ascii_uppercase().starts_with(r"USB\ROOT_HUB"));
        if !is_root {
            continue;
        }
        let Some(controller) = parent_device_id(hub.devinst) else {
            continue;
        };
        out.insert(controller.to_ascii_uppercase(), hub_ports(&hub.path));
    }
    Ok(out)
}

fn hub_interface(set: &DeviceInfoSet, index: u32) -> Option<HubInterface> {
    // SAFETY: zeroed plain-data structs with cbSize set as the API requires.
    let mut interface: SP_DEVICE_INTERFACE_DATA = unsafe { std::mem::zeroed() };
    interface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
    // SAFETY: `interface` is a valid, initialised output struct.
    let ok = unsafe {
        SetupDiEnumDeviceInterfaces(
            set.0,
            std::ptr::null(),
            &GUID_DEVINTERFACE_USB_HUB,
            index,
            &mut interface,
        )
    };
    if ok == 0 {
        return None;
    }
    let mut required = 0u32;
    // SAFETY: a null detail buffer asks for the required size.
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set.0,
            &interface,
            std::ptr::null_mut(),
            0,
            &mut required,
            std::ptr::null_mut(),
        )
    };
    let header = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    if (required as usize) < header || required > 64 * 1024 {
        return None;
    }
    // u64 storage satisfies the struct's alignment.
    let mut buf = vec![0u64; (required as usize).div_ceil(8)];
    let detail = buf.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    // SAFETY: `buf` is at least `header` bytes and suitably aligned.
    unsafe { (*detail).cbSize = header as u32 };
    // SAFETY: as above.
    let mut devinfo: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
    devinfo.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
    // SAFETY: `detail` points to `required` writable bytes.
    let ok = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set.0,
            &interface,
            detail,
            required,
            std::ptr::null_mut(),
            &mut devinfo,
        )
    };
    if ok == 0 {
        return None;
    }
    let bytes: Vec<u8> = buf
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .take(required as usize)
        .collect();
    let offset = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
    let wide: Vec<u16> = bytes
        .get(offset..)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let path = wide_to_string(&wide);
    (!path.is_empty()).then_some(HubInterface {
        path,
        devinst: devinfo.DevInst,
    })
}

fn device_id(devinst: u32) -> Option<String> {
    let mut buf = vec![0u16; MAX_DEVICE_ID_LEN as usize + 1];
    // SAFETY: `buf` holds `buf.len()` UTF-16 units.
    let rc = unsafe { CM_Get_Device_IDW(devinst, buf.as_mut_ptr(), buf.len() as u32, 0) };
    (rc == CR_SUCCESS).then(|| wide_to_string(&buf))
}

fn parent_device_id(devinst: u32) -> Option<String> {
    let mut parent = 0u32;
    // SAFETY: `parent` is a valid output location.
    let rc = unsafe { CM_Get_Parent(&mut parent, devinst, 0) };
    if rc != CR_SUCCESS {
        return None;
    }
    device_id(parent)
}

fn hub_ports(path: &str) -> Vec<UsbPortInfo> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is NUL-terminated; the handle is checked and closed by `Handle`.
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE || raw.is_null() {
        return Vec::new();
    }
    let hub = Handle(raw);
    let mut node = vec![0u8; size_of::<USB_NODE_INFORMATION>()];
    let Some(count) = ioctl(&hub, IOCTL_USB_GET_NODE_INFORMATION, &mut node, 7)
        .and_then(|_| hub_port_count(&node))
    else {
        return Vec::new();
    };
    (1..=count)
        .map(|port| {
            let mut props =
                connector_properties_request(port, size_of::<USB_PORT_CONNECTOR_PROPERTIES>());
            let connector = ioctl(
                &hub,
                IOCTL_USB_GET_PORT_CONNECTOR_PROPERTIES,
                &mut props,
                16,
            )
            .and_then(|_| decode_connector_properties(&props));
            let mut v2 = connection_v2_request(port).to_vec();
            let protocols = ioctl(
                &hub,
                IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2,
                &mut v2,
                12,
            )
            .and_then(|_| decode_connection_v2(&v2));
            usb_port_info(port, connector, protocols)
        })
        .collect()
}

/// Buffered IOCTL that reads and writes `buf` in place; succeeds only when
/// at least `min_returned` bytes came back.
fn ioctl(handle: &Handle, code: u32, buf: &mut [u8], min_returned: u32) -> Option<u32> {
    let len = u32::try_from(buf.len()).ok()?;
    let ptr = buf.as_mut_ptr();
    let mut returned = 0u32;
    // SAFETY: `buf` is valid for `len` bytes as both input and output.
    let ok = unsafe {
        DeviceIoControl(
            handle.0,
            code,
            ptr.cast_const().cast(),
            len,
            ptr.cast(),
            len,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    (ok != 0 && returned >= min_returned).then_some(returned)
}
