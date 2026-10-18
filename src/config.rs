//! `frame0.ini` persistence (GetPrivateProfileString). The path defaults to
//! `%APPDATA%\frame0\frame0.ini` and can be overridden with `FRAME0_INI`.
//! The pre-rename `%APPDATA%\frame` folder is moved over on first run.

use crate::capture::{CaptureSettings, Mode, Output};
use crate::win::hotkey::HotkeySpec;
use crate::win::wide;
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows_sys::Win32::UI::Shell::{FOLDERID_RoamingAppData, KF_FLAG_CREATE, SHGetKnownFolderPath};
use windows_sys::core::PWSTR;

const APP_DIR: &str = "frame0";
const INI_NAME: &str = "frame0.ini";
/// Pre-rename locations, migrated once.
const OLD_APP_DIR: &str = "frame";
const OLD_INI_NAME: &str = "frame.ini";
pub const DEFAULT_HOTKEY: &str = "Ctrl+Win+PgDn";

pub struct Config {
    pub ini_path: String,
    pub hotkey_str: String,
    pub hotkey: HotkeySpec,
    /// Options applied to new captures started from the tray / hotkey.
    pub include_cursor: bool,
    pub delay: u32,
    pub interval: u32,
    pub output: Output,
    pub effects: u32,
    pub last_fixed: (i32, i32),
    pub last: Option<CaptureSettings>,
    pub last_export_dir: String,
}

impl Config {
    pub fn load() -> Self {
        let ini_path = ensure_ini_path();
        // A UTF-16LE BOM makes the *PrivateProfile*W APIs keep Unicode
        // (preset names, paths) instead of converting to the ANSI code page.
        if !std::path::Path::new(&ini_path).exists() {
            let _ = std::fs::write(&ini_path, [0xFFu8, 0xFE]);
        }
        let p = ini_path.as_str();
        let hotkey_str = read_ini_string(p, "hotkey", "capture", DEFAULT_HOTKEY);
        let hotkey = crate::win::hotkey::parse(&hotkey_str).unwrap_or_else(HotkeySpec::default_capture);
        let fw = read_int(p, "capture", "fixed_w", 800);
        let fh = read_int(p, "capture", "fixed_h", 600);
        Self {
            hotkey,
            hotkey_str,
            include_cursor: read_bool(p, "capture", "include_cursor"),
            delay: read_int(p, "capture", "delay", 0).max(0) as u32,
            interval: read_int(p, "capture", "interval", 10).max(1) as u32,
            output: Output::parse(&read_ini_string(p, "capture", "output", "editor")),
            effects: read_int(p, "capture", "effects", 0) as u32,
            last_fixed: (fw, fh),
            last: read_settings(p, "last"),
            last_export_dir: read_ini_string(p, "ui", "last_export_dir", ""),
            ini_path,
        }
    }

    /// Persist the capture options (not presets / last).
    pub fn save_options(&self) {
        let p = self.ini_path.as_str();
        write_ini_string(p, "hotkey", "capture", &self.hotkey_str);
        write_ini_string(p, "capture", "include_cursor", bool_str(self.include_cursor));
        write_ini_string(p, "capture", "delay", &self.delay.to_string());
        write_ini_string(p, "capture", "interval", &self.interval.to_string());
        write_ini_string(p, "capture", "output", self.output.as_str());
        write_ini_string(p, "capture", "effects", &self.effects.to_string());
        write_ini_string(p, "capture", "fixed_w", &self.last_fixed.0.to_string());
        write_ini_string(p, "capture", "fixed_h", &self.last_fixed.1.to_string());
    }

    pub fn set_last(&mut self, s: &CaptureSettings) {
        write_settings(&self.ini_path, "last", s);
        self.last = Some(s.clone());
    }

    pub fn set_last_export_dir(&mut self, dir: &str) {
        self.last_export_dir = dir.into();
        write_ini_string(&self.ini_path, "ui", "last_export_dir", dir);
    }

    /// Presets live in `[preset.0]`, `[preset.1]`, … (contiguous).
    pub fn presets(&self) -> Vec<(String, CaptureSettings)> {
        let mut out = Vec::new();
        for i in 0..100 {
            let sec = preset_section(i);
            let name = read_ini_string(&self.ini_path, &sec, "name", "");
            if name.is_empty() {
                break;
            }
            if let Some(s) = read_settings(&self.ini_path, &sec) {
                out.push((name, s));
            }
        }
        out
    }

    pub fn add_preset(&self, name: &str, s: &CaptureSettings) {
        let mut all = self.presets();
        all.push((name.to_string(), s.clone()));
        self.write_presets(&all, all.len() - 1);
    }

    pub fn delete_preset(&self, index: usize) {
        let mut all = self.presets();
        if index < all.len() {
            all.remove(index);
            self.write_presets(&all, index);
            // Drop the now-unused trailing section.
            delete_section(&self.ini_path, &preset_section(all.len()));
        }
    }

