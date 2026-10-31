//! Tab strip interactions: the right-click menu, renaming, reordering and
//! dragging tabs between windows.

use slint::ComponentHandle;

use crate::app::{App, TabKind};

/// The gap between tabs, in logical pixels.
const TAB_GAP: f32 = 2.0;
/// How far below the pointer a dragged window hangs over another window's
/// tab row, in logical pixels.
const HANG_BELOW: f32 = 48.0;

impl App {
    pub fn tab_menu(&mut self, index: usize, action: &str) {
        if index >= self.tabs.len() {
            return;
        }
        match action {
            "rename" => {
                self.select_tab(index);
                if matches!(self.tabs[index].kind, TabKind::Terminal { .. }) {
                    self.tabs[index].renaming = true;
                    self.refresh_chrome();
                }
            }
            "duplicate" => {
                self.select_tab(index);
                self.run_named(wtpp_config::Action::DuplicateTab);
            }
            "split" => {
                self.select_tab(index);
                self.run_named(wtpp_config::Action::SplitPane { side: None, duplicate: true });
            }
            "close-right" => {
                for i in (index + 1..self.tabs.len()).rev() {
                    self.close_tab(i);
                }
            }
            "close-others" => {
                for i in (0..self.tabs.len()).rev() {
                    if i != index {
                        self.close_tab(i);
                    }
                }
            }
            "close" => self.close_tab(index),
            "move-to-new-window" => {
                if let Some(moved) = self.take_tab(index) {
                    crate::request_window(Some(moved), crate::Placement::Default);
                }
            }
            color if color.starts_with("color:") => {
                self.tabs[index].color = wtpp_config::Color::parse(&color["color:".len()..]);
                self.refresh_chrome();
            }
            other => log::warn!("unknown tab menu action {other}"),
        }
    }

    /// The inline rename finished. An empty name goes back to the shell's
    /// title.
    pub fn tab_renamed(&mut self, index: usize, name: &str) {
        let Some(tab) = self.tabs.get_mut(index) else { return };
        if !tab.renaming {
            return;
        }
        tab.renaming = false;
        let name = name.trim();
        tab.title = (!name.is_empty()).then(|| name.to_string());
        self.refresh_chrome();
    }

    /// A tab is being dragged: `strip_x` is the pointer within the tab strip,
    /// `tab_width` the width of one tab, (`x`, `y`) the pointer in the
    /// window. Within the tab row it reorders tabs. Pulled out of the row it
    /// lifts off, to be dropped on another window's tab row or anywhere else
    /// for a new window; the only tab in a window carries the window along.
    pub fn tab_drag(&mut self, index: usize, strip_x: f32, tab_width: f32, x: f32, y: f32) {
        let current = *self.dragging_tab.get_or_insert(index);
        let n = self.tabs.len();
        if current >= n {
            return;
        }
        let movable = matches!(self.tabs[current].kind, TabKind::Terminal { .. });
        let in_row = self.in_tab_row(x, y);
        if self.tab_lift.is_none() && movable && (n == 1 || !in_row) {
            // Hold the window by the tab: the pointer stays where it is
            // across the tab, centred in the tab row.
            let grab = (x, self.ui().get_titlebar_height() / 2.0);
            // A tab leaving a window with others gets a floating preview
            // (snapshotted before the tab dims); a lone tab moves its window.
            let preview = (n > 1).then(|| self.capture_preview(grab)).flatten();
            self.tab_lift = Some(TabLift { grab, screen: None, preview });
            self.ui().set_tab_lifted(n > 1);
        }
        if self.tab_lift.is_some() {
            let screen = crate::platform::cursor_pos().or_else(|| window_to_screen(self.ui().window(), x, y));
            let Some(lift) = self.tab_lift.as_mut() else { return };
            lift.screen = screen;
            let grab = lift.grab;
            let lift = &*lift;
            let target = crate::tab_drop_target(self.id, screen);
            if n == 1 {
                // Over another window's tab row the window hangs below the
                // pointer, so that row (and where the tab will go) stays
                // visible; the tab dims as it's about to leave.
                let hold = if target.is_some() { (grab.0, -HANG_BELOW) } else { grab };
                if let Some(screen) = screen {
                    self.move_window_to(screen, hold);
                }
                self.ui().set_tab_lifted(target.is_some());
                return;
            }
            if let (Some((preview, (ox, oy))), Some((sx, sy))) = (&lift.preview, screen) {
                // Out of the way over a tab row, where the marker shows.
                preview.show_at(target.is_none().then_some((sx - ox, sy - oy)));
            }
            if !(in_row && target.is_none()) {
                return;
            }
            // Back in our own tab row: reorder again.
            self.tab_lift = None;
            self.ui().set_tab_lifted(false);
        }

        let slot = (strip_x / (tab_width + TAB_GAP)).floor().clamp(0.0, (n - 1) as f32) as usize;
        if slot == current {
            return;
        }
        let tab = self.tabs.remove(current);
        self.tabs.insert(slot, tab);
        // The dragged tab is the active one (pressing it selected it).
        self.active = slot;
        self.dragging_tab = Some(slot);
        self.refresh_chrome();
    }

