use crate::capture::{self, CaptureSettings, FX_BORDER, FX_SHADOW, Mode, Output, ScreenRect};
use crate::config::Config;
use crate::editor::{Document, EditorWindow};
use crate::error::Result;
use crate::gfx::Gfx;
use crate::gfx::bitmap::Bitmap;
use crate::win::tray::{self, Menu, Tray};
use crate::win::wide;
use crate::win::window::{WindowOpts, create_window, message_loop, post_quit, register_class};
use std::cell::RefCell;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, HWND_MESSAGE, IsWindow, KillTimer, MB_ICONWARNING, MB_SETFOREGROUND,
    MB_TOPMOST, MessageBoxW, SetTimer, WM_COMMAND, WM_DESTROY, WM_HOTKEY, WM_TIMER, WS_OVERLAPPED,
};

pub const TRAY_CLASS: &str = "frame0.tray.msg";
const TIP: &str = "Frame Zero – screenshot tool";

// Tray menu command ids.
const CMD_TAKE: u16 = 100;
const CMD_FULLSCREEN: u16 = 101;
const CMD_WINDOW: u16 = 102;
const CMD_FIXED: u16 = 103;
const CMD_SCROLL: u16 = 104;
const CMD_REPEAT: u16 = 105;
const CMD_CANCEL_DELAY: u16 = 106;
const CMD_OPEN_FILE: u16 = 110;
const CMD_PASTE: u16 = 111;
const CMD_SAVE_PRESET: u16 = 120;
const CMD_DELAY_OFF: u16 = 130;
const CMD_DELAY_3: u16 = 131;
const CMD_DELAY_5: u16 = 132;
const CMD_DELAY_10: u16 = 133;
const CMD_DELAY_CUSTOM: u16 = 134;
const CMD_CURSOR: u16 = 140;
const CMD_OUT_EDITOR: u16 = 150;
const CMD_OUT_CLIPBOARD: u16 = 151;
const CMD_OUT_FILE: u16 = 152;
const CMD_FX_BORDER: u16 = 160;
const CMD_FX_SHADOW: u16 = 161;
const CMD_INTERVAL_START: u16 = 170;
const CMD_INTERVAL_STOP: u16 = 171;
const CMD_HOTKEY: u16 = 180;
const CMD_STAMPS: u16 = 181;
const CMD_EXIT: u16 = 199;
const CMD_MONITOR: u16 = 200; // + index (< 64)
const CMD_OPEN_WINDOW: u16 = 300; // + index (< 300)
const CMD_PRESET: u16 = 600; // + index (< 100)
const CMD_PRESET_DEL: u16 = 700; // + index (< 100)

const TIMER_DELAY: usize = 1;
const TIMER_INTERVAL: usize = 2;

#[derive(Default)]
struct Runtime {
    /// A capture / prompt / overlay is in progress (nested modal loop).
    busy: bool,
    /// Delayed capture waiting for its countdown: (settings, reuse).
    pending: Option<(CaptureSettings, bool)>,
    countdown: u32,
    /// Running interval capture.
    interval: Option<CaptureSettings>,
    /// Window list captured when the tray menu was built.
    menu_windows: Vec<capture::windows::WindowEntry>,
    hotkey_ok: bool,
}

pub struct App {
    pub hwnd: HWND,
    pub tray: Tray,
    pub config: RefCell<Config>,
    pub gfx: Rc<Gfx>,
    pub editor: RefCell<Option<EditorWindow>>,
    rt: RefCell<Runtime>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

impl App {
    pub fn new() -> Result<Self> {
        let config = Config::load();

        let class = register_class(TRAY_CLASS, tray_wndproc, 0, 0)?;
        let class = Box::leak(Box::new(class));

        let hwnd = create_window(&WindowOpts {
            class,
            title: "Frame Zero",
            ex_style: 0,
            style: WS_OVERLAPPED,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            parent: HWND_MESSAGE,
            menu: 0,
            user_data: core::ptr::null_mut(),
        })?;

        let tray = Tray::install(hwnd, TIP)?;

        // A taken hotkey must not stop the app from starting.
        let hotkey_ok =
            crate::win::hotkey::register(hwnd, crate::win::hotkey::HOTKEY_ID_CAPTURE, config.hotkey).is_ok();
        if !hotkey_ok {
            let mut msg = String::from("The hotkey ");
            msg.push_str(&config.hotkey_str);
            msg.push_str(" is in use by another program. Choose another via Options ▸ Set hotkey…");
            tray.balloon("Frame Zero: hotkey unavailable", &msg, true);
        }

        let gfx = Rc::new(Gfx::create()?);

        Ok(Self {
            hwnd,
            tray,
            config: RefCell::new(config),
            gfx,
            editor: RefCell::new(None),
            rt: RefCell::new(Runtime { hotkey_ok, ..Default::default() }),
        })
    }

