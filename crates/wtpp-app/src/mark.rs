//! Mark mode (Ctrl+Shift+M): select text with the keyboard, like WT.
//!
//! Arrows move the mark, Shift extends the selection from where it started,
//! Home/End/PgUp/PgDn and Ctrl+Home/End jump, Enter copies, Esc leaves.

use wtpp_input::{Key, Mods};
use wtpp_session::alacritty_terminal::grid::Dimensions;
use wtpp_session::alacritty_terminal::index::{Column, Line, Point, Side};
use wtpp_session::alacritty_terminal::selection::{Selection, SelectionType};

use crate::app::App;
use crate::pane_tree::PaneId;

pub struct MarkState {
    pane: PaneId,
    cursor: Point,
    /// Where a Shift-selection started.
    anchor: Option<Point>,
}

/// Move `p` by one key press within a grid of `lines` (history included,
/// `top`..=`bottom`) and `cols` columns.
fn step(p: Point, key: Key, ctrl: bool, top: i32, bottom: i32, cols: usize, page: i32) -> Point {
    let last = cols - 1;
    let line = |l: i32| Line(l.clamp(top, bottom));
    match key {
        Key::Left if p.column.0 > 0 => Point::new(p.line, p.column - 1),
        Key::Left if p.line.0 > top => Point::new(p.line - 1, Column(last)),
        Key::Right if p.column.0 < last => Point::new(p.line, p.column + 1),
        Key::Right if p.line.0 < bottom => Point::new(p.line + 1, Column(0)),
        Key::Up => Point::new(line(p.line.0 - 1), p.column),
        Key::Down => Point::new(line(p.line.0 + 1), p.column),
        Key::PageUp => Point::new(line(p.line.0 - page), p.column),
        Key::PageDown => Point::new(line(p.line.0 + page), p.column),
        Key::Home if ctrl => Point::new(Line(top), Column(0)),
        Key::End if ctrl => Point::new(Line(bottom), Column(last)),
        Key::Home => Point::new(p.line, Column(0)),
        Key::End => Point::new(p.line, Column(last)),
        _ => p,
    }
}

impl App {
    pub fn toggle_mark_mode(&mut self) {
        if self.mark.take().is_some() {
            if let Some(term) = self.focused_term() {
                term.lock().selection = None;
            }
            self.render();
            return;
        }
        let Some((_, pane)) = self.active_terminal() else { return };
        let Some(term) = self.focused_term() else { return };
        let cursor = term.lock().grid().cursor.point;
        self.mark = Some(MarkState { pane, cursor, anchor: None });
        self.update_mark_selection();
    }

    /// A key while in mark mode. Every key is consumed, like WT.
    pub fn mark_key(&mut self, key: Key, mods: Mods) {
        let Some(term) = self.focused_term() else { return };
        let Some(mark) = self.mark.as_mut() else { return };
        match key {
            Key::Escape => {
                self.mark = None;
                term.lock().selection = None;
            }
            Key::Enter => {
                self.mark = None;
                self.run_named(wtpp_config::Action::Copy);
            }
            _ => {
                let t = term.lock();
                let (top, bottom) = (t.topmost_line().0, t.bottommost_line().0);
                let (cols, page) = (t.columns(), t.screen_lines() as i32);
                drop(t);
                let moved = step(mark.cursor, key, mods.ctrl, top, bottom, cols, page);
                mark.anchor = if mods.shift { mark.anchor.or(Some(mark.cursor)) } else { None };
                mark.cursor = moved;
                self.update_mark_selection();
                return;
            }
        }
        self.render();
    }

    pub(crate) fn mark_active_for(&self, pane: PaneId) -> bool {
        self.mark.as_ref().is_some_and(|m| m.pane == pane)
    }

    /// Show the mark (or the Shift-selection) as the selection, and keep it
    /// in view.
    fn update_mark_selection(&mut self) {
        let (Some(mark), Some(term)) = (&self.mark, self.focused_term()) else { return };
        let (a, b) = match mark.anchor {
            Some(anchor) if anchor <= mark.cursor => (anchor, mark.cursor),
            Some(anchor) => (mark.cursor, anchor),
            None => (mark.cursor, mark.cursor),
        };
        {
            let mut t = term.lock();
            let mut selection = Selection::new(SelectionType::Simple, a, Side::Left);
            selection.update(b, Side::Right);
            t.selection = Some(selection);
            t.scroll_to_point(mark.cursor);
        }
        self.render();
    }

    /// Select the whole buffer, history included.
    pub fn select_all(&mut self) {
        let Some(term) = self.focused_term() else { return };
        {
            let mut t = term.lock();
            let start = Point::new(t.topmost_line(), Column(0));
            let end = Point::new(t.bottommost_line(), t.last_column());
            let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
            selection.update(end, Side::Right);
            t.selection = Some(selection);
        }
        self.render();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    #[test]
    fn movement_wraps_and_clamps() {
        let s = |pt, key, ctrl| step(pt, key, ctrl, -10, 4, 80, 5);
        assert_eq!(s(p(0, 0), Key::Left, false), p(-1, 79));
        assert_eq!(s(p(0, 79), Key::Right, false), p(1, 0));
        assert_eq!(s(p(-10, 3), Key::Up, false), p(-10, 3));
        assert_eq!(s(p(4, 3), Key::Down, false), p(4, 3));
        assert_eq!(s(p(0, 3), Key::PageUp, false), p(-5, 3));
        assert_eq!(s(p(0, 3), Key::Home, true), p(-10, 0));
        assert_eq!(s(p(0, 3), Key::End, false), p(0, 79));
        assert_eq!(s(p(4, 79), Key::Right, false), p(4, 79));
    }
}
