//! Scrolling capture: grab a rect, wheel-scroll the content under it, grab
//! again, find how far the content moved and append the newly revealed rows.

use crate::error::Result;
use crate::gfx::bitmap::Bitmap;
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
    VK_ESCAPE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

const MAX_STEPS: usize = 40;
const MAX_HEIGHT: usize = 30_000;
const SETTLE_MS: u32 = 200;

pub fn capture(rect: super::ScreenRect) -> Result<Bitmap> {
    let (x, y, w, h) = rect;
    let first = super::bitblt::region(x, y, w, h, false)?;
    let (wu, hu) = (w as usize, h as usize);
    let stride = wu * 4;
    // Ignore the right-hand strip (scrollbar thumb moves every frame) and a
    // few px on the left when hashing rows.
    let hash_x1 = wu - (wu / 8).min(40);
    let hash_x0 = (wu / 64).min(4);

    let mut saved: POINT = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut saved);
        SetCursorPos(x + w / 2, y + h / 2);
    }
    // Start conservatively, then adapt once we know how far one notch scrolls.
    let mut notches = (h / 250).clamp(1, 10);

    let mut prev_hash = row_hashes(&first, hash_x0, hash_x1);
    let mut body: Vec<u8> = first.bgra.clone();
    let mut footer: Option<Vec<u8>> = None;
    let mut footer_rows = 0usize;

    for _ in 0..MAX_STEPS {
        if esc_pressed() {
            break;
        }
        wheel(-120 * notches);
        crate::win::window::sleep_ms(SETTLE_MS);
        if esc_pressed() {
            break;
        }
        let Ok(cur) = super::bitblt::region(x, y, w, h, false) else { break };
        let cur_hash = row_hashes(&cur, hash_x0, hash_x1);

        // Fixed header / footer: rows identical at the same position.
        let top = (0..hu).take_while(|&i| prev_hash[i] == cur_hash[i]).count();
        if top == hu {
            break; // nothing moved: end of content
        }
        let bot = (0..hu - top).take_while(|&i| prev_hash[hu - 1 - i] == cur_hash[hu - 1 - i]).count();
        let (c0, c1) = (top, hu - bot);
        let Some(shift) = best_shift(&prev_hash[c0..c1], &cur_hash[c0..c1]) else { break };

        if footer.is_none() {
            // Strip the footer from the accumulated body once; re-appended at the end.
            footer_rows = bot;
            footer = Some(body[(hu - bot) * stride..].to_vec());
            body.truncate((hu - bot) * stride);
        }
        // Aim for ~60 % of the content height per step (keeps a solid overlap).
        let per_notch = (shift as i32 / notches).max(1);
        notches = (((c1 - c0) as i32 * 6 / 10) / per_notch).clamp(1, 20);
        let new_from = c1 - shift;
        let new_to = c1.min(hu - footer_rows);
        if new_to > new_from {
            body.extend_from_slice(&cur.bgra[new_from * stride..new_to * stride]);
        }
        prev_hash = cur_hash;
        if body.len() / stride >= MAX_HEIGHT {
            break;
        }
    }
    unsafe {
        SetCursorPos(saved.x, saved.y);
    }
    if let Some(f) = footer {
        body.extend_from_slice(&f);
    }
    let rows = (body.len() / stride).min(MAX_HEIGHT);
    body.truncate(rows * stride);
    Ok(Bitmap { width: w as u32, height: rows as u32, bgra: body })
}

fn esc_pressed() -> bool {
    unsafe { (GetAsyncKeyState(VK_ESCAPE as i32) as u16 & 0x8000) != 0 }
}

fn wheel(delta: i32) {
    unsafe {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: delta as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        SendInput(1, &input, core::mem::size_of::<INPUT>() as i32);
    }
}

/// Per-row hash over sampled pixels. Bit 0 is set when the row is uniform
/// (a blank row carries no alignment information).
fn row_hashes(b: &Bitmap, x0: usize, x1: usize) -> Vec<u64> {
    let stride = b.width as usize * 4;
    (0..b.height as usize)
        .map(|y| {
            let row = &b.bgra[y * stride..(y + 1) * stride];
            let first = u32::from_ne_bytes([row[x0 * 4], row[x0 * 4 + 1], row[x0 * 4 + 2], 0]);
            let mut h: u64 = 0xcbf29ce484222325;
            let mut uniform = true;
            for px in row[x0 * 4..x1 * 4].chunks_exact(4) {
                let v = u32::from_ne_bytes([px[0], px[1], px[2], 0]);
                uniform &= v == first;
                h = (h ^ v as u64).wrapping_mul(0x100000001b3);
            }
            (h & !1) | uniform as u64
        })
        .collect()
}

/// Content scrolled up by `s` rows means cur[i] == prev[i + s]. Pick the
/// shift with the lowest mismatch ratio over non-uniform rows.
fn best_shift(prev: &[u64], cur: &[u64]) -> Option<usize> {
    let n = prev.len();
    let mut best: Option<(usize, u32, u32)> = None; // (shift, mismatches, compared)
    for s in 1..n {
        let (mut miss, mut cmp) = (0u32, 0u32);
        for i in 0..n - s {
            let (a, b) = (cur[i], prev[i + s]);
            if a & 1 == 1 && b & 1 == 1 {
                if a != b {
                    miss += 1;
                }
                continue; // both blank: no information
            }
            cmp += 1;
            if a != b {
                miss += 1;
            }
        }
        if cmp < 8 {
            continue;
        }
        let better = match best {
            None => true,
            // miss/cmp < bmiss/bcmp, ties go to the larger comparison set.
            Some((_, bm, bc)) => {
                let l = miss as u64 * bc as u64;
                let r = bm as u64 * cmp as u64;
                l < r || (l == r && cmp > bc)
            }
        };
        if better {
            best = Some((s, miss, cmp));
        }
    }
    let (s, miss, cmp) = best?;
    // Require a convincing match (≤ 10 % mismatching rows).
    (miss * 10 <= cmp).then_some(s)
}