    pub fn tab_drag_end(&mut self) {
        let index = self.dragging_tab.take();
        let Some(lift) = self.tab_lift.take() else { return };
        self.ui().set_tab_lifted(false);
        let Some(index) = index else { return };
        let screen = crate::platform::cursor_pos().or(lift.screen);
        let target = crate::tab_drop_target(self.id, screen);
        crate::tab_drop_target(self.id, None);
        match target {
            Some((window, slot)) => {
                if let Some(moved) = self.take_tab(index) {
                    crate::give_tab(window, moved, slot);
                }
            }
            // The only tab carried its window here already.
            None if self.tabs.len() == 1 => {}
            None => {
                let placement = screen.map(|screen| self.new_window_placement(screen));
                if let Some(moved) = self.take_tab(index) {
                    crate::request_window(Some(moved), placement.unwrap_or(crate::Placement::Default));
                }
            }
        }
    }

    /// A see-through snapshot of this window to drag along, and the point
    /// on it matching window point `grab`.
    fn capture_preview(&self, grab: (f32, f32)) -> Option<(crate::platform::DragPreview, (i32, i32))> {
        const WIDTH: f32 = 360.0;
        let ui = self.ui();
        let window = ui.window();
        let scale = window.scale_factor();
        let full = window.size().width as f32;
        let width = (WIDTH * scale).min(full);
        let preview = crate::platform::DragPreview::capture(window, width as u32)?;
        let k = scale * width / full;
        Some((preview, ((grab.0 * k).round() as i32, (grab.1 * k).round() as i32)))
    }

    /// Whether (`x`, `y`) is in (or just around) the tab row.
    fn in_tab_row(&self, x: f32, y: f32) -> bool {
        const SLACK: f32 = 24.0;
        let ui = self.ui();
        let width = ui.window().size().width as f32 / ui.window().scale_factor();
        (-SLACK..=ui.get_titlebar_height() + SLACK).contains(&y) && (-SLACK..=width + SLACK).contains(&x)
    }

    /// Move the window so that window point `grab` is at `screen`.
    fn move_window_to(&self, (sx, sy): (i32, i32), grab: (f32, f32)) {
        let ui = self.ui();
        let window = ui.window();
        if window.is_maximized() {
            window.set_maximized(false);
        }
        let (Some((left, top)), outer) = (crate::client_origin(window), window.position()) else { return };
        let scale = window.scale_factor();
        // The frame around the client area (invisible resize borders).
        let (frame_x, frame_y) = (left - outer.x, top - outer.y);
        let x = sx - (grab.0 * scale).round() as i32 - frame_x;
        let y = sy - (grab.1 * scale).round() as i32 - frame_y;
        window.set_position(slint::PhysicalPosition::new(x, y));
    }

    /// A tab torn off at `screen` opens in a window of this one's size, with
    /// the tab under the pointer.
    fn new_window_placement(&self, (sx, sy): (i32, i32)) -> crate::Placement {
        let ui = self.ui();
        let window = ui.window();
        let scale = window.scale_factor();
        let outer = window.position();
        let (frame_x, frame_y) = crate::client_origin(window).map_or((0, 0), |(l, t)| (l - outer.x, t - outer.y));
        let grab_x = ((ui.get_tabs_left() + 60.0) * scale).round() as i32;
        let grab_y = (ui.get_titlebar_height() / 2.0 * scale).round() as i32;
        let size = window.size();
        let (mut x, mut y) = (sx - grab_x - frame_x, sy - grab_y - frame_y);
        // Keep it on the monitor it was dropped on, clear of the taskbar.
        if let Some((left, top, right, bottom)) = crate::platform::work_area_at(sx, sy) {
            let (width, height) = (size.width as i32 + 2 * frame_x, size.height as i32 + frame_y + frame_x);
            x = x.min(right - width).max(left - frame_x);
            y = y.min(bottom - height).max(top);
        }
        crate::Placement::At(slint::PhysicalPosition::new(x, y), size)
    }
}

/// A tab dragged out of the tab row.
pub struct TabLift {
    /// The point of the window held under the pointer (window coordinates).
    grab: (f32, f32),
    /// The pointer on screen, last we knew.
    screen: Option<(i32, i32)>,
    /// The floating snapshot, and where the pointer sits on it.
    preview: Option<(crate::platform::DragPreview, (i32, i32))>,
}

/// A window point in screen coordinates, from the window's position (when
/// the OS can't tell us where the pointer is).
fn window_to_screen(window: &slint::Window, x: f32, y: f32) -> Option<(i32, i32)> {
    let (left, top) = crate::client_origin(window)?;
    let scale = window.scale_factor();
    Some((left + (x * scale).round() as i32, top + (y * scale).round() as i32))
}
