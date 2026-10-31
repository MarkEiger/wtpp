//! The command palette (Ctrl+Shift+P): fuzzy search over every action.

use slint::{ModelRc, VecModel};
use wtpp_config::Action;

use crate::app::App;
use crate::{MainWindow, PaletteItem};

/// How many matches to list.
const MAX_ITEMS: usize = 14;

impl App {
    pub fn open_palette(&mut self) {
        let ui = self.ui();
        ui.set_palette_query("".into());
        self.palette_changed("");
        ui.set_palette_visible(true);
        ui.invoke_focus_palette();
    }

    pub fn palette_close(&mut self) {
        let ui = self.ui();
        ui.set_palette_visible(false);
        ui.invoke_focus_keys();
    }

    pub fn palette_changed(&mut self, query: &str) {
        let names: Vec<String> = self.profiles.iter().map(|p| p.name.clone()).collect();
        let mut scored: Vec<(i32, usize, Action)> = Action::catalog(names.len())
            .into_iter()
            // Needs text; only useful as a binding.
            .filter(|a| !matches!(a, Action::SendInput(_)))
            .enumerate()
            .filter_map(|(i, a)| fuzzy_score(&a.label(&names), query).map(|s| (s, i, a)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.palette = scored.into_iter().take(MAX_ITEMS).map(|(_, _, a)| a).collect();

        let items: Vec<PaletteItem> = self
            .palette
            .iter()
            .map(|a| PaletteItem {
                label: a.label(&names).into(),
                keys: self.keymap.chord_label(a).unwrap_or_default().into(),
            })
            .collect();
        let ui: MainWindow = self.ui();
        ui.set_palette_items(ModelRc::new(VecModel::from(items)));
        ui.set_palette_selected(0);
    }

    pub fn palette_run(&mut self, index: usize) {
        let action = self.palette.get(index).cloned();
        self.palette_close();
        if let Some(action) = action {
            self.run_named(action);
        }
    }
}

/// Subsequence match of `query` in `label`, case-insensitive. Higher is
/// better: consecutive letters and word starts score extra. `None` if the
/// query letters don't all appear in order.
fn fuzzy_score(label: &str, query: &str) -> Option<i32> {
    let label: Vec<char> = label.to_lowercase().chars().collect();
    let mut score = 0;
    let mut pos = 0;
    let mut prev_match: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = (pos..label.len()).find(|&i| label[i] == q)?;
        score += 1;
        if prev_match.is_some_and(|p| p + 1 == found) {
            score += 5;
        }
        if found == 0 || !label[found - 1].is_alphanumeric() {
            score += 3;
        }
        prev_match = Some(found);
        pos = found + 1;
    }
    if score == 0 {
        // Empty query: keep the catalog's natural order.
        return Some(0);
    }
    // Prefer shorter labels for equal matches.
    Some(score * 100 - label.len() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy() {
        assert!(fuzzy_score("Split pane (right)", "spr").is_some());
        assert!(fuzzy_score("Split pane (right)", "xyz").is_none());
        assert!(fuzzy_score("New tab", "").is_some());
        // Word starts beat scattered letters.
        assert!(fuzzy_score("New tab", "nt") > fuzzy_score("Scroll down one page", "nt"));
        // Consecutive letters beat gaps.
        assert!(fuzzy_score("Paste", "pas") > fuzzy_score("Split pane (auto)", "pas"));
    }
}
