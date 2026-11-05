//! PawnIO, the signed kernel driver that CPU, RAM and motherboard sensors are read
//! through. It is a separate product, so it may not be on the machine.

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_ANY, RegGetValueW};
use windows::core::w;

pub const DOWNLOAD_PAGE: &str = "https://pawnio.eu";

pub fn is_installed() -> bool {
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SYSTEM\CurrentControlSet\Services\PawnIO"),
            w!("ImagePath"),
            RRF_RT_ANY,
            None,
            None,
            None,
        )
    };
    status == ERROR_SUCCESS
}
