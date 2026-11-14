//! Tab strip interactions: the right-click menu, renaming, reordering and
//! dragging tabs between windows.

use slint::ComponentHandle;

use crate::app::{App, TabKind};
use crate::tab_groups::{self, GroupId, TabGroup};

/// The gap between tabs, in logical pixels.
const TAB_GAP: f32 = 2.0;
/// How far below the pointer a dragged window hangs over another window's
/// tab row, in logical pixels.
const HANG_BELOW: f32 = 48.0;

/// Each tab's start and width in the strip, once the UI has laid it out.
pub(crate) type SharedExtents = std::rc::Rc<std::cell::RefCell<Vec<Option<(f32, f32)>>>>;

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
            "split" | "split-default" => {
                self.select_tab(index);
                let duplicate = action == "split";
                self.run_named(wtpp_config::Action::SplitPane { side: None, duplicate });
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
            group if group.starts_with("group:") => {
                // Settings isn't a tab you'd keep with others.
                if !matches!(self.tabs[index].kind, TabKind::Terminal { .. }) {
                    return;
                }
                match &group["group:".len()..] {
                    "new" => {
                        let at = self.leave_group(index);
                        let used: Vec<_> = self.groups.iter().map(|g| g.color).collect();
                        let mut group = TabGroup::new(tab_groups::next_color(&used));
                        group.renaming = true;
                        self.tabs[at].group = Some(group.id);
                        self.groups.push(group);
                    }
                    "remove" => {
                        self.leave_group(index);
                    }
                    join => {
                        let Some(gid) = join.strip_prefix("join:").and_then(|id| id.parse::<GroupId>().ok()) else {
                            log::warn!("unknown tab menu action {action}");
                            return;
                        };
                        if self.tabs[index].group == Some(gid) || self.group(gid).is_none() {
                            return;
                        }
                        let at = self.leave_group(index);
                        // After the group's last tab, keeping it together.
                        let Some(to) = tab_groups::join_target(&self.tab_groups(), at, gid) else { return };
                        self.move_tab(at, to);
                        self.tabs[to].group = Some(gid);
                    }
                }
                self.relayout();
            }
            other => log::warn!("unknown tab menu action {other}"),
        }
    }

    /// Take tab `index` out of its group, moving it just past the group's
    /// other tabs so they stay together. Returns where the tab is now.
    fn leave_group(&mut self, index: usize) -> usize {
        let Some(gid) = self.tabs[index].group.take() else { return index };
        let to = tab_groups::leave_target(&self.tab_groups(), index, gid);
        self.move_tab(index, to);
        to
    }

    /// Move a tab from `from` to `to`, keeping the active and dragged tabs.
    pub(crate) fn move_tab(&mut self, from: usize, to: usize) {
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = tab_groups::moved_index(self.active, from, to);
        self.dragging_tab = self.dragging_tab.map(|i| tab_groups::moved_index(i, from, to));
    }

    /// Each tab's group, in tab order.
    pub(crate) fn tab_groups(&self) -> Vec<Option<GroupId>> {
        self.tabs.iter().map(|t| t.group).collect()
    }

    pub(crate) fn group(&self, id: GroupId) -> Option<&TabGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    fn group_mut(&mut self, id: GroupId) -> Option<&mut TabGroup> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// Which tabs are tucked away in collapsed groups (never `active`).
    pub(crate) fn hidden_tabs(&self, active: usize) -> Vec<bool> {
        tab_groups::hidden(&self.tab_groups(), |g| self.group(g).is_some_and(|g| g.collapsed), active)
    }

    /// Keep tab groups consistent after anything that adds, moves, closes or
    /// selects tabs: each group's tabs together, no empty groups, and the
    /// active tab never hidden (selecting a tab in a collapsed group, from a
    /// shortcut, the palette or a closed neighbour, opens the group).
    pub(crate) fn tidy_groups(&mut self) {
        if self.groups.is_empty() && self.tabs.iter().all(|t| t.group.is_none()) {
            return;
        }
        let order = tab_groups::contiguous_order(&self.tab_groups());
        if order.iter().enumerate().any(|(k, &i)| k != i) {
            let mut old: Vec<_> = self.tabs.drain(..).map(Some).collect();
            self.tabs = order.iter().filter_map(|&i| old[i].take()).collect();
            let new_index = tab_groups::inverse(&order);
            self.active = new_index.get(self.active).copied().unwrap_or(self.active);
            self.dragging_tab = self.dragging_tab.map(|i| new_index.get(i).copied().unwrap_or(i));
        }
        let tabs = &self.tabs;
        self.groups.retain(|g| tabs.iter().any(|t| t.group == Some(g.id)));
        let groups = &self.groups;
        for tab in &mut self.tabs {
            if tab.group.is_some_and(|id| !groups.iter().any(|g| g.id == id)) {
                tab.group = None;
            }
        }
        if let Some(gid) = self.tabs.get(self.active).and_then(|t| t.group)
            && let Some(group) = self.group_mut(gid)
        {
            group.collapsed = false;
        }
    }

    /// An action from a group chip's menu (or a click on the chip, which
    /// toggles), for the group tab `index` is in.
    pub fn group_menu(&mut self, index: usize, action: &str) {
        let Some(gid) = self.tabs.get(index).and_then(|t| t.group) else { return };
        match action {
            "rename" => {
                if let Some(group) = self.group_mut(gid) {
                    group.renaming = true;
                }
                self.refresh_chrome();
            }
            "toggle" => self.toggle_group(gid),
            "ungroup" => {
                for tab in self.tabs.iter_mut().filter(|t| t.group == Some(gid)) {
                    tab.group = None;
                }
                self.relayout();
            }
            "close" => {
                let members: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.tabs[i].group == Some(gid)).collect();
                for i in members.into_iter().rev() {
                    self.close_tab(i);
                }
            }
            color if color.starts_with("color:") => {
                if let (Some(c), Some(group)) = (wtpp_config::Color::parse(&color["color:".len()..]), self.group_mut(gid)) {
                    group.color = c;
                }
                self.refresh_chrome();
            }
            other => log::warn!("unknown group menu action {other}"),
        }
    }

    /// Collapse or expand a group. Collapsing the group of the active tab
    /// first selects the nearest tab outside it; with none, it stays open.
    fn toggle_group(&mut self, gid: GroupId) {
        let Some(group) = self.group(gid) else { return };
        if !group.collapsed {
            let collapsed = |g| self.group(g).is_some_and(|g| g.collapsed);
            let Some(target) = tab_groups::collapse_target(&self.tab_groups(), gid, collapsed, self.active) else { return };
            self.active = target;
            self.tabs[target].bell = false;
        }
        if let Some(group) = self.group_mut(gid) {
            group.collapsed = !group.collapsed;
        }
        self.relayout();
    }

    /// The chip's inline rename finished. An empty title leaves just the
    /// coloured dot.
    pub fn group_renamed(&mut self, index: usize, name: &str) {
        let Some(gid) = self.tabs.get(index).and_then(|t| t.group) else { return };
        let Some(group) = self.group_mut(gid) else { return };
        if !group.renaming {
            return;
        }
        group.renaming = false;
        group.title = name.trim().to_string();
        self.refresh_chrome();
    }

    /// Every tab's extent in the strip, once the UI has reported them all.
    pub(crate) fn extents(&self) -> Option<Vec<(f32, f32)>> {
        let extents = self.tab_extents.borrow();
        if extents.len() != self.tabs.len() {
            return None;
        }
        extents.iter().copied().collect()
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
        if self.dragging_tab.is_none() {
            self.drag_origin = self.tabs.get(index).and_then(|t| t.group);
        }
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
            self.ui().set_drag_hint("".into());
            let screen = crate::platform::cursor_pos(self.ui().window()).or_else(|| window_to_screen(self.ui().window(), x, y));
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
                if preview.show_at(target.is_none().then_some((sx - ox, sy - oy))) && cfg!(target_os = "macos") {
                    // Once Slint has created it (it takes the key window).
                    let win = self.id;
                    slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
                        crate::with_window(win, |app| {
                            use slint::winit_030::WinitWindowAccessor;
                            app.ui().window().with_winit_window(|w| w.focus_window());
                        });
                    });
                }
            }
            if !(in_row && target.is_none()) {
                return;
            }
            // Back in our own tab row: reorder again.
            self.tab_lift = None;
            self.ui().set_tab_lifted(false);
        }

        // Where the tab would go among the others, by their real extents
        // (tabs differ in width, and group chips sit between them).
        let extents = self.extents();
        let slot = match &extents {
            Some(extents) => {
                let others: Vec<_> = (0..n).filter(|&i| i != current).map(|i| extents[i]).collect();
                tab_groups::slot_at(&others, strip_x).min(n - 1)
            }
            None => (strip_x / (tab_width + TAB_GAP)).floor().clamp(0.0, (n - 1) as f32) as usize,
        };
        if slot == current {
            self.drag_over_group_edge(current, strip_x);
            return;
        }
        // The dragged tab is the active one (pressing it selected it).
        let own = self.tabs[current].group;
        self.move_tab(current, slot);
        if let Some(mut extents) = extents {
            // Until the UI reports the new layout, guess it: the moved
            // range packed from where it started.
            let (lo, hi) = (current.min(slot), current.max(slot));
            let mut x = extents[lo].0;
            let moved = extents.remove(current);
            extents.insert(slot, moved);
            for extent in &mut extents[lo..=hi] {
                extent.0 = x;
                if extent.1 > 0.0 {
                    x += extent.1 + TAB_GAP;
                }
            }
            *self.tab_extents.borrow_mut() = extents.into_iter().map(Some).collect();
        }
        // Dragged into a group it joins; dragged away from its own it leaves.
        let terminal = matches!(self.tabs[slot].kind, TabKind::Terminal { .. });
        self.tabs[slot].group = terminal.then(|| tab_groups::group_at_insert(&self.tab_groups(), slot, own)).flatten();
        self.tidy_groups();
        self.refresh_chrome();
        if let Some(current) = self.dragging_tab {
            self.show_drag_hint(current);
        }
    }

    /// The dragged tab at a group's edge: pulled far enough past it, it
    /// leaves the group; pulled into the group's end tab, it joins. A hint
    /// says what letting go does. Nothing is final until the drop.
    fn drag_over_group_edge(&mut self, current: usize, x: f32) {
        if matches!(self.tabs[current].kind, TabKind::Terminal { .. })
            && let Some(extents) = self.extents()
            && let Some(group) = tab_groups::edge_group(&self.tab_groups(), &extents, current, x, TAB_GAP)
        {
            self.tabs[current].group = group;
            self.tidy_groups();
            self.refresh_chrome();
        }
        if let Some(current) = self.dragging_tab {
            self.show_drag_hint(current);
        }
    }

    /// Under the dragged tab, when it's in another group than it started
    /// in: "Release to add to <group>", or "Release to remove from <group>".
    fn show_drag_hint(&self, current: usize) {
        let name = |g: &TabGroup, empty: &str| match g.title.trim() {
            "" => empty.to_string(),
            title => format!("\u{201c}{title}\u{201d}"),
        };
        let now = self.tabs.get(current).and_then(|t| t.group);
        let hint = if now == self.drag_origin {
            None
        } else if let Some(g) = now.and_then(|g| self.group(g)) {
            Some(format!("Release to add to {}", name(g, "this group")))
        } else {
            self.drag_origin.and_then(|g| self.group(g)).map(|g| format!("Release to remove from {}", name(g, "its group")))
        };
        let x = self.extents().and_then(|e| e.get(current).map(|&(start, _)| start)).unwrap_or(0.0);
        let ui = self.ui();
        ui.set_drag_hint_x(x);
        ui.set_drag_hint(hint.unwrap_or_default().into());
    }

    pub fn tab_drag_end(&mut self) {
        let index = self.dragging_tab.take();
        self.drag_origin = None;
        self.ui().set_drag_hint("".into());
        let Some(lift) = self.tab_lift.take() else { return };
        self.ui().set_tab_lifted(false);
        let Some(index) = index else { return };
        let screen = crate::platform::cursor_pos(self.ui().window()).or(lift.screen);
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
        let title = self.tabs.get(self.dragging_tab?).map(|t| self.tab_title(t)).unwrap_or_default();
        let preview = crate::platform::DragPreview::capture(window, width as u32, &title)?;
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
        if window.is_fullscreen() {
            window.set_fullscreen(false);
        }
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

    /// A tab torn off at `screen` opens in a window of this one's size (the
    /// default size if this one fills the screen), with the tab under the
    /// pointer.
    fn new_window_placement(&self, (sx, sy): (i32, i32)) -> crate::Placement {
        let ui = self.ui();
        let window = ui.window();
        let scale = window.scale_factor();
        let outer = window.position();
        let (frame_x, frame_y) = crate::client_origin(window).map_or((0, 0), |(l, t)| (l - outer.x, t - outer.y));
        let grab_x = ((ui.get_tabs_left() + 60.0) * scale).round() as i32;
        let grab_y = (ui.get_titlebar_height() / 2.0 * scale).round() as i32;
        let size = if window.is_maximized() || window.is_fullscreen() {
            crate::default_window_size(&self.settings).to_physical(scale)
        } else {
            window.size()
        };
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
