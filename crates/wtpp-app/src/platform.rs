//! Small OS integrations.

/// Open a file or URL with the system's default handler.
pub fn open(target: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // `start` resolves file associations and URL protocols (and shows
        // the "open with" dialog when there is no association).
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(target)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(target).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(target).spawn().map(|_| ())
    }
}

/// Print a line to the shell that started wt++ (`wtpp --version`). On Windows
/// wt++ is a GUI app without a console of its own, so it writes to its
/// parent's console unless stdout is redirected (`wtpp -v > file`, a pipe).
pub fn print_line(text: &str) {
    #[cfg(windows)]
    {
        use std::io::Write;
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
        // SAFETY: no preconditions.
        let stdout = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        // SAFETY: no preconditions; fails harmlessly without a parent console.
        if (stdout.is_null() || stdout == INVALID_HANDLE_VALUE)
            && unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0
            && let Ok(mut console) = std::fs::OpenOptions::new().write(true).open("CONOUT$")
        {
            // The shell doesn't wait for GUI apps and has printed its next
            // prompt already: start on a fresh line.
            let _ = writeln!(console, "\r\n{text}");
            return;
        }
    }
    println!("{text}");
}

/// The terminal bell: the system's default sound.
pub fn bell() {
    #[cfg(windows)]
    // SAFETY: MessageBeep has no preconditions.
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(
            windows_sys::Win32::UI::WindowsAndMessaging::MB_OK,
        );
    }
}

/// Make native popup menus (tab right-click menu) follow the app theme.
/// Uses uxtheme's `SetPreferredAppMode` (ordinal 135) and `FlushMenuThemes`
/// (136): undocumented, but stable since Windows 10 1903 and what Explorer
/// and Windows Terminal rely on. Missing → no-op.
pub fn native_menu_theme(dark: bool) {
    #[cfg(windows)]
    // SAFETY: the ordinals have these signatures on every Windows version
    // that exports them; we check they exist before calling.
    unsafe {
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
        let name: Vec<u16> = "uxtheme.dll\0".encode_utf16().collect();
        let lib = LoadLibraryW(name.as_ptr());
        if lib.is_null() {
            return;
        }
        const FORCE_DARK: i32 = 2;
        const FORCE_LIGHT: i32 = 3;
        if let Some(set_mode) = GetProcAddress(lib, 135 as *const u8) {
            let set_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            set_mode(if dark { FORCE_DARK } else { FORCE_LIGHT });
        }
        if let Some(flush) = GetProcAddress(lib, 136 as *const u8) {
            let flush: extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
    }
    #[cfg(not(windows))]
    let _ = dark;
}

/// Whether the system asks apps for a light theme (Windows "app mode").
pub fn system_prefers_light() -> bool {
    #[cfg(windows)]
    // SAFETY: reads one DWORD registry value into a correctly sized buffer.
    unsafe {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let value = wide("AppsUseLightTheme");
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        );
        status == 0 && data == 1
    }
    #[cfg(not(windows))]
    false
}

/// The icon embedded in an executable, as RGBA pixels (Windows only).
/// `program` may be a bare name (`cmd.exe`): it's looked up like the shell
/// would, in PATH and System32.
pub fn exe_icon(program: &str) -> Option<slint::Image> {
    #[cfg(windows)]
    {
        imp::exe_icon(program)
    }
    #[cfg(not(windows))]
    {
        let _ = program;
        None
    }
}

/// The US-layout character of the letter, digit or punctuation key held
/// down right now, unshifted. Lets shortcuts like Ctrl+Shift+W work while a
/// Hebrew or Russian layout is active (Windows switches layouts on Alt+Shift,
/// which every pane shortcut uses). Windows only; elsewhere `None`.
pub fn physical_key_char() -> Option<char> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
        const OEM: [(i32, char); 11] = [
            (0xBA, ';'), (0xBB, '='), (0xBC, ','), (0xBD, '-'), (0xBE, '.'), (0xBF, '/'),
            (0xC0, '`'), (0xDB, '['), (0xDC, '\\'), (0xDD, ']'), (0xDE, '\''),
        ];
        let letters = (b'A'..=b'Z').map(|v| (v as i32, (v as char).to_ascii_lowercase()));
        let digits = (b'0'..=b'9').map(|v| (v as i32, v as char));
        // SAFETY: GetKeyState has no preconditions. It reports the state as
        // of the key message being handled, which is the one we're in.
        letters.chain(digits).chain(OEM).find(|&(vk, _)| unsafe { GetKeyState(vk) } < 0).map(|(_, c)| c)
    }
    #[cfg(not(windows))]
    None
}

