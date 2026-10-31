//! Tabs in the title bar on Windows, like Windows Terminal.
//!
//! The window keeps its native frame (so resizing, Aero Snap, shadows and
//! rounded corners all still work), but the client area is extended over the
//! caption. A window subclass then tells Windows which parts of our tab row
//! act as caption (drag, double-click to maximize, system menu) and where our
//! maximize button is, so Windows 11 shows its Snap Layouts flyout on hover.

use std::sync::atomic::{AtomicBool, AtomicI32};

/// One window's tab row geometry in physical pixels, updated by the UI
/// after layout and read by that window's subclass procedure.
#[derive(Default)]
pub struct Geometry {
    /// The window these belong to (passed to the callbacks).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub window: usize,
    /// Height of the tab row.
    pub titlebar_height: AtomicI32,
    /// Start of the empty, draggable part of the tab row (after the buttons).
    pub drag_start: AtomicI32,
    /// Left edge of the minimize/maximize/close buttons.
    pub buttons_start: AtomicI32,
    /// Width of one caption button.
    pub button_width: AtomicI32,
    /// The pointer is over our maximize button (Windows owns that area).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub max_hovered: AtomicBool,
}

impl Geometry {
    pub fn new(window: usize) -> Geometry {
        Geometry {
            window,
            drag_start: AtomicI32::new(i32::MAX),
            buttons_start: AtomicI32::new(i32::MAX),
            ..Default::default()
        }
    }
}

/// Maximize-button hover changed / maximize button clicked, for a window.
pub type OnHover = fn(usize, bool);
pub type OnMaxClick = fn(usize);

#[cfg(windows)]
pub use imp::{install, set_client_size};

#[cfg(not(windows))]
pub fn install(
    _window: &slint::Window,
    _geometry: &std::sync::Arc<Geometry>,
    _on_hover: OnHover,
    _on_max_click: OnMaxClick,
) -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_client_size(_window: &slint::Window, _width: u32, _height: u32) {}

#[cfg(windows)]
mod imp {
    use std::sync::OnceLock;
    use std::sync::atomic::Ordering;

    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    use super::{Geometry, OnHover, OnMaxClick};

    static CALLBACKS: OnceLock<(OnHover, OnMaxClick)> = OnceLock::new();