    pub fn editor_closed(self: &Rc<Self>) {
        *self.editor.borrow_mut() = None;
    }

    pub fn run(self) -> Result<()> {
        APP.with(|cell| *cell.borrow_mut() = Some(Rc::new(self)));
        message_loop();
        APP.with(|cell| cell.borrow_mut().take());
        crate::win::com::uninit_apartment();
        Ok(())
    }

    fn busy(&self) -> bool {
        self.rt.borrow().busy
    }

    fn set_busy(&self, b: bool) {
        self.rt.borrow_mut().busy = b;
    }

    /// Settings for a new capture of `mode`, using the current options.
    fn settings(&self, mode: Mode) -> CaptureSettings {
        let c = self.config.borrow();
        let mut s = CaptureSettings::new(mode);
        s.delay = c.delay;
        s.include_cursor = c.include_cursor;
        s.output = c.output;
        s.effects = c.effects;
        s
    }

    fn on_tray_event(self: &Rc<Self>, lparam: LPARAM) {
        if tray::is_double_click(lparam) {
            self.on_command(CMD_TAKE);
        } else if tray::is_right_click(lparam) && !self.busy() {
            let menu = self.build_menu();
            let cmd = menu.track(self.hwnd);
            drop(menu);
            if cmd != 0 {
                self.on_command(cmd);
            }
        }
    }

    // ---------------------------------------------------------------- menu