/// Whether a process named `exe` (e.g. `claude.exe`, any case) runs under
/// process `root`: `root` itself or any of its descendants. Windows only;
/// elsewhere `false`.
pub fn runs_under(root: u32, exe: &str) -> bool {
    #[cfg(windows)]
    {
        imp::runs_under(root, exe)
    }
    #[cfg(not(windows))]
    {
        let _ = (root, exe);
        false
    }
}

/// Whether process `pid` has started a process that's still running. Windows
/// only; elsewhere `false`.
pub fn has_children(pid: u32) -> bool {
    #[cfg(windows)]
    {
        imp::processes().iter().any(|&(_, parent, _)| parent == pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

/// The mouse pointer in screen coordinates (physical pixels, as `window`'s
/// position is measured), where the OS tells us: not on Wayland, where
/// callers work it out from the window position.
pub fn cursor_pos(window: &slint::Window) -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        let _ = window;
        use windows_sys::Win32::Foundation::POINT;
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: `p` is a valid out pointer.
        (unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p) } != 0).then_some((p.x, p.y))
    }
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct CGPoint {
            x: f64,
            y: f64,
        }
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGEventCreate(source: *const std::ffi::c_void) -> *const std::ffi::c_void;
            fn CGEventGetLocation(event: *const std::ffi::c_void) -> CGPoint;
            fn CFRelease(cf: *const std::ffi::c_void);
        }
        // Points from the main screen's top-left, as winit measures window
        // positions (before scaling them to pixels).
        // SAFETY: a null source is allowed; the event is released once.
        let p = unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let p = CGEventGetLocation(event);
            CFRelease(event);
            p
        };
        let scale = window.scale_factor() as f64;
        Some(((p.x * scale).round() as i32, (p.y * scale).round() as i32))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use slint::winit_030::WinitWindowAccessor;
        use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use x11rb::protocol::xproto::ConnectionExt;
        let x11 = window
            .with_winit_window(|w| {
                w.window_handle().is_ok_and(|h| matches!(h.as_raw(), RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)))
            })
            .unwrap_or(false);
        if !x11 {
            return None;
        }
        thread_local! {
            static X11: std::cell::OnceCell<Option<(x11rb::rust_connection::RustConnection, u32)>> =
                const { std::cell::OnceCell::new() };
        }
        X11.with(|x| {
            let (conn, root) = x
                .get_or_init(|| {
                    let (conn, screen) = x11rb::connect(None).ok()?;
                    let root = x11rb::connection::Connection::setup(&conn).roots.get(screen)?.root;
                    Some((conn, root))
                })
                .as_ref()?;
            let reply = conn.query_pointer(*root).ok()?.reply().ok()?;
            Some((reply.root_x as i32, reply.root_y as i32))
        })
    }
}

/// The usable area (without the taskbar) of the monitor at screen point
/// (`x`, `y`): left, top, right, bottom in physical pixels. Windows only.
pub fn work_area_at(x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
        // SAFETY: plain value arguments; `info` is a valid out pointer with
        // its size set.
        unsafe {
            let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            (GetMonitorInfoW(monitor, &mut info) != 0)
                .then_some((info.rcWork.left, info.rcWork.top, info.rcWork.right, info.rcWork.bottom))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (x, y);
        None
    }
}

