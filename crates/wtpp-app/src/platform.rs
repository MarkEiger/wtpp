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

/// The mouse position in a window, in logical coordinates, read from the
/// OS. winit's file drops carry no position (and no mouse moves arrive
/// during an OLE drag), so this is where a drop landed. Windows only.
pub fn cursor_in_window(window: &slint::Window) -> Option<(f32, f32)> {
    #[cfg(windows)]
    {
        use slint::winit_030::WinitWindowAccessor;
        use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::Foundation::{HWND, POINT};
        use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let hwnd = window.with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get() as HWND),
            _ => None,
        })??;
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: plain Win32 calls on our own window with a valid POINT.
        if unsafe { GetCursorPos(&mut point) == 0 || ScreenToClient(hwnd, &mut point) == 0 } {
            return None;
        }
        let scale = window.scale_factor();
        Some((point.x as f32 / scale, point.y as f32 / scale))
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;

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
}