    fn build_menu(&self) -> Menu {
        let c = self.config.borrow();
        let rt_pending = self.rt.borrow().pending.is_some();
        let interval_on = self.rt.borrow().interval.is_some();
        let m = Menu::new();

        if rt_pending {
            m.item(CMD_CANCEL_DELAY, "Cancel delayed capture").sep();
        }
        let mut take = String::from("&Take screenshot\t");
        if self.rt.borrow().hotkey_ok {
            take.push_str(&c.hotkey_str);
        } else {
            take.push_str("(no hotkey)");
        }
        m.item(CMD_TAKE, &take);

        // Capture ▸
        let cap = Menu::new();
        cap.item(CMD_FULLSCREEN, "&Full screen (all monitors)");
        let mons = capture::monitors::list();
        if mons.len() > 1 {
            cap.sep();
            for (i, mon) in mons.iter().enumerate().take(64) {
                let mut t = String::from("Monitor &");
                t.push_str(&(i + 1).to_string());
                t.push_str(":  ");
                t.push_str(&mon.w.to_string());
                t.push_str(" × ");
                t.push_str(&mon.h.to_string());
                if mon.primary {
                    t.push_str("  (primary)");
                }
                cap.item(CMD_MONITOR + i as u16, &t);
            }
        }
        cap.sep();
        cap.item(CMD_WINDOW, "&Window");
        let wins = capture::windows::list(0);
        let wm = Menu::new();
        for (i, e) in wins.iter().enumerate().take(300) {
            let mut t = tray::menu_label(&e.title, 60);
            if e.minimized {
                t.push_str("  (minimized)");
            }
            wm.item(CMD_OPEN_WINDOW + i as u16, &t);
        }
        cap.sub("&Open window", wm, !wins.is_empty());
        self.rt.borrow_mut().menu_windows = wins;
        cap.sep();
        cap.item(CMD_FIXED, "Fi&xed region…");
        cap.item(CMD_SCROLL, "&Scrolling capture…");
        m.sub("&Capture", cap, true);
        m.item_flags(CMD_REPEAT, "&Repeat last capture", c.last.is_some(), false);

        // Edit screenshot ▸
        let ed = Menu::new();
        ed.item(CMD_OPEN_FILE, "&Open image or project file…");
        ed.item_flags(CMD_PASTE, "&Paste from clipboard", clipboard_has_image(), false);
        m.sub("&Edit screenshot", ed, true);

        // Presets ▸
        let presets = c.presets();
        let pm = Menu::new();
        let del = Menu::new();
        if presets.is_empty() {
            pm.item_flags(0, "(no presets)", false, false);
        }
        for (i, (name, _)) in presets.iter().enumerate().take(100) {
            let label = tray::menu_label(name, 60);
            pm.item(CMD_PRESET + i as u16, &label);
            del.item(CMD_PRESET_DEL + i as u16, &label);
        }
        pm.sep();
        pm.item_flags(CMD_SAVE_PRESET, "&Save last capture as preset…", c.last.is_some(), false);
        pm.sub("&Delete preset", del, !presets.is_empty());
        m.sub("&Presets", pm, true);

        // Options ▸
        let om = Menu::new();
        let dm = Menu::new();
        dm.item(CMD_DELAY_OFF, "&Off")
            .item(CMD_DELAY_3, "&3 seconds")
            .item(CMD_DELAY_5, "&5 seconds")
            .item(CMD_DELAY_10, "1&0 seconds");
        let delay_cmd = match c.delay {
            0 => CMD_DELAY_OFF,
            3 => CMD_DELAY_3,
            5 => CMD_DELAY_5,
            10 => CMD_DELAY_10,
            _ => CMD_DELAY_CUSTOM,
        };
        let mut custom = String::from("&Custom");
        if delay_cmd == CMD_DELAY_CUSTOM {
            custom.push_str(" (");
            custom.push_str(&c.delay.to_string());
            custom.push_str(" s)");
        }
        custom.push('…');
        dm.item(CMD_DELAY_CUSTOM, &custom);
        dm.radio(CMD_DELAY_OFF, CMD_DELAY_CUSTOM, delay_cmd);
        om.sub("&Delay", dm, true);
        om.item_flags(CMD_CURSOR, "Include &cursor", true, c.include_cursor);
        let am = Menu::new();
        am.item(CMD_OUT_EDITOR, "Open &editor")
            .item(CMD_OUT_CLIPBOARD, "Copy to &clipboard")
            .item(CMD_OUT_FILE, "Save to &file (Pictures)");
        let out_cmd = match c.output {
            Output::Editor => CMD_OUT_EDITOR,
            Output::Clipboard => CMD_OUT_CLIPBOARD,
            Output::File => CMD_OUT_FILE,
        };
        am.radio(CMD_OUT_EDITOR, CMD_OUT_FILE, out_cmd);
        om.sub("&After capture", am, true);
        let fm = Menu::new();
        fm.item_flags(CMD_FX_BORDER, "&Border", true, c.effects & FX_BORDER != 0);
        fm.item_flags(CMD_FX_SHADOW, "Drop &shadow", true, c.effects & FX_SHADOW != 0);
        om.sub("E&ffects", fm, true);
        let im = Menu::new();
        let mut start = String::from("&Start every ");
        start.push_str(&c.interval.to_string());
        start.push_str(" s…");
        im.item_flags(CMD_INTERVAL_START, &start, !interval_on, false);
        im.item_flags(CMD_INTERVAL_STOP, "S&top", interval_on, false);
        om.sub(if interval_on { "&Interval capture (running)" } else { "&Interval capture" }, im, true);
        om.sep();
        om.item(CMD_HOTKEY, "Set &hotkey…");
        om.item(CMD_STAMPS, "Open &stamps folder");
        m.sub("&Options", om, true);

        m.sep();
        m.item(CMD_EXIT, "E&xit");
        m
    }

