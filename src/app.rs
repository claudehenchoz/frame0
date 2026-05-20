use crate::config::Config;
use crate::editor::EditorWindow;
use crate::error::{Error, Result};
use crate::gfx::Gfx;
use crate::win::tray::{self, Tray};
use crate::win::wide;
use crate::win::window::{WindowOpts, create_window, message_loop, post_quit, register_class};
use std::cell::RefCell;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, HWND_MESSAGE, WM_COMMAND, WM_DESTROY, WM_HOTKEY, WS_OVERLAPPED,
};

pub struct App {
    pub hwnd: HWND,
    pub tray: Tray,
    pub config: Config,
    pub gfx: Rc<Gfx>,
    pub editor: RefCell<Option<EditorWindow>>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

impl App {
    pub fn new() -> Result<Self> {
        let config = Config::load();

        let class = register_class("frame.tray.msg", tray_wndproc, 0, 0)?;
        let class = Box::leak(Box::new(class));

        let hwnd = create_window(&WindowOpts {
            class,
            title: "frame",
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

        let tray = Tray::install(hwnd, "frame – screenshot tool")?;

        crate::win::hotkey::register(hwnd, crate::win::hotkey::HOTKEY_ID_CAPTURE, config.hotkey)
            .map_err(|_| Error::msg("Failed to register global hotkey"))?;

        let gfx = Rc::new(Gfx::create()?);

        Ok(Self {
            hwnd,
            tray,
            config,
            gfx,
            editor: RefCell::new(None),
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

    fn on_hotkey(self: &Rc<Self>) {
        self.capture_and_save();
    }

    fn on_tray_event(self: &Rc<Self>, lparam: LPARAM) {
        if tray::is_double_click(lparam) {
            self.capture_and_save();
            return;
        }
        if tray::is_right_click(lparam) {
            let cmd = unsafe { tray::show_context_menu(self.hwnd) };
            self.on_command(cmd);
        }
    }

    fn on_command(self: &Rc<Self>, cmd: u16) {
        match cmd {
            tray::CMD_TAKE | tray::CMD_REPEAT_LAST => self.capture_and_save(),
            tray::CMD_EXIT => post_quit(0),
            _ => {}
        }
    }

    fn capture_and_save(self: &Rc<Self>) {
        let bg = match crate::capture::bitblt::virtual_screen(false) {
            Ok(b) => b,
            Err(_) => return,
        };
        let virt = crate::capture::monitors::virtual_screen_bounds();
        let chosen = crate::overlay::region::pick(&bg, virt).ok().flatten();
        let Some((x, y, w, h)) = chosen else { return };
        let Ok(shot) = crate::capture::bitblt::region(x, y, w, h, self.config.include_cursor) else {
            return;
        };
        self.open_editor(shot);
    }

    pub fn open_editor(self: &Rc<Self>, bmp: crate::gfx::bitmap::Bitmap) {
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
}

fn build_save_path() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dir = std::env::var("USERPROFILE")
        .map(|h| format!("{}\\Pictures", h))
        .unwrap_or_else(|_| ".".to_string());
    let _ = std::fs::create_dir_all(&dir);
    format!("{}\\frame-{}.png", dir, ts)
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
                    app.on_hotkey();
                }
                return 0;
            }
            tray::WM_TRAY => {
                app.on_tray_event(l);
                return 0;
            }
            WM_COMMAND => {
                let cmd = (w & 0xFFFF) as u16;
                app.on_command(cmd);
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
