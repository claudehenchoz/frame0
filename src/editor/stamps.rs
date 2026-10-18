//! Stamp catalogue.
//!
//! Built-in stamps cost nothing in the binary: emoji stamps (`"e:<emoji>"`)
//! render through DirectWrite's colour-font path (Segoe UI Emoji), and label
//! badges (`"l:<TEXT>"`) are drawn as vector rounded rectangles. User stamps
//! are PNG/JPG files dropped into `%APPDATA%\frame0\stamps\` — a file in a
//! sub-folder lands in that category, files in the root land in "Custom".
//! Their asset id is `"f:<path relative to the stamps folder>"`.

use crate::gfx::bitmap::Bitmap;
use crate::gfx::color::Color;
use std::cell::RefCell;
use std::rc::Rc;

pub struct StampDef {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
}

macro_rules! s {
    ($id:expr, $name:expr, $cat:expr) => {
        StampDef { id: $id, name: $name, category: $cat }
    };
}

pub const BUILTIN: &[StampDef] = &[
    s!("e:\u{2705}", "check done ok yes", "Marks"),
    s!("e:\u{274C}", "cross no wrong delete x", "Marks"),
    s!("e:\u{2753}", "question help unknown", "Marks"),
    s!("e:\u{2757}", "exclamation important attention", "Marks"),
    s!("e:\u{26A0}\u{FE0F}", "warning caution alert", "Marks"),
    s!("e:\u{1F6AB}", "forbidden stop no blocked", "Marks"),
    s!("e:\u{2B50}", "star favourite rating", "Marks"),
    s!("e:\u{1F4CC}", "pin pushpin note", "Marks"),
    s!("e:\u{1F3AF}", "target goal focus", "Marks"),
    s!("e:\u{1F534}", "red dot circle", "Marks"),
    s!("e:\u{1F7E2}", "green dot circle", "Marks"),
    s!("e:\u{27A1}\u{FE0F}", "arrow right next", "Marks"),
    s!("e:\u{1F44D}", "thumbs up like good", "Faces"),
    s!("e:\u{1F44E}", "thumbs down dislike bad", "Faces"),
    s!("e:\u{1F642}", "smile happy face", "Faces"),
    s!("e:\u{1F615}", "confused face unsure", "Faces"),
    s!("e:\u{1F621}", "angry face mad", "Faces"),
    s!("e:\u{1F914}", "thinking face hmm", "Faces"),
    s!("e:\u{1F389}", "party celebrate tada", "Faces"),
    s!("e:\u{1F440}", "eyes look see", "Faces"),
    s!("e:\u{1F41B}", "bug defect issue", "Objects"),
    s!("e:\u{1F4A1}", "idea bulb tip", "Objects"),
    s!("e:\u{1F525}", "fire hot urgent", "Objects"),
    s!("e:\u{1F512}", "lock secure private", "Objects"),
    s!("e:\u{1F513}", "unlock open", "Objects"),
    s!("e:\u{1F50D}", "search magnifier find", "Objects"),
    s!("e:\u{23F0}", "clock time alarm", "Objects"),
    s!("e:\u{1F4CE}", "paperclip attachment", "Objects"),
    s!("e:\u{2709}\u{FE0F}", "mail envelope email", "Objects"),
    s!("e:\u{2699}\u{FE0F}", "gear settings config", "Objects"),
    s!("e:\u{1F5D1}\u{FE0F}", "trash delete bin", "Objects"),
    s!("e:\u{1F4BE}", "save disk floppy", "Objects"),
    s!("l:NEW", "new label badge", "Labels"),
    s!("l:TODO", "todo label badge", "Labels"),
    s!("l:DONE", "done label badge", "Labels"),
    s!("l:BUG", "bug label badge", "Labels"),
    s!("l:FIXED", "fixed label badge", "Labels"),
    s!("l:WIP", "wip work in progress label", "Labels"),
    s!("l:DRAFT", "draft label badge", "Labels"),
    s!("l:APPROVED", "approved label badge", "Labels"),
    s!("l:CONFIDENTIAL", "confidential secret label", "Labels"),
    s!("l:BEFORE", "before label badge", "Labels"),
    s!("l:AFTER", "after label badge", "Labels"),
];

pub const CATEGORIES: &[&str] = &["All", "Marks", "Faces", "Objects", "Labels", "Custom"];

pub const DEFAULT: &str = "e:\u{2705}";

/// Colour for a label badge, derived from its text.
pub fn label_color(text: &str) -> Color {
    match text {
        "DONE" | "FIXED" | "APPROVED" | "AFTER" => Color::rgb(0x2E, 0xA8, 0x4F),
        "BUG" | "CONFIDENTIAL" => Color::rgb(0xE5, 0x3E, 0x3E),
        "TODO" | "WIP" | "BEFORE" => Color::rgb(0xF0, 0x8C, 0x00),
        "DRAFT" => Color::rgb(0x80, 0x80, 0x88),
        _ => Color::rgb(0x2C, 0x7B, 0xE5),
    }
}