    fn on_command(self: &Rc<Self>, cmd: u16) {
        if cmd == CMD_EXIT {
            post_quit(0);
            return;
        }
        if self.busy() {
            return;
        }
        match cmd {
            CMD_TAKE => self.run_capture(self.settings(Mode::Region)),
            CMD_FULLSCREEN => self.run_capture(self.settings(Mode::Fullscreen)),
            CMD_WINDOW => self.run_capture(self.settings(Mode::Window)),
            CMD_SCROLL => self.run_capture(self.settings(Mode::Scrolling)),
            CMD_FIXED => self.fixed_region(),
            CMD_REPEAT => {
                let last = self.config.borrow().last.clone();
                if let Some(s) = last {
                    self.start(s, true);
                }
            }
            CMD_CANCEL_DELAY => self.cancel_delay(),
            CMD_OPEN_FILE => self.open_file(),
            CMD_PASTE => match crate::win::clipboard::read_image(self.hwnd) {
                Some(bmp) => self.open_editor(bmp),
                None => message(self.hwnd, "The clipboard does not contain an image."),
            },
            CMD_SAVE_PRESET => self.save_preset(),
            CMD_DELAY_OFF | CMD_DELAY_3 | CMD_DELAY_5 | CMD_DELAY_10 => {
                let d = [0, 3, 5, 10][(cmd - CMD_DELAY_OFF) as usize];
                self.update_options(|c| c.delay = d);
            }
            CMD_DELAY_CUSTOM => {
                let cur = self.config.borrow().delay.to_string();
                if let Some(v) = self.ask("Capture delay", &[("Delay (seconds)", &cur)]) {
                    if let Ok(d) = v[0].parse::<u32>() {
                        self.update_options(|c| c.delay = d.min(3600));
                    }
                }
            }
            CMD_CURSOR => self.update_options(|c| c.include_cursor = !c.include_cursor),
            CMD_OUT_EDITOR => self.update_options(|c| c.output = Output::Editor),
            CMD_OUT_CLIPBOARD => self.update_options(|c| c.output = Output::Clipboard),
            CMD_OUT_FILE => self.update_options(|c| c.output = Output::File),
            CMD_FX_BORDER => self.update_options(|c| c.effects ^= FX_BORDER),
            CMD_FX_SHADOW => self.update_options(|c| c.effects ^= FX_SHADOW),
            CMD_INTERVAL_START => self.interval_prompt(),
            CMD_INTERVAL_STOP => self.stop_interval(),
            CMD_HOTKEY => self.set_hotkey(),
            CMD_STAMPS => open_stamps_folder(),
            c if (CMD_MONITOR..CMD_MONITOR + 64).contains(&c) => {
                let mut s = self.settings(Mode::Monitor);
                s.monitor = (c - CMD_MONITOR) as u32;
                self.run_capture(s);
            }
            c if (CMD_OPEN_WINDOW..CMD_PRESET).contains(&c) => {
                let e = self.rt.borrow().menu_windows.get((c - CMD_OPEN_WINDOW) as usize).cloned();
                if let Some(e) = e {
                    let mut s = self.settings(Mode::WindowHandle);
                    s.hwnd = e.hwnd;
                    self.run_capture(s);
                }
            }
            c if (CMD_PRESET..CMD_PRESET + 100).contains(&c) => {
                let p = self.config.borrow().presets().into_iter().nth((c - CMD_PRESET) as usize);
                if let Some((_, s)) = p {
                    self.start(s, true);
                }
            }
            c if (CMD_PRESET_DEL..CMD_PRESET_DEL + 100).contains(&c) => {
                self.config.borrow().delete_preset((c - CMD_PRESET_DEL) as usize);
            }
            _ => {}
        }
    }

    fn update_options(&self, f: impl FnOnce(&mut Config)) {
        let mut c = self.config.borrow_mut();
        f(&mut c);
        c.save_options();
    }

    /// Modal prompt with the app marked busy.
    fn ask(&self, title: &str, fields: &[(&str, &str)]) -> Option<Vec<String>> {
        self.set_busy(true);
        let r = crate::win::prompt::ask(0, title, fields);
        self.set_busy(false);
        r
    }

    // ------------------------------------------------------------- capture

    /// Execute any capture: delay → resolve (overlay / window / rect) →
    /// record as last capture → effects → output routing.
    pub fn run_capture(self: &Rc<Self>, s: CaptureSettings) {
        self.start(s, false);
    }

    /// `reuse`: reuse the resolved rect / window stored in `s` instead of
    /// asking interactively (repeat last, presets, interval).
    fn start(self: &Rc<Self>, s: CaptureSettings, reuse: bool) {
        {
            let mut rt = self.rt.borrow_mut();
            if rt.busy || rt.pending.is_some() {
                return;
            }
            if s.delay > 0 {
                rt.countdown = s.delay;
                rt.pending = Some((s, reuse));
                drop(rt);
                self.countdown_tip();
                unsafe { SetTimer(self.hwnd, TIMER_DELAY, 1000, None) };
                return;
            }
        }
        self.execute(s, reuse, false);
    }