/// Bring `window` to the front and focus it, even though this process isn't
/// the foreground one: a console handed over by Windows (wt++ as the default
/// terminal) arrives while the Start menu or Explorer has the foreground, and
/// Windows would otherwise open the new window behind it. Attaching to the
/// foreground thread's input is what lets us take over.
#[cfg(windows)]
pub fn bring_to_front(window: &slint::Window) {
    {
        use slint::winit_030::WinitWindowAccessor;
        use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SW_SHOW, SetForegroundWindow, ShowWindow,
        };
        let hwnd = window.with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get() as windows_sys::Win32::Foundation::HWND),
            _ => None,
        });
        let Some(Some(hwnd)) = hwnd else { return };
        // SAFETY: plain window/thread calls on live handles; the input
        // attachment is undone before returning.
        unsafe {
            let foreground = GetForegroundWindow();
            if foreground == hwnd {
                return;
            }
            let theirs = GetWindowThreadProcessId(foreground, std::ptr::null_mut());
            let ours = GetCurrentThreadId();
            let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, 1) != 0;
            ShowWindow(hwnd, SW_SHOW);
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
            if attached {
                AttachThreadInput(ours, theirs, 0);
            }
        }
    }
}

/// A floating, see-through snapshot of a window that follows the pointer
/// while a tab is dragged out of it (like WT's tab drag preview). It never
/// takes focus or the mouse. A native layered window on Windows, a Slint
/// window elsewhere (not on Wayland, which can't place windows).
pub struct DragPreview {
    #[cfg(windows)]
    hwnd: windows_sys::Win32::Foundation::HWND,
    #[cfg(not(windows))]
    window: crate::DragPreviewWindow,
    /// Its native window lets the pointer through (once it exists).
    #[cfg(not(windows))]
    click_through: std::cell::Cell<bool>,
}

