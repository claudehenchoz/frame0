use crate::win::hotkey::HotkeySpec;
use crate::win::wide;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_RoamingAppData, KF_FLAG_CREATE, SHGetKnownFolderPath,
};
use windows_sys::core::PWSTR;

const APP_DIR: &str = "frame";
const INI_NAME: &str = "frame.ini";

pub struct Config {
    pub ini_path: String,
    pub hotkey: HotkeySpec,
    pub include_cursor: bool,
    pub last_export_dir: String,
}

impl Config {
    pub fn load() -> Self {
        let ini_path = ensure_ini_path();
        let hotkey_str = read_ini_string(&ini_path, "hotkey", "capture", "Ctrl+Win+PgDn");
        let hotkey = crate::win::hotkey::parse(&hotkey_str).unwrap_or_else(HotkeySpec::default_capture);
        let include_cursor = read_ini_string(&ini_path, "capture", "include_cursor", "false")
            .eq_ignore_ascii_case("true");
        let last_export_dir = read_ini_string(&ini_path, "ui", "last_export_dir", "");
        Self {
            ini_path,
            hotkey,
            include_cursor,
            last_export_dir,
        }
    }

    pub fn set_last_export_dir(&mut self, dir: &str) {
        self.last_export_dir = dir.into();
        write_ini_string(&self.ini_path, "ui", "last_export_dir", dir);
    }
}

fn ensure_ini_path() -> String {
    let mut path_pw: PWSTR = core::ptr::null_mut();
    unsafe {
        // Best-effort COM init; safe to call multiple times.
        CoInitializeEx(core::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let hr = SHGetKnownFolderPath(
            &FOLDERID_RoamingAppData,
            KF_FLAG_CREATE as u32,
            0,
            &mut path_pw,
        );
        if hr < 0 || path_pw.is_null() {
            return String::from(INI_NAME);
        }
        let mut len = 0;
        while *path_pw.add(len) != 0 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(path_pw, len);
        let mut roaming = String::from_utf16_lossy(slice);
        // SHGetKnownFolderPath buffer must be freed with CoTaskMemFree
        windows_sys::Win32::System::Com::CoTaskMemFree(path_pw as _);
        roaming.push('\\');
        roaming.push_str(APP_DIR);
        let dir_w = wide(&roaming);
        if GetFileAttributesW(dir_w.as_ptr()) == INVALID_FILE_ATTRIBUTES {
            let _ = std::fs::create_dir_all(&roaming);
        }
        roaming.push('\\');
        roaming.push_str(INI_NAME);
        roaming
    }
}

fn read_ini_string(path: &str, section: &str, key: &str, default: &str) -> String {
    use windows_sys::Win32::System::WindowsProgramming::GetPrivateProfileStringW;
    let sec = wide(section);
    let key_w = wide(key);
    let def = wide(default);
    let path_w = wide(path);
    let mut buf = [0u16; 512];
    unsafe {
        let n = GetPrivateProfileStringW(
            sec.as_ptr(),
            key_w.as_ptr(),
            def.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            path_w.as_ptr(),
        );
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn write_ini_string(path: &str, section: &str, key: &str, value: &str) {
    use windows_sys::Win32::System::WindowsProgramming::WritePrivateProfileStringW;
    let sec = wide(section);
    let key_w = wide(key);
    let val = wide(value);
    let path_w = wide(path);
    unsafe {
        WritePrivateProfileStringW(sec.as_ptr(), key_w.as_ptr(), val.as_ptr(), path_w.as_ptr());
    }
}