    fn countdown_tip(&self) {
        let mut t = String::from("Frame Zero – capturing in ");
        t.push_str(&self.rt.borrow().countdown.to_string());
        t.push_str(" s");
        self.tray.modify_tip(&t);
    }

    fn cancel_delay(&self) {
        unsafe { KillTimer(self.hwnd, TIMER_DELAY) };
        self.rt.borrow_mut().pending = None;
        self.restore_tip();
    }

    fn restore_tip(&self) {
        let iv = self.rt.borrow().interval.as_ref().map(|s| s.interval);
        match iv {
            Some(n) => {
                let mut t = String::from("Frame Zero – interval capture every ");
                t.push_str(&n.to_string());
                t.push_str(" s");
                self.tray.modify_tip(&t);
            }
            None => self.tray.modify_tip(TIP),
        }
    }

    fn on_timer(self: &Rc<Self>, id: usize) {
        match id {
            TIMER_DELAY => {
                let fire = {
                    let mut rt = self.rt.borrow_mut();
                    rt.countdown = rt.countdown.saturating_sub(1);
                    if rt.countdown == 0 { rt.pending.take() } else { None }
                };
                if let Some((s, reuse)) = fire {
                    unsafe { KillTimer(self.hwnd, TIMER_DELAY) };
                    self.restore_tip();
                    self.execute(s, reuse, false);
                } else if self.rt.borrow().pending.is_some() {
                    self.countdown_tip();
                } else {
                    unsafe { KillTimer(self.hwnd, TIMER_DELAY) };
                }
            }
            TIMER_INTERVAL => {
                let s = self.rt.borrow().interval.clone();
                if let Some(s) = s {
                    if !self.busy() && self.rt.borrow().pending.is_none() {
                        self.execute(s, true, true);
                    }
                }
            }
            _ => {}
        }
    }

    /// `from_interval`: a timer tick — don't record as last capture, don't
    /// notify, don't (re)start the interval.
    fn execute(self: &Rc<Self>, mut s: CaptureSettings, reuse: bool, from_interval: bool) {
        self.set_busy(true);
        let bmp = self.grab(&mut s, reuse);
        self.set_busy(false);
        let Some(bmp) = bmp else { return };
        if !from_interval {
            self.config.borrow_mut().set_last(&s);
            if s.interval > 0 {
                self.start_interval(s.clone());
            }
        }
        let bmp = capture::effects::apply(bmp, s.effects);
        self.output(bmp, s.output, from_interval);
    }

    /// Capture according to `s`, filling in the resolved rect / window.
    fn grab(&self, s: &mut CaptureSettings, reuse: bool) -> Option<Bitmap> {
        use capture::bitblt;
        let cur = s.include_cursor;
        let virt = capture::monitors::virtual_screen_bounds();
        let live = |r: ScreenRect| bitblt::region(r.0, r.1, r.2, r.3, cur).ok();
        match s.mode {
            Mode::Fullscreen => {
                s.region = Some(virt);
                live(virt)
            }
            Mode::Monitor => {
                let mons = capture::monitors::list();
                let m = mons.get(s.monitor as usize).or_else(|| mons.iter().find(|m| m.primary))?;
                let r = (m.x, m.y, m.w, m.h);
                s.region = Some(r);
                live(r)
            }
            Mode::WindowHandle => self.grab_window(s).or_else(|| s.region.and_then(live)),
            Mode::Window if reuse && s.hwnd != 0 && unsafe { IsWindow(s.hwnd) } != 0 => self.grab_window(s),
            Mode::FixedRegion if !reuse && s.fixed_pos.is_some() && s.fixed_size.is_some() => {
                let ((x, y), (w, h)) = (s.fixed_pos?, s.fixed_size?);
                s.region = Some((x, y, w, h));
                live((x, y, w, h))
            }
            Mode::Scrolling if reuse && s.region.is_some() => {
                let r = s.region?;
                capture::scrolling::capture(r).ok()
            }
            _ if reuse && s.region.is_some() => live(s.region?),
            _ => self.grab_interactive(s, virt),
        }
    }

