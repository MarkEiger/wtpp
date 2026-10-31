//! Tab strip interactions: the right-click menu, renaming and reordering.

use crate::app::{App, TabKind};

/// Start of the tab strip (the UI passes strip-relative positions) and the
/// gap between tabs, in logical pixels.
const ROW_START: f32 = 0.0;
const TAB_GAP: f32 = 2.0;

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
                    crate::request_window(Some(moved));
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

    /// A tab is being dragged; `x` is the pointer in window coordinates and
    /// `tab_width` the width of one tab.
    pub fn tab_drag(&mut self, index: usize, x: f32, tab_width: f32) {
        let current = *self.dragging_tab.get_or_insert(index);
        let n = self.tabs.len();
        let slot = ((x - ROW_START) / (tab_width + TAB_GAP)).floor().clamp(0.0, (n - 1) as f32) as usize;
        if slot == current || current >= n {
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
        self.dragging_tab = None;
    }
}
