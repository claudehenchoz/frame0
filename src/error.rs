use core::fmt;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Clone, Copy)]
pub enum Error {
    Hr(i32),
    Win32(u32),
    Generic(&'static str),
}

impl Error {
    pub fn from_hr(hr: i32) -> Self {
        Error::Hr(hr)
    }
    pub fn last_win32() -> Self {
        unsafe { Error::Win32(windows_sys::Win32::Foundation::GetLastError()) }
    }
    pub fn msg(s: &'static str) -> Self {
        Error::Generic(s)
    }
}

impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Error::Hr(e.code().0)
    }
}

pub fn hr_ok(hr: i32) -> Result<()> {
    if hr >= 0 {
        Ok(())
    } else {
        Err(Error::Hr(hr))
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Hr(h) => write!(f, "HRESULT 0x{:08X}", *h as u32),
            Error::Win32(e) => write!(f, "Win32 {}", e),
            Error::Generic(s) => f.write_str(s),
        }
    }
}