    fn grab_window(&self, s: &mut CaptureSettings) -> Option<Bitmap> {
        let fb = capture::windows::frame_bounds(s.hwnd);
        s.region = Some((fb.left, fb.top, fb.right - fb.left, fb.bottom - fb.top));
        let mut bmp = capture::print_window::capture(s.hwnd).ok()?;
        if s.include_cursor {
            if let Some(c) = capture::bitblt::snap_cursor() {
                capture::bitblt::draw_cursor(&mut bmp, (fb.left, fb.top), &c);
            }
        }
        Some(bmp)
    }

    /// Snapshot the screen, let the user pick on the frozen image, crop.
    fn grab_interactive(&self, s: &mut CaptureSettings, virt: ScreenRect) -> Option<Bitmap> {
        use crate::overlay::region::{self, Picked};
        let cursor = if s.include_cursor { capture::bitblt::snap_cursor() } else { None };
        let bg = capture::bitblt::virtual_screen(false).ok()?;
        let fixed = if s.mode == Mode::FixedRegion { s.fixed_size } else { None };
        let mut window = s.mode == Mode::Window;
        let mut rect = None;
        if !window {
            match region::pick(&bg, virt, fixed)? {
                Picked::Rect(r) => rect = Some(r),
                Picked::Window => window = true,
            }
        }
        if window {
            let (hwnd, r) = crate::overlay::window_pick::pick(&bg, virt)?;
            s.hwnd = hwnd;
            if s.mode != Mode::Scrolling {
                s.mode = Mode::Window;
            }
            rect = Some(r);
        }
        let r = rect?;
        s.region = Some(r);
        if s.mode == Mode::Scrolling {
            // Let the overlay disappear before grabbing live frames.
            crate::win::window::sleep_ms(120);
            return capture::scrolling::capture(r).ok();
        }
        let mut bmp = capture::bitblt::crop(&bg, (virt.0, virt.1), r)?;
        if let Some(c) = cursor {
            capture::bitblt::draw_cursor(&mut bmp, (r.0.max(virt.0), r.1.max(virt.1)), &c);
        }
        Some(bmp)
    }

    fn fixed_region(self: &Rc<Self>) {
        let (fw, fh) = self.config.borrow().last_fixed;
        let (ws, hs) = (fw.to_string(), fh.to_string());
        let Some(v) = self.ask(
            "Fixed region",
            &[("Width", &ws), ("Height", &hs), ("X (optional)", ""), ("Y (optional)", "")],
        ) else {
            return;
        };
        let (Ok(w), Ok(h)) = (v[0].parse::<i32>(), v[1].parse::<i32>()) else {
            message(self.hwnd, "Width and height must be whole numbers.");
            return;
        };
        if w < 1 || h < 1 {
            return;
        }
        self.update_options(|c| c.last_fixed = (w, h));
        let mut s = self.settings(Mode::FixedRegion);
        s.fixed_size = Some((w, h));
        if let (Ok(x), Ok(y)) = (v[2].parse::<i32>(), v[3].parse::<i32>()) {
            s.fixed_pos = Some((x, y));
        }
        self.run_capture(s);
    }

    // ------------------------------------------------------------- output

    fn output(self: &Rc<Self>, bmp: Bitmap, out: Output, quiet: bool) {
        match out {
            Output::Editor => self.open_editor(bmp),
            Output::Clipboard => {
                let png = crate::gfx::gdiplus::encode_png_bytes(&bmp).unwrap_or_default();
                if crate::win::clipboard::write_image(self.hwnd, &bmp, &png).is_ok() && !quiet {
                    self.tray.balloon("Frame Zero", "Screenshot copied to the clipboard.", false);
                }
            }
            Output::File => {
                let path = build_save_path();
                match crate::gfx::gdiplus::save_png(&bmp, &path) {
                    Ok(()) if !quiet => {
                        let mut t = String::from("Saved ");
                        t.push_str(&path);
                        self.tray.balloon("Frame Zero", &t, false);
                    }
                    Ok(()) => {}
                    Err(_) => self.tray.balloon("Frame Zero", "Could not save the screenshot.", true),
                }
            }
        }
    }

