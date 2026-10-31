//! Find (Ctrl+Shift+F): plain-text search through a pane's scrollback.

use wtpp_render::{Highlight, HighlightKind};
use wtpp_session::alacritty_terminal::Term;
use wtpp_session::alacritty_terminal::grid::Dimensions;
use wtpp_session::alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use wtpp_session::alacritty_terminal::selection::{Selection, SelectionType};
use wtpp_session::alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

use crate::app::App;

/// Stop counting matches beyond this (the count is just for the status).
const MAX_COUNT: usize = 999;

#[derive(Default)]
pub struct FindState {
    query: String,
    current: Option<Match>,
    /// Compiled query while the find bar is open (for highlighting).
    regex: Option<RegexSearch>,
    case_sensitive: bool,
    use_regex: bool,
}

impl FindState {
    fn compile(&self) -> Option<Result<RegexSearch, ()>> {
        if self.query.is_empty() {
            return None;
        }
        let pattern = if self.use_regex { self.query.clone() } else { escape_regex(&self.query) };
        // alacritty is smart-case; force the mode the toggle asks for.
        let pattern = if self.case_sensitive { format!("(?-i){pattern}") } else { format!("(?i){pattern}") };
        Some(RegexSearch::new(&pattern).map_err(|_| ()))
    }
}

/// Matches in the terminal's current viewport, for highlighting.
pub fn visible_matches<T>(find: &FindState, term: &Term<T>) -> Vec<Highlight> {
    let Some(regex) = &find.regex else { return Vec::new() };
    let mut regex = regex.clone();
    let offset = term.grid().display_offset() as i32;
    let top = Point::new(Line(-offset), Column(0));
    let bottom = Point::new(Line(term.screen_lines() as i32 - 1 - offset), term.last_column());
    RegexIter::new(top, bottom, Direction::Right, term, &mut regex)
        .take(500)
        .map(|m| Highlight { start: *m.start(), end: *m.end(), kind: HighlightKind::Match })
        .collect()
}

impl App {
    pub fn open_find(&mut self) {
        let ui = self.ui();
        ui.set_find_visible(true);
        ui.invoke_focus_find();
    }

    pub fn find_close(&mut self) {
        let ui = self.ui();
        ui.set_find_visible(false);
        ui.set_find_status("".into());
        self.find.current = None;
        self.find.regex = None;
        ui.invoke_focus_keys();
        self.render();
    }

    /// The case-sensitive or regex toggle changed.
    pub fn find_options(&mut self, case_sensitive: bool, use_regex: bool) {
        self.find.case_sensitive = case_sensitive;
        self.find.use_regex = use_regex;
        self.find.current = None;
        self.find_step(false);
    }

    pub fn find_changed(&mut self, query: &str) {
        self.find.query = query.to_string();
        self.find.current = None;
        // New text searches up from the cursor: the most recent match first.
        self.find_step(false);
    }

    /// Go to the next match below (`down`) or above the current one.
    pub fn find_next(&mut self, down: bool) {
        self.find_step(down);
    }

    fn find_step(&mut self, down: bool) {
        let ui = self.ui();
        let Some(term) = self.focused_term() else { return };
        let mut regex = match self.find.compile() {
            None => {
                self.find.regex = None;
                term.lock().selection = None;
                ui.set_find_status("".into());
                self.render();
                return;
            }
            Some(Err(())) => {
                self.find.regex = None;
                ui.set_find_status("Invalid".into());
                self.render();
                return;
            }
            Some(Ok(regex)) => regex,
        };
        self.find.regex = Some(regex.clone());

        let status = {
            let mut term = term.lock();
            let direction = if down { Direction::Right } else { Direction::Left };
            let origin = match &self.find.current {
                Some(m) if down => m.end().add(&*term, Boundary::None, 1),
                Some(m) => m.start().sub(&*term, Boundary::None, 1),
                None => term.grid().cursor.point,
            };
            let side = if down { Side::Left } else { Side::Right };
            match term.search_next(&mut regex, origin, direction, side, None) {
                Some(m) => {
                    let mut selection = Selection::new(SelectionType::Simple, *m.start(), Side::Left);
                    selection.update(*m.end(), Side::Right);
                    term.selection = Some(selection);
                    term.scroll_to_point(*m.start());

                    // "3/10", counted from the top of the scrollback.
                    let top = Point::new(term.topmost_line(), Column(0));
                    let bottom = Point::new(term.bottommost_line(), term.last_column());
                    let mut index = 0;
                    let mut total = 0;
                    for found in RegexIter::new(top, bottom, Direction::Right, &term, &mut regex).take(MAX_COUNT) {
                        total += 1;
                        if found.start() <= m.start() {
                            index = total;
                        }
                    }
                    self.find.current = Some(m);
                    if total >= MAX_COUNT { format!("{index}/{MAX_COUNT}+") } else { format!("{index}/{total}") }
                }
                None => {
                    term.selection = None;
                    self.find.current = None;
                    "No results".to_string()
                }
            }
        };
        ui.set_find_status(status.into());
        self.render();
    }
}

/// Match the query literally.
fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_queries() {
        assert_eq!(escape_regex("a.b*(c)"), r"a\.b\*\(c\)");
        assert!(RegexSearch::new(&escape_regex("[x] {y} ^$ a|b")).is_ok());
        let find = |query: &str, case_sensitive, use_regex| FindState {
            query: query.into(),
            case_sensitive,
            use_regex,
            ..Default::default()
        };
        assert!(matches!(find("a.b", false, false).compile(), Some(Ok(_))));
        assert!(matches!(find("a(b", false, true).compile(), Some(Err(()))));
        assert!(find("", false, false).compile().is_none());
    }
}