    /// Take over the caption of `window`. Returns false if this isn't a
    /// native Win32 window (then the native title bar stays). `geometry`
    /// must outlive the window (the app keeps it for the window's lifetime).
    pub fn install(
        window: &slint::Window,
        geometry: &std::sync::Arc<Geometry>,
        on_hover: OnHover,
        on_max_click: OnMaxClick,
    ) -> bool {
        let _ = CALLBACKS.set((on_hover, on_max_click));
        let hwnd = window.with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get() as HWND),
            _ => None,
        });
        let Some(Some(hwnd)) = hwnd else { return false };
        // SAFETY: plain Win32 calls on a window owned by this thread; the
        // subclass proc only reads atomics and forwards to DefSubclassProc.
        unsafe {
            let data = std::sync::Arc::as_ptr(geometry) as usize;
            if SetWindowSubclass(hwnd, Some(subclass_proc), 1, data) == 0 {
                return false;
            }
            // Recompute the frame with our WM_NCCALCSIZE.
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        true
    }

    /// Resize so the client area (our whole UI) is `width`×`height` physical
    /// pixels. Needed after `install`: the window was created with a caption
    /// that is now part of the client area, so winit's size math (which still
    /// assumes a caption) would be off by its height.
    pub fn set_client_size(window: &slint::Window, width: u32, height: u32) {
        let hwnd = window.with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get() as HWND),
            _ => None,
        });
        let Some(Some(hwnd)) = hwnd else { return };
        // SAFETY: plain Win32 calls on a window owned by this thread.
        unsafe {
            let mut outer = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let mut client = outer;
            GetWindowRect(hwnd, &mut outer);
            GetClientRect(hwnd, &mut client);
            let dx = (outer.right - outer.left) - client.right;
            let dy = (outer.bottom - outer.top) - client.bottom;
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                width as i32 + dx,
                height as i32 + dy,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// Resize border thickness (frame + padding), in physical pixels.
    unsafe fn frame_thickness(hwnd: HWND) -> i32 {
        unsafe {
            let dpi = GetDpiForWindow(hwnd);
            GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        }
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        unsafe {
            // SAFETY: `data` is the Arc<Geometry> the app keeps alive for
            // as long as this window exists.
            let geometry = &*(data as *const Geometry);
            match msg {
                WM_NCCALCSIZE if wparam != 0 => {
                    // Let Windows compute the normal frame, then give the
                    // caption back to the client area. Side and bottom
                    // borders stay native (resizing, shadow).
                    let params = &mut *(lparam as *mut NCCALCSIZE_PARAMS);
                    let top = params.rgrc[0].top;
                    let ret = DefSubclassProc(hwnd, msg, wparam, lparam);
                    if ret != 0 {
                        return ret;
                    }
                    params.rgrc[0].top = top;
                    // A maximized window hangs off-screen by its frame.
                    if IsZoomed(hwnd) != 0 {
                        params.rgrc[0].top += frame_thickness(hwnd);
                    }
                    0
                }
                WM_NCHITTEST => {
                    let hit = DefSubclassProc(hwnd, msg, wparam, lparam);
                    if hit as u32 != HTCLIENT {
                        return hit;
                    }
                    let mut pt = POINT { x: (lparam & 0xFFFF) as i16 as i32, y: ((lparam >> 16) & 0xFFFF) as i16 as i32 };
                    ScreenToClient(hwnd, &mut pt);
                    hit_test(hwnd, geometry, pt) as LRESULT
                }
                // Our maximize button is a native HTMAXBUTTON (for Snap
                // Layouts), so its hover and click arrive as non-client
                // messages.
                WM_NCMOUSEMOVE => {
                    set_max_hovered(geometry, wparam as u32 == HTMAXBUTTON);
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }
                WM_NCMOUSELEAVE | WM_MOUSEMOVE => {
                    set_max_hovered(geometry, false);
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }
                WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam as u32 == HTMAXBUTTON => 0,
                WM_NCLBUTTONUP if wparam as u32 == HTMAXBUTTON => {
                    if let Some((_, on_max_click)) = CALLBACKS.get() {
                        on_max_click(geometry.window);
                    }
                    0
                }
                _ => DefSubclassProc(hwnd, msg, wparam, lparam),
            }
        }
    }

    fn set_max_hovered(geometry: &Geometry, hovered: bool) {
        if geometry.max_hovered.swap(hovered, Ordering::AcqRel) != hovered
            && let Some((on_hover, _)) = CALLBACKS.get()
        {
            on_hover(geometry.window, hovered);
        }
    }

    unsafe fn hit_test(hwnd: HWND, geometry: &Geometry, pt: POINT) -> u32 {
        let maximized = unsafe { IsZoomed(hwnd) } != 0;
        let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(hwnd, &mut rect) };
        let width = rect.right;
        let border = unsafe { frame_thickness(hwnd) };

        // The top resize edge is inside our client area now.
        if !maximized && pt.y < border {
            return if pt.x < border * 2 {
                HTTOPLEFT
            } else if pt.x >= width - border * 2 {
                HTTOPRIGHT
            } else {
                HTTOP
            };
        }

        let titlebar = geometry.titlebar_height.load(Ordering::Acquire);
        if pt.y >= titlebar {
            return HTCLIENT;
        }
        let buttons = geometry.buttons_start.load(Ordering::Acquire);
        let button = geometry.button_width.load(Ordering::Acquire);
        if pt.x >= buttons {
            let index = (pt.x - buttons) / button.max(1);
            return if index == 1 { HTMAXBUTTON } else { HTCLIENT };
        }
        if pt.x >= geometry.drag_start.load(Ordering::Acquire) {
            return HTCAPTION;
        }
        HTCLIENT
    }
}