    pub fn open_editor(self: &Rc<Self>, bmp: Bitmap) {
        let mut slot = self.editor.borrow_mut();
        if let Some(existing) = slot.as_mut() {
            existing.replace_image(bmp);
            existing.activate();
            return;
        }
        if let Ok(ed) = EditorWindow::create(self.clone(), bmp) {
            *slot = Some(ed);
        }
    }

    pub fn open_document(self: &Rc<Self>, doc: Document) {
        let mut slot = self.editor.borrow_mut();
        if let Some(existing) = slot.as_mut() {
            existing.replace_document(doc);
            existing.activate();
            return;
        }
        if let Ok(ed) = EditorWindow::create_with_doc(self.clone(), doc) {
            *slot = Some(ed);
        }
    }

    fn open_file(self: &Rc<Self>) {
        self.set_busy(true);
        let path = crate::win::dialog::open_file(
            0,
            "Open image or project",
            "Images and projects|*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.frame0|All files|*.*",
        );
        self.set_busy(false);
        let Ok(Some(path)) = path else { return };
        if path.to_ascii_lowercase().ends_with(".frame0") {
            match crate::io::project::load(&path) {
                Ok(doc) => self.open_document(doc),
                Err(_) => message(self.hwnd, "Could not open the project file."),
            }
        } else {
            match crate::gfx::gdiplus::decode_file(&path) {
                Ok(bmp) => self.open_editor(bmp),
                Err(_) => message(self.hwnd, "Could not open the image."),
            }
        }
    }

    // ------------------------------------------------- presets / interval

    fn save_preset(&self) {
        let last = self.config.borrow().last.clone();
        let Some(last) = last else { return };
        let mut name = String::from(match last.mode {
            Mode::Fullscreen => "Full screen",
            Mode::Monitor => "Monitor",
            Mode::Region | Mode::FixedRegion => "Region",
            Mode::Window | Mode::WindowHandle => "Window",
            Mode::Scrolling => "Scrolling",
        });
        if let Some((x, y, w, h)) = last.region.filter(|_| matches!(last.mode, Mode::Region | Mode::FixedRegion)) {
            for (sep, v) in [(" ", w), ("×", h), (" at ", x), (",", y)] {
                name.push_str(sep);
                name.push_str(&v.to_string());
            }
        }
        if let Some(v) = self.ask("Save preset", &[("Preset name", &name)]) {
            if !v[0].is_empty() {
                self.config.borrow().add_preset(&v[0], &last.to_preset());
            }
        }
    }

    fn interval_prompt(&self) {
        let cur = self.config.borrow().interval.to_string();
        let Some(v) = self.ask("Interval capture", &[("Every (seconds)", &cur)]) else { return };
        let Ok(n) = v[0].parse::<u32>() else { return };
        let n = n.clamp(1, 86_400);
        self.update_options(|c| c.interval = n);
        let mut s = self.config.borrow().last.clone().unwrap_or_else(|| self.settings(Mode::Fullscreen));
        if !matches!(s.mode, Mode::Fullscreen | Mode::Monitor | Mode::WindowHandle) && s.region.is_none() {
            s = self.settings(Mode::Fullscreen);
        }
        s.interval = n;
        s.delay = 0;
        s.output = Output::File;
        self.start_interval(s);
    }

    fn start_interval(&self, s: CaptureSettings) {
        unsafe { SetTimer(self.hwnd, TIMER_INTERVAL, s.interval.max(1) * 1000, None) };
        self.rt.borrow_mut().interval = Some(s);
        self.restore_tip();
    }

    fn stop_interval(&self) {
        unsafe { KillTimer(self.hwnd, TIMER_INTERVAL) };
        self.rt.borrow_mut().interval = None;
        self.restore_tip();
    }

    // -------------------------------------------------------------- hotkey