/// Map a v1/v2 project's numeric stamp index to the new asset ids.
pub fn legacy_asset(idx: u8) -> &'static str {
    match idx {
        1 => "e:\u{274C}",
        2 => "e:\u{1F534}",
        3 => "e:\u{2B50}",
        _ => "e:\u{2705}",
    }
}

pub struct Entry {
    pub id: String,
    pub name: String,
    pub category: String,
}

pub fn stamps_dir() -> String {
    format!("{}\\stamps", crate::config::app_dir())
}

thread_local! {
    static CUSTOM: RefCell<Option<Vec<Entry>>> = const { RefCell::new(None) };
    static BITMAPS: RefCell<Vec<(String, Rc<Bitmap>)>> = const { RefCell::new(Vec::new()) };
}

/// Forget the cached folder listing (e.g. after the user adds files).
pub fn refresh() {
    CUSTOM.with(|c| *c.borrow_mut() = None);
}

fn is_image(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".png") || l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".bmp") || l.ends_with(".gif")
}

fn scan() -> Vec<Entry> {
    let root = stamps_dir();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&root) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if path.is_dir() {
            if let Ok(sub) = std::fs::read_dir(&path) {
                for f in sub.flatten() {
                    let fname = f.file_name().to_string_lossy().into_owned();
                    if is_image(&fname) {
                        out.push(Entry {
                            id: format!("f:{}\\{}", name, fname),
                            name: stem(&fname),
                            category: name.clone(),
                        });
                    }
                }
            }
        } else if is_image(&name) {
            out.push(Entry { id: format!("f:{}", name), name: stem(&name), category: "Custom".into() });
        }
    }
    out
}

fn stem(f: &str) -> String {
    match f.rfind('.') {
        Some(i) => f[..i].replace(['_', '-'], " "),
        None => f.to_string(),
    }
}

/// Category names: built-ins plus any user sub-folders.
pub fn categories() -> Vec<String> {
    let mut v: Vec<String> = CATEGORIES.iter().map(|s| s.to_string()).collect();
    CUSTOM.with(|c| {
        let mut c = c.borrow_mut();
        let list = c.get_or_insert_with(scan);
        for e in list.iter() {
            if !v.contains(&e.category) {
                v.push(e.category.clone());
            }
        }
    });
    v
}

/// All stamps matching `category` ("All" = any) and a case-insensitive
/// search query over names/keywords.
pub fn search(category: &str, query: &str) -> Vec<String> {
    let q = query.trim().to_lowercase();
    let cat_ok = |c: &str| category == "All" || category == c;
    let mut out: Vec<String> = BUILTIN
        .iter()
        .filter(|d| cat_ok(d.category) && (q.is_empty() || d.name.contains(q.as_str())))
        .map(|d| d.id.to_string())
        .collect();
    CUSTOM.with(|c| {
        let mut c = c.borrow_mut();
        let list = c.get_or_insert_with(scan);
        for e in list.iter() {
            if cat_ok(&e.category) && (q.is_empty() || e.name.to_lowercase().contains(q.as_str())) {
                out.push(e.id.clone());
            }
        }
    });
    out
}

/// Pixels for a file stamp (cached). Embedded project stamps are registered
/// with `register` so documents stay self-contained.
pub fn file_bitmap(id: &str) -> Option<Rc<Bitmap>> {
    if let Some(b) = BITMAPS.with(|m| m.borrow().iter().find(|(k, _)| k == id).map(|(_, b)| b.clone())) {
        return Some(b);
    }
    let rel = id.strip_prefix("f:")?;
    let path = format!("{}\\{}", stamps_dir(), rel);
    let bm = Rc::new(crate::gfx::gdiplus::decode_file(&path).ok()?);
    register(id, bm.clone());
    Some(bm)
}

pub fn register(id: &str, bm: Rc<Bitmap>) {
    BITMAPS.with(|m| {
        let mut m = m.borrow_mut();
        if !m.iter().any(|(k, _)| k == id) {
            m.push((id.to_string(), bm));
        }
    });
}

/// Natural aspect ratio (w/h) for a stamp, used when placing it.
pub fn aspect(id: &str) -> f32 {
    if let Some(t) = id.strip_prefix("l:") {
        return (t.chars().count() as f32 * 0.62 + 1.2).max(1.6);
    }
    if id.starts_with("f:") {
        if let Some(b) = file_bitmap(id) {
            return b.width as f32 / (b.height.max(1) as f32);
        }
    }
    1.0
}