    fn write_presets(&self, all: &[(String, CaptureSettings)], from: usize) {
        for (i, (name, s)) in all.iter().enumerate().skip(from) {
            let sec = preset_section(i);
            delete_section(&self.ini_path, &sec);
            write_ini_string(&self.ini_path, &sec, "name", name);
            write_settings(&self.ini_path, &sec, s);
        }
    }
}

fn preset_section(i: usize) -> String {
    let mut s = String::from("preset.");
    s.push_str(&i.to_string());
    s
}

fn bool_str(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

fn read_bool(path: &str, section: &str, key: &str) -> bool {
    read_ini_string(path, section, key, "false").eq_ignore_ascii_case("true")
}

fn read_int(path: &str, section: &str, key: &str, default: i32) -> i32 {
    read_ini_string(path, section, key, "").trim().parse().unwrap_or(default)
}

fn parse_ints<const N: usize>(s: &str) -> Option<[i32; N]> {
    let mut out = [0; N];
    let mut it = s.split(',');
    for v in out.iter_mut() {
        *v = it.next()?.trim().parse().ok()?;
    }
    Some(out)
}

fn join_ints(v: &[i32]) -> String {
    let mut s = String::new();
    for (i, n) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&n.to_string());
    }
    s
}

pub fn write_settings(path: &str, sec: &str, s: &CaptureSettings) {
    let w = |k: &str, v: &str| write_ini_string(path, sec, k, v);
    w("mode", s.mode.as_str());
    w("monitor", &s.monitor.to_string());
    w("region", &s.region.map(|r| join_ints(&[r.0, r.1, r.2, r.3])).unwrap_or_default());
    w("fixed_size", &s.fixed_size.map(|r| join_ints(&[r.0, r.1])).unwrap_or_default());
    w("fixed_pos", &s.fixed_pos.map(|r| join_ints(&[r.0, r.1])).unwrap_or_default());
    w("delay", &s.delay.to_string());
    w("interval", &s.interval.to_string());
    w("include_cursor", bool_str(s.include_cursor));
    w("output", s.output.as_str());
    w("effects", &s.effects.to_string());
}

pub fn read_settings(path: &str, sec: &str) -> Option<CaptureSettings> {
    let r = |k: &str| read_ini_string(path, sec, k, "");
    let mut s = CaptureSettings::new(Mode::parse(&r("mode"))?);
    s.monitor = r("monitor").parse().unwrap_or(0);
    s.region = parse_ints::<4>(&r("region")).map(|v| (v[0], v[1], v[2], v[3]));
    s.fixed_size = parse_ints::<2>(&r("fixed_size")).map(|v| (v[0], v[1]));
    s.fixed_pos = parse_ints::<2>(&r("fixed_pos")).map(|v| (v[0], v[1]));
    s.delay = r("delay").parse().unwrap_or(0);
    s.interval = r("interval").parse().unwrap_or(0);
    s.include_cursor = r("include_cursor").eq_ignore_ascii_case("true");
    s.output = Output::parse(&r("output"));
    s.effects = r("effects").parse().unwrap_or(0);
    Some(s)
}

/// `%APPDATA%\frame0` (created if missing).
pub fn app_dir() -> String {
    let mut path_pw: PWSTR = core::ptr::null_mut();
    unsafe {
        // Best-effort COM init; safe to call multiple times.
        CoInitializeEx(core::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let hr = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_CREATE as u32, 0, &mut path_pw);
        if hr < 0 || path_pw.is_null() {
            return String::from(".");
        }
        let mut len = 0;
        while *path_pw.add(len) != 0 {
            len += 1;
        }
        let mut dir = String::from_utf16_lossy(core::slice::from_raw_parts(path_pw, len));
        // SHGetKnownFolderPath buffer must be freed with CoTaskMemFree
        windows_sys::Win32::System::Com::CoTaskMemFree(path_pw as _);
        dir.push('\\');
        let old = format!("{}{}", dir, OLD_APP_DIR);
        dir.push_str(APP_DIR);
        if !std::path::Path::new(&dir).exists() && std::path::Path::new(&old).is_dir() {
            // One-time move of the pre-rename settings + stamps folder.
            if std::fs::rename(&old, &dir).is_ok() {
                let _ = std::fs::rename(format!("{}\\{}", dir, OLD_INI_NAME), format!("{}\\{}", dir, INI_NAME));
            }
        }
        let _ = std::fs::create_dir_all(&dir);
        dir
    }
}

fn ensure_ini_path() -> String {
    if let Ok(p) = std::env::var("FRAME0_INI") {
        if !p.is_empty() {
            return p;
        }
    }
    let mut p = app_dir();
    p.push('\\');
    p.push_str(INI_NAME);
    p
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

fn delete_section(path: &str, section: &str) {
    use windows_sys::Win32::System::WindowsProgramming::WritePrivateProfileStringW;
    let sec = wide(section);
    let path_w = wide(path);
    unsafe {
        WritePrivateProfileStringW(sec.as_ptr(), core::ptr::null(), core::ptr::null(), path_w.as_ptr());
    }
}