#[cfg(not(windows))]
thread_local! {
    /// Set from creating the preview until its native window is created
    /// (whenever Slint does that): the window attributes hook then makes it
    /// a borderless, unfocusable overlay.
    static CREATING_PREVIEW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The window attributes hook's part for the drag preview window (see
/// `DragPreview`); other windows pass through unchanged.
#[cfg(not(windows))]
pub fn preview_window_attributes(
    attributes: slint::winit_030::winit::window::WindowAttributes,
) -> slint::winit_030::winit::window::WindowAttributes {
    use slint::winit_030::winit::window::WindowLevel;
    if !CREATING_PREVIEW.replace(false) {
        return attributes;
    }
    let attributes =
        attributes.with_decorations(false).with_active(false).with_window_level(WindowLevel::AlwaysOnTop).with_transparent(true);
    // X11: not managed by the window manager at all (no frame, no taskbar
    // entry, placed exactly where we say).
    #[cfg(all(unix, not(target_os = "macos")))]
    let attributes = {
        use slint::winit_030::winit::platform::x11::WindowAttributesExtX11;
        attributes.with_override_redirect(true)
    };
    attributes
}

impl DragPreview {
    /// Snapshot `window`'s client area as it is on screen, `width` physical
    /// pixels wide. Hidden until `show_at`.
    pub fn capture(window: &slint::Window, width: u32, label: &str) -> Option<DragPreview> {
        #[cfg(windows)]
        {
            let _ = label;
            imp::preview_capture(window, width).map(|hwnd| DragPreview { hwnd })
        }
        #[cfg(not(windows))]
        {
            // Wayland can't place a window at the pointer.
            cursor_pos(window)?;
            CREATING_PREVIEW.set(true);
            let preview = crate::DragPreviewWindow::new().ok()?;
            let size = window.size();
            let height = (width as f32 * size.height as f32 / size.width.max(1) as f32).round() as u32;
            match window.take_snapshot() {
                Ok(pixels) => preview.set_snapshot(slint::Image::from_rgba8(pixels)),
                Err(e) => log::debug!("tab drag preview: no snapshot ({e})"),
            }
            preview.set_label(label.into());
            let scale = window.scale_factor();
            preview.set_preview_width(width as f32 / scale);
            preview.set_preview_height(height as f32 / scale);
            Some(DragPreview { window: preview, click_through: std::cell::Cell::new(false) })
        }
    }

    /// Show it with its top-left at screen (`x`, `y`), or hide it. True
    /// if that just put it on screen (macOS then makes it the key window:
    /// the caller takes that back).
    pub fn show_at(&self, at: Option<(i32, i32)>) -> bool {
        #[cfg(windows)]
        {
            imp::preview_show(self.hwnd, at);
            false
        }
        #[cfg(not(windows))]
        {
            use slint::ComponentHandle;
            let window = self.window.window();
            let Some((x, y)) = at else {
                window.hide().ok();
                return false;
            };
            window.set_position(slint::PhysicalPosition::new(x, y));
            let shown = !window.is_visible();
            if shown {
                window.show().ok();
            }
            if !self.click_through.get() {
                // The pointer goes through it, to whatever is below.
                use slint::winit_030::WinitWindowAccessor;
                let done = window.with_winit_window(|w| w.set_cursor_hittest(false).is_ok());
                self.click_through.set(done.unwrap_or(false));
            }
            shown
        }
    }
}

impl Drop for DragPreview {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: our own window, destroyed once.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
        }
        #[cfg(not(windows))]
        {
            use slint::ComponentHandle;
            self.window.window().hide().ok();
            // Never shown: the next window isn't the preview.
            CREATING_PREVIEW.set(false);
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::HWND;

    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW,
        ReleaseDC,
    };
    use windows_sys::Win32::UI::Shell::ExtractIconExW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

    fn resolve(program: &str) -> Option<PathBuf> {
        let path = PathBuf::from(program);
        if path.is_absolute() {
            return path.exists().then_some(path);
        }
        let with_ext = if path.extension().is_some() { path.clone() } else { path.with_extension("exe") };
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        if let Some(root) = std::env::var_os("SystemRoot") {
            dirs.push(PathBuf::from(root).join("System32"));
        }
        dirs.into_iter().map(|d| d.join(&with_ext)).find(|p| p.exists())
    }

    /// (pid, parent pid, name) of every process.
    pub fn processes() -> Vec<(u32, u32, String)> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
        };

        let mut procs = Vec::new();
        // SAFETY: a snapshot we close; `entry` is sized as the API requires.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return procs;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut entry) != 0;
            while ok {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                procs.push((entry.th32ProcessID, entry.th32ParentProcessID, name));
                ok = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
        procs
    }

    pub fn runs_under(root: u32, exe: &str) -> bool {
        let procs = processes();
        // Walk down from `root`. Parent ids can be stale (an exited parent's
        // id reused), so only processes found under `root` are followed.
        let mut todo = vec![root];
        let mut seen = std::collections::HashSet::new();
        while let Some(pid) = todo.pop() {
            if !seen.insert(pid) {
                continue;
            }
            for (child, parent, name) in &procs {
                if *child == pid && name.eq_ignore_ascii_case(exe) {
                    return true;
                }
                if *parent == pid && *child != pid {
                    todo.push(*child);
                }
            }
        }
        false
    }

    pub fn exe_icon(program: &str) -> Option<slint::Image> {
        let path = resolve(program)?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: standard icon extraction; every handle we get is released.
        unsafe {
            let mut large: HICON = std::ptr::null_mut();
            if ExtractIconExW(wide.as_ptr(), 0, &mut large, std::ptr::null_mut(), 1) == 0 || large.is_null() {
                return None;
            }
            let image = icon_pixels(large);
            DestroyIcon(large);
            image
        }
    }

    unsafe fn icon_pixels(icon: HICON) -> Option<slint::Image> {
        unsafe {
            let mut info: ICONINFO = std::mem::zeroed();
            if GetIconInfo(icon, &mut info) == 0 {
                return None;
            }
            let result = (|| {
                let mut bitmap: BITMAP = std::mem::zeroed();
                if GetObjectW(info.hbmColor, std::mem::size_of::<BITMAP>() as i32, &mut bitmap as *mut _ as *mut _) == 0 {
                    return None;
                }
                let (w, h) = (bitmap.bmWidth, bitmap.bmHeight);
                let mut header: BITMAPINFO = std::mem::zeroed();
                header.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                let mut bgra = vec![0u8; (w * h * 4) as usize];
                let dc = GetDC(std::ptr::null_mut());
                let lines = GetDIBits(dc, info.hbmColor, 0, h as u32, bgra.as_mut_ptr() as *mut _, &mut header, DIB_RGB_COLORS);
                ReleaseDC(std::ptr::null_mut(), dc);
                if lines == 0 {
                    return None;
                }
                let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w as u32, h as u32);
                for (dst, src) in buffer.make_mut_slice().iter_mut().zip(bgra.as_chunks::<4>().0) {
                    *dst = slint::Rgba8Pixel { r: src[2], g: src[1], b: src[0], a: src[3] };
                }
                Some(slint::Image::from_rgba8(buffer))
            })();
            DeleteObject(info.hbmColor as _);
            DeleteObject(info.hbmMask as _);
            result
        }
    }

    use std::os::windows::ffi::OsStrExt;

    pub fn preview_capture(window: &slint::Window, width: u32) -> Option<HWND> {
        use windows_sys::Win32::Foundation::{POINT, SIZE};
        use windows_sys::Win32::Graphics::Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection, DeleteDC, HALFTONE,
            SRCCOPY, SelectObject, SetStretchBltMode, StretchBlt,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, RegisterClassW, ULW_ALPHA, UpdateLayeredWindow, WNDCLASSW, WS_EX_LAYERED,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
        };

        let (left, top) = crate::client_origin(window)?;
        let size = window.size();
        let (src_w, src_h) = (size.width as i32, size.height as i32);
        if src_w <= 0 || src_h <= 0 {
            return None;
        }
        let w = (width as i32).min(src_w);
        let h = (src_h as i64 * w as i64 / src_w as i64) as i32;
        let class: Vec<u16> = "wtpp-drag-preview\0".encode_utf16().collect();
        // SAFETY: plain GDI/window calls on handles created and released
        // here; the DIB's pixels are `w * h` BGRA words.
        unsafe {
            let wc = WNDCLASSW { lpfnWndProc: Some(DefWindowProcW), lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            RegisterClassW(&wc); // Fails harmlessly once registered.
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                w,
                h,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let screen = GetDC(std::ptr::null_mut());
            let mem = CreateCompatibleDC(screen);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits = std::ptr::null_mut();
            let dib = CreateDIBSection(mem, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            let old = SelectObject(mem, dib as _);
            SetStretchBltMode(mem, HALFTONE);
            StretchBlt(mem, 0, 0, w, h, screen, left, top, src_w, src_h, SRCCOPY);
            // GDI leaves alpha at 0: make the snapshot opaque, then fade
            // the whole window with the blend below.
            if !bits.is_null() {
                for px in std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize) {
                    *px |= 0xFF00_0000;
                }
            }
            let blend =
                BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 210, AlphaFormat: AC_SRC_ALPHA as u8 };
            let (origin, extent) = (POINT { x: 0, y: 0 }, SIZE { cx: w, cy: h });
            UpdateLayeredWindow(hwnd, screen, std::ptr::null(), &extent, mem, &origin, 0, &blend, ULW_ALPHA);
            SelectObject(mem, old);
            DeleteObject(dib as _);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
            Some(hwnd)
        }
    }

    pub fn preview_show(hwnd: HWND, at: Option<(i32, i32)>) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_TOPMOST, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SetWindowPos, ShowWindow,
        };
        // SAFETY: our own window.
        unsafe {
            match at {
                Some((x, y)) => {
                    SetWindowPos(hwnd, HWND_TOPMOST, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
                    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
                None => {
                    ShowWindow(hwnd, SW_HIDE);
                }
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn finds_descendants_by_name() {
        let me = std::process::id();
        let mut child = std::process::Command::new("ping").args(["-n", "5", "127.0.0.1"]).spawn().unwrap();
        assert!(super::runs_under(me, "PING.EXE"));
        assert!(!super::runs_under(me, "claude.exe"));
        assert!(!super::runs_under(child.id(), "cmd.exe"));
        child.kill().unwrap();
    }
}
