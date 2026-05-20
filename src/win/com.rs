use crate::error::Result;
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

pub fn init_apartment() -> Result<()> {
    unsafe {
        let hr = CoInitializeEx(core::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        if hr < 0 && hr != -2147417850i32 {
            return Err(crate::error::Error::from_hr(hr));
        }
    }
    Ok(())
}

pub fn uninit_apartment() {
    unsafe { CoUninitialize() }
}