    fn set_hotkey(&self) {
        use crate::win::hotkey::{self, HOTKEY_ID_CAPTURE};
        let cur = self.config.borrow().hotkey_str.clone();
        let Some(v) = self.ask("Set hotkey", &[("Hotkey (e.g. Ctrl+Win+PgDn)", &cur)]) else { return };
        let Some(spec) = hotkey::parse(&v[0]) else {
            message(self.hwnd, "Unrecognised hotkey. Use e.g. Ctrl+Shift+S, Ctrl+Win+PgDn or Alt+F9.");
            return;
        };
        hotkey::unregister(self.hwnd, HOTKEY_ID_CAPTURE);
        if hotkey::register(self.hwnd, HOTKEY_ID_CAPTURE, spec).is_ok() {
            self.rt.borrow_mut().hotkey_ok = true;
            self.update_options(|c| {
                c.hotkey = spec;
                c.hotkey_str = v[0].clone();
            });
        } else {
            let old = self.config.borrow().hotkey;
            let ok = hotkey::register(self.hwnd, HOTKEY_ID_CAPTURE, old).is_ok();
            self.rt.borrow_mut().hotkey_ok = ok;
            message(self.hwnd, "That hotkey is already in use by another program.");
        }
    }
}

fn clipboard_has_image() -> bool {
    use windows_sys::Win32::System::DataExchange::{IsClipboardFormatAvailable, RegisterClipboardFormatW};
    use windows_sys::Win32::System::Ole::CF_DIB;
    unsafe {
        let png = RegisterClipboardFormatW(wide("PNG").as_ptr());
        IsClipboardFormatAvailable(CF_DIB as u32) != 0 || (png != 0 && IsClipboardFormatAvailable(png) != 0)
    }
}

fn message(_owner: HWND, text: &str) {
    unsafe {
        MessageBoxW(
            0,
            wide(text).as_ptr(),
            wide("Frame Zero").as_ptr(),
            MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND,
        );
    }
}

fn open_stamps_folder() {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let mut dir = crate::config::app_dir();
    dir.push_str("\\stamps");
    let _ = std::fs::create_dir_all(&dir);
    unsafe {
        ShellExecuteW(
            0,
            wide("open").as_ptr(),
            wide(&dir).as_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// `%USERPROFILE%\Pictures\frame0-YYYY-MM-DD_HH-MM-SS[-n].png`
fn build_save_path() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = unsafe { core::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    let dir = std::env::var("USERPROFILE")
        .map(|h| format!("{}\\Pictures", h))
        .unwrap_or_else(|_| ".".to_string());
    let _ = std::fs::create_dir_all(&dir);
    let base = format!(
        "{}\\frame0-{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        dir, t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    );
    let mut path = format!("{}.png", base);
    let mut n = 2;
    while std::path::Path::new(&path).exists() {
        path = format!("{}-{}.png", base, n);
        n += 1;
    }
    path
}

impl Drop for App {
    fn drop(&mut self) {
        crate::win::hotkey::unregister(self.hwnd, crate::win::hotkey::HOTKEY_ID_CAPTURE);
    }
}

unsafe extern "system" fn tray_wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let app = APP.with(|cell| cell.borrow().clone());
    if let Some(app) = app {
        match msg {
            WM_HOTKEY => {
                if w as i32 == crate::win::hotkey::HOTKEY_ID_CAPTURE {
                    app.on_command(CMD_TAKE);
                }
                return 0;
            }
            tray::WM_TRAY => {
                app.on_tray_event(l);
                return 0;
            }
            WM_COMMAND => {
                app.on_command((w & 0xFFFF) as u16);
                return 0;
            }
            WM_TIMER => {
                app.on_timer(w);
                return 0;
            }
            WM_DESTROY => {
                post_quit(0);
                return 0;
            }
            _ => {
                if msg == app.tray.taskbar_created_msg {
                    app.tray.re_add();
                    return 0;
                }
            }
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// `frame0.exe --cmd <id> [--pid <pid>]`: post a tray command to a running
/// instance (scripting / testing). Returns true if handled.
pub fn forward_command() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId, PostMessageW};
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let Some(cmd) = arg("--cmd").and_then(|c| c.parse::<usize>().ok()) else { return false };
    let pid: u32 = arg("--pid").and_then(|p| p.parse().ok()).unwrap_or(0);
    let class = wide(TRAY_CLASS);
    let mut h: HWND = 0;
    unsafe {
        loop {
            h = FindWindowExW(HWND_MESSAGE, h, class.as_ptr(), core::ptr::null());
            if h == 0 {
                break;
            }
            let mut p = 0u32;
            GetWindowThreadProcessId(h, &mut p);
            if pid == 0 || p == pid {
                PostMessageW(h, WM_COMMAND, cmd, 0);
                break;
            }
        }
    }
    true
}
