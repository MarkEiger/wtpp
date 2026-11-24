//! The Agents view: one pane listing every coding agent in every window
//! (what it's doing, where, and for how long), like herdr's sidebar, with a
//! live preview of the selected one's screen under the list; Enter or a
//! click goes to the agent. It's a terminal with no program behind it: wt++
//! writes the list into it (see `App::draw_agents`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wtpp_input::{Key, Mods};
use wtpp_session::Session;
use wtpp_session::status::State;

use crate::WindowId;
use crate::pane_tree::PaneId;

/// The new-tab menu's and the tab's icon.
pub(crate) const ICON: &str = "🤖";
pub(crate) const TITLE: &str = "Agents";

/// The lines above the list: the heading and the columns' titles.
const TOP: usize = 2;

/// The fewest lines the preview gets (its frame included), or none at all.
const PREVIEW_MIN: usize = 6;

thread_local! {
    /// Every window's agents, as last collected (see `crate::on_agents`).
    static ROWS: std::cell::RefCell<Vec<Row>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) fn rows() -> Vec<Row> {
    ROWS.with(|r| r.borrow().clone())
}

pub(crate) fn set_rows(rows: Vec<Row>) {
    ROWS.with(|r| *r.borrow_mut() = rows);
}

/// The spinners' frame at `now`.
pub(crate) fn frame(now: Instant) -> usize {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    (now.saturating_duration_since(*START.get_or_init(|| now)).as_millis() / 100) as usize
}

/// An agent, as the view lists it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub(crate) window: WindowId,
    pub(crate) pane: PaneId,
    /// What it's called: as renamed, or its tab's title (`#1`, `#2`, … when
    /// its tab has several).
    pub(crate) name: String,
    /// herdr's name for it (`claude`, `codex`, …): its harness.
    pub(crate) agent: &'static str,
    pub(crate) state: Option<State>,
    /// Since when it's been in that state.
    pub(crate) since: Instant,
    /// Its window's number, when there are several.
    pub(crate) window_number: Option<usize>,
    /// Its tab's number.
    pub(crate) tab: usize,
    /// Its profile's name.
    pub(crate) profile: String,
    /// Its folder, short (see `short_dir`), if known.
    pub(crate) dir: String,
    /// What its screen shows.
    pub(crate) preview: Arc<Preview>,
}

impl Row {
    /// Where it is: window, tab, profile.
    fn place(&self) -> String {
        let window = self.window_number.map(|n| format!("Window {n} · "));
        format!("{}Tab {} · {}", window.unwrap_or_default(), self.tab, self.profile)
    }
}

/// A screen, as the preview shows it: its lines' cells, each a character,
/// how many columns it takes, and its look (an index in `styles`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Preview {
    /// SGR parameters; the first is the default look (none).
    pub(crate) styles: Vec<String>,
    pub(crate) lines: Vec<Vec<(char, u8, u16)>>,
    /// Where its cursor is (line, column), if it shows one.
    pub(crate) cursor: Option<(usize, usize)>,
}

impl Preview {
    /// Plain lines (no looks), for the tests.
    #[cfg(test)]
    pub(crate) fn text(lines: &[&str]) -> Preview {
        Preview { styles: vec![String::new()], lines: lines.iter().map(|l| l.chars().map(|c| (c, 1, 0)).collect()).collect(), cursor: None }
    }
}

/// What `session`'s screen shows now, with its colors.
pub(crate) fn capture(session: &Session) -> Preview {
    use wtpp_session::alacritty_terminal::grid::Dimensions;
    use wtpp_session::alacritty_terminal::index::{Column, Line};
    use wtpp_session::alacritty_terminal::term::TermMode;
    use wtpp_session::alacritty_terminal::term::cell::Flags;

    let term = session.term().lock();
    let grid = term.grid();
    let mut styles = vec![String::new()];
    let mut known: HashMap<String, u16> = HashMap::from([(String::new(), 0)]);
    let mut lines = Vec::with_capacity(grid.screen_lines());
    for line in 0..grid.screen_lines() as i32 {
        let row = &grid[Line(line)];
        let mut cells = Vec::with_capacity(grid.columns());
        for c in 0..grid.columns() {
            let cell = &row[Column(c)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let sgr = sgr(cell.fg, cell.bg, cell.flags);
            let style = *known.entry(sgr).or_insert_with_key(|sgr| {
                styles.push(sgr.clone());
                (styles.len() - 1) as u16
            });
            let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            let c = if cell.c.is_control() { ' ' } else { cell.c };
            cells.push((c, width, style));
        }
        // Blank to the end: nothing to show.
        while cells.last().is_some_and(|&(c, _, s)| c == ' ' && s == 0) {
            cells.pop();
        }
        lines.push(cells);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let at = grid.cursor.point;
    let cursor = term.mode().contains(TermMode::SHOW_CURSOR).then_some((at.line.0.max(0) as usize, at.column.0));
    Preview { styles, lines, cursor }
}

/// The SGR parameters for a cell's look (empty: the default).
fn sgr(fg: wtpp_session::alacritty_terminal::vte::ansi::Color, bg: wtpp_session::alacritty_terminal::vte::ansi::Color, flags: wtpp_session::alacritty_terminal::term::cell::Flags) -> String {
    use wtpp_session::alacritty_terminal::term::cell::Flags;
    use wtpp_session::alacritty_terminal::vte::ansi::{Color, NamedColor};

    let mut out: Vec<String> = Vec::new();
    for (flag, code) in [(Flags::BOLD, "1"), (Flags::DIM, "2"), (Flags::ITALIC, "3"), (Flags::ALL_UNDERLINES, "4"), (Flags::INVERSE, "7"), (Flags::STRIKEOUT, "9")] {
        if flags.intersects(flag) {
            out.push(code.into());
        }
    }
    let mut dim = false;
    for (color, base) in [(fg, 30u16), (bg, 40)] {
        match color {
            Color::Named(named) => match named as u16 {
                n @ 0..=7 => out.push((base + n).to_string()),
                n @ 8..=15 => out.push((base + 60 + n - 8).to_string()),
                n @ 259..=266 => {
                    out.push((base + n - 259).to_string());
                    dim |= base == 30;
                }
                _ => dim |= base == 30 && named == NamedColor::DimForeground,
            },
            Color::Indexed(i) => out.push(format!("{};5;{i}", base + 8)),
            Color::Spec(rgb) => out.push(format!("{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b)),
        }
    }
    if dim && !flags.contains(Flags::DIM) {
        out.push("2".into());
    }
    out.join(";")
}

/// A pane showing the view.
#[derive(Default)]
pub(crate) struct View {
    /// The selected agent's pane: it stays selected as others come and go.
    selected: Option<PaneId>,
    /// The first row on screen, when they don't all fit.
    first: usize,
    /// How many rows the list has room for.
    room: usize,
    /// How high the screen is.
    lines: usize,
    /// The rows shown, in order.
    shown: Vec<(WindowId, PaneId)>,
    /// The selected agent's new name, as typed so far (see `Outcome::Rename`).
    editing: Option<String>,
    /// The rows' names, to start renaming from.
    names: Vec<String>,
    /// Typing into the selected agent (from a click on its preview): the
    /// keys are its, not the view's (see `View::typing`).
    typing: bool,
    /// What was last written, not written again unless something changed.
    drawn: String,
}

/// What a key in the view does.
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    /// Nothing (the key isn't the view's).
    None,
    /// The selection moved: draw again.
    Moved,
    /// Go to this agent.
    Go(WindowId, PaneId),
    /// Call this agent this (`None`: what it'd be called by default).
    Rename(WindowId, PaneId, Option<String>),
}

/// Where things go on a screen `lines` high with `rows` agents: how many
/// rows the list has room for, and the preview's first line and height.
fn layout(rows: usize, lines: usize) -> (usize, Option<(usize, usize)>) {
    // The heading, and the help at the bottom.
    let avail = lines.saturating_sub(TOP + 1).max(1);
    if rows == 0 || avail < PREVIEW_MIN + 4 {
        return (avail, None);
    }
    // The list gets what it needs, up to about a third; the preview the rest.
    let list = rows.min((avail / 3).max(3)).min(avail - PREVIEW_MIN - 1);
    let start = TOP + list + 1;
    (list, Some((start, avail - list - 1)))
}

impl View {
    /// Enter the alternate screen (no scrollback to fill) without a cursor.
    pub(crate) const START: &'static [u8] = b"\x1b[?1049h\x1b[?25l";

    fn index(&self) -> Option<usize> {
        let selected = self.selected?;
        self.shown.iter().position(|&(_, p)| p == selected)
    }

    /// The agent the keys go to, if typing into one (see `View::click`).
    pub(crate) fn typing(&self) -> Option<(WindowId, PaneId)> {
        let at = self.index().filter(|_| self.typing)?;
        self.shown.get(at).copied()
    }

    fn select(&mut self, index: usize) -> Outcome {
        match self.shown.get(index) {
            Some(&(_, pane)) if Some(pane) != self.selected => {
                self.selected = Some(pane);
                Outcome::Moved
            }
            _ => Outcome::None,
        }
    }

    pub(crate) fn key(&mut self, key: Key, mods: Mods) -> Outcome {
        if mods.ctrl || mods.alt || mods.meta || self.shown.is_empty() {
            return Outcome::None;
        }
        let last = self.shown.len() - 1;
        let at = self.index().unwrap_or(0);
        if let Some(name) = &mut self.editing {
            match key {
                Key::Char(c) => name.push(c),
                Key::Backspace => {
                    name.pop();
                }
                Key::Escape => self.editing = None,
                Key::Enter => {
                    let name = self.editing.take().unwrap_or_default();
                    let name = name.trim();
                    let (window, pane) = self.shown[at];
                    return Outcome::Rename(window, pane, (!name.is_empty()).then(|| name.to_string()));
                }
                _ => return Outcome::None,
            }
            return Outcome::Moved;
        }
        let page = self.room.max(1);
        match key {
            Key::Char('r') | Key::F(2) => {
                self.editing = Some(self.names.get(at).cloned().unwrap_or_default());
                Outcome::Moved
            }
            Key::Up | Key::Char('k') => self.select(at.saturating_sub(1)),
            Key::Down | Key::Char('j') => self.select((at + 1).min(last)),
            Key::Home | Key::Char('g') => self.select(0),
            Key::End | Key::Char('G') => self.select(last),
            Key::PageUp => self.select(at.saturating_sub(page)),
            Key::PageDown => self.select((at + page).min(last)),
            Key::Enter | Key::Char(' ') => self.go(at),
            _ => Outcome::None,
        }
    }

    /// A click on screen line `line` (`double`: the second of a double
    /// click). On a row, it selects its agent; on the preview, it types into
    /// the selected one, there and then. Double: go to the agent. Anywhere
    /// else, it stops typing.
    pub(crate) fn click(&mut self, line: usize, double: bool) -> Outcome {
        let was = (self.editing.take().is_some(), self.typing);
        self.typing = false;
        let changed = if was != (false, false) { Outcome::Moved } else { Outcome::None };
        if line >= TOP + self.room {
            let (_, preview) = layout(self.shown.len(), self.lines);
            return match (preview, self.index()) {
                (Some((start, height)), Some(at)) if (start..start + height).contains(&line) => {
                    if double {
                        return self.go(at);
                    }
                    self.typing = true;
                    Outcome::Moved
                }
                _ => changed,
            };
        }
        let Some(index) = line.checked_sub(TOP).map(|i| i + self.first).filter(|&i| i < self.shown.len()) else {
            return changed;
        };
        if double {
            return self.go(index);
        }
        match self.select(index) {
            Outcome::None => changed,
            moved => moved,
        }
    }

    fn go(&self, index: usize) -> Outcome {
        self.shown.get(index).map_or(Outcome::None, |&(window, pane)| Outcome::Go(window, pane))
    }

    /// The text that shows `rows` on a `cols`×`lines` screen, or `None` if
    /// that's what it shows already. `frame` turns the spinners.
    pub(crate) fn draw(&mut self, rows: &[Row], cols: usize, lines: usize, now: Instant, frame: usize) -> Option<String> {
        self.shown = rows.iter().map(|r| (r.window, r.pane)).collect();
        self.names = rows.iter().map(|r| r.name.clone()).collect();
        if self.index().is_none() {
            // The one being renamed or typed into is gone.
            self.editing = None;
            self.typing = false;
            self.selected = rows.first().map(|r| r.pane);
        }
        let (room, _) = layout(rows.len(), lines);
        self.room = room;
        self.lines = lines;
        let at = self.index().unwrap_or(0);
        if at < self.first {
            self.first = at;
        } else if at >= self.first + room {
            self.first = at + 1 - room;
        }
        self.first = self.first.min(rows.len().saturating_sub(room));
        let text = render(rows, self.index(), self.editing.as_deref(), self.typing, self.first, cols, lines, now, frame);
        (text != self.drawn).then(|| {
            self.drawn = text.clone();
            text
        })
    }
}

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Icon, its color (SGR) and the word for a state.
fn look(state: Option<State>, frame: usize) -> (char, &'static str, &'static str) {
    match state {
        Some(State::Blocked) => ('!', "1;33", "waiting"),
        Some(State::Working) => (SPINNER[frame % SPINNER.len()], "36", "working"),
        Some(State::Done) => ('✓', "32", "done"),
        Some(State::Error) => ('✗', "31", "error"),
        Some(State::Idle) | None => ('·', "2", "idle"),
    }
}

/// How long, briefly: `14s`, `2m`, `1h`.
fn age(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

/// `text` cut to `width` characters, with `…` if it was longer.
fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    if width > 0 {
        out.push('…');
    }
    out
}

/// `text` cut to `width` and padded to it.
fn cell(text: &str, width: usize) -> String {
    format!("{:<width$}", fit(text, width))
}

/// The table's columns' widths: name, harness, where and folder (0: not
/// shown, for want of room).
struct Columns {
    name: usize,
    agent: usize,
    place: usize,
    dir: usize,
}

impl Columns {
    const STATE: usize = 9;
    const TIME: usize = 4;

    fn new(rows: &[Row], typed: usize, cols: usize) -> Columns {
        let widest = |f: &dyn Fn(&Row) -> usize, title: &str| rows.iter().map(f).max().unwrap_or(0).max(title.chars().count());
        // Names get up to a quarter of the width (more while one is typed).
        let name = widest(&|r| r.name.chars().count(), "NAME").max(typed).min((cols / 4).max(typed.min(cols / 2)));
        let mut agent = widest(&|r| r.agent.chars().count(), "HARNESS");
        // " STATE____  NAME  HARNESS  …  TIME "
        let fixed = |name: usize, agent: usize| 1 + Self::STATE + 2 + name + 2 + agent + 2 + Self::TIME + 1;
        let mut name = name.max(1);
        if fixed(name, agent) > cols {
            name = cols.saturating_sub(fixed(0, agent)).max(1);
            agent = cols.saturating_sub(fixed(name, 0)).clamp(1, agent);
        }
        let rest = cols.saturating_sub(fixed(name, agent));
        let place = widest(&|r| r.place().chars().count(), "WHERE");
        let dir = widest(&|r| r.dir.chars().count(), "FOLDER");
        let (place, dir) = if rest >= place + dir + 4 {
            // The folder takes what's left over.
            (place, rest - place - 4)
        } else if rest >= 15 {
            // The shorter one whole if it takes no more than half.
            let half = (rest - 4) / 2;
            let p = if dir <= half { rest - 4 - dir } else { place.min(half).max(5) };
            (p, rest - 4 - p)
        } else if rest >= 7 {
            (rest - 2, 0)
        } else {
            name += rest;
            (0, 0)
        };
        Columns { name, agent, place, dir }
    }

    /// The columns' texts in a line, with `sel` (SGR) restored after each.
    #[allow(clippy::too_many_arguments)]
    fn line(&self, state: &str, name: &str, agent: &str, place: &str, dir: &str, time: &str, sel: &str) -> String {
        let mut out = format!(" {state}  {name}  \x1b[0;{sel};2m{}\x1b[0;{sel}m", cell(agent, self.agent));
        if self.place > 0 {
            out.push_str(&format!("  {}", cell(place, self.place)));
        }
        if self.dir > 0 {
            out.push_str(&format!("  \x1b[0;{sel};2m{}\x1b[0;{sel}m", cell(dir, self.dir)));
        }
        out.push_str(&format!("  \x1b[0;{sel};2m{time:>w$}\x1b[0;{sel}m ", w = Self::TIME));
        out
    }
}

#[allow(clippy::too_many_arguments)]
fn render(rows: &[Row], selected: Option<usize>, editing: Option<&str>, typing: bool, first: usize, cols: usize, lines: usize, now: Instant, frame: usize) -> String {
    let cols = cols.max(10);
    // No cursor, unless renaming or typing (see `CURSOR`).
    let mut out = String::from("\x1b[?25l\x1b[0m\x1b[H\x1b[2J");
    let mut cursor = None;
    let at = |out: &mut String, line: usize| out.push_str(&format!("\x1b[{};1H", line + 1));

    at(&mut out, 0);
    out.push_str(&format!(" \x1b[1m{TITLE}\x1b[0m"));

    if rows.is_empty() {
        at(&mut out, TOP + 1);
        out.push_str(&fit("  No coding agents running.", cols));
        at(&mut out, TOP + 3);
        out.push_str(&format!("\x1b[2m{}\x1b[0m", fit("  Start one in any tab (claude, codex, opencode, gemini, …)", cols)));
        at(&mut out, TOP + 4);
        out.push_str(&format!("\x1b[2m{}\x1b[0m", fit("  and it shows up here.", cols)));
        return out;
    }

    let typed = editing.map_or(0, |e| e.chars().count() + 1);
    let columns = Columns::new(rows, typed, cols);
    let (room, preview) = layout(rows.len(), lines);

    // The columns' titles, underlined across the width.
    at(&mut out, 1);
    let titles = columns.line(&cell("STATE", Columns::STATE), &cell("NAME", columns.name), "HARNESS", "WHERE", "FOLDER", "TIME", "2;4");
    out.push_str(&format!("\x1b[2;4m{titles}\x1b[0m"));

    for (i, row) in rows.iter().enumerate().skip(first).take(room) {
        let (icon, color, word) = look(row.state, frame);
        let sel = if Some(i) == selected { "7" } else { "0" };
        let state = format!("\x1b[0;{sel};{color}m{icon} {word:<7}\x1b[0;{sel}m");
        let name = match editing.filter(|_| Some(i) == selected) {
            // The end of what's typed, and a cursor.
            Some(typed) => {
                let width = columns.name;
                let shown: String = typed.chars().rev().take(width.saturating_sub(1)).collect::<Vec<_>>().into_iter().rev().collect();
                let fill = width.saturating_sub(shown.chars().count());
                // The terminal's own cursor, blinking, after what's typed.
                cursor = Some((TOP + i - first, 1 + Columns::STATE + 2 + shown.chars().count(), BAR));
                format!("\x1b[0;4m{shown}{}\x1b[0;{sel}m", " ".repeat(fill))
            }
            None => format!("\x1b[0;{sel};1m{}\x1b[0;{sel}m", cell(&row.name, columns.name)),
        };
        let time = age(now.saturating_duration_since(row.since));
        at(&mut out, TOP + i - first);
        out.push_str(&format!("\x1b[0;{sel}m"));
        out.push_str(&columns.line(&state, &name, row.agent, &row.place(), &row.dir, &time, sel));
        out.push_str("\x1b[0m");
    }

    if let (Some((start, height)), Some(row)) = (preview, selected.and_then(|i| rows.get(i)))
        && let Some(at) = draw_preview(&mut out, row, start, height, cols, frame, typing)
    {
        cursor = cursor.or(Some((at.0, at.1, BLOCK)));
    }

    // What the keys do, at the bottom (when there's room under the list).
    if lines > TOP + rows.len().min(room) + 1 {
        at(&mut out, lines - 1);
        let help = if editing.is_some() {
            " Enter rename · Esc cancel · empty: the tab's name"
        } else if typing {
            " Typing into it · click the list to stop · double-click to go to it"
        } else {
            " ↑↓ select · Enter or double-click go to it · click its screen to type · r rename"
        };
        out.push_str(&format!("\x1b[2m{}\x1b[0m", fit(help, cols)));
    }
    if let Some((line, col, shape)) = cursor {
        out.push_str(&format!("\x1b[{};{}H{shape}\x1b[?25h", line + 1, col + 1));
    }
    out
}

/// Cursor shapes (DECSCUSR): a blinking bar, to type a name; a blinking
/// block, where the agent's cursor is.
const BAR: &str = "\x1b[5 q";
const BLOCK: &str = "\x1b[1 q";

/// The selected agent's screen, framed, `height` lines from line `start`:
/// its last lines, as many as fit. `typing`: framed in color, and where the
/// agent's cursor is on screen, if it shows one there.
fn draw_preview(out: &mut String, row: &Row, start: usize, height: usize, cols: usize, frame: usize, typing: bool) -> Option<(usize, usize)> {
    // The frame: dim, or while typing, bright.
    let edge = if typing { "1;36" } else { "2" };
    let at = |out: &mut String, line: usize| out.push_str(&format!("\x1b[{};1H", line + 1));
    let inner = cols.saturating_sub(4);

    // ╭─ name · claude · waiting ───────╮
    let (_, color, word) = look(row.state, frame);
    let title = fit(&row.name, inner.saturating_sub(row.agent.len() + word.len() + 8).max(1));
    let fill = cols.saturating_sub(title.chars().count() + row.agent.chars().count() + word.len() + 11);
    at(out, start);
    out.push_str(&format!(
        "\x1b[{edge}m╭─\x1b[0m \x1b[1m{title}\x1b[0m \x1b[2m·\x1b[0m {} \x1b[2m·\x1b[0m \x1b[{color}m{word}\x1b[0m \x1b[{edge}m{}╮\x1b[0m",
        row.agent,
        "─".repeat(fill),
    ));

    let body = height.saturating_sub(2);
    let preview = &row.preview;
    // Down to its cursor, even on a blank line.
    let shown = preview.lines.len().max(preview.cursor.map_or(0, |c| c.0 + 1));
    let skip = shown.saturating_sub(body);
    for n in 0..body {
        at(out, start + 1 + n);
        out.push_str(&format!("\x1b[{edge}m│\x1b[0m "));
        let mut used = 0;
        if preview.lines.is_empty() && n == 0 {
            let text = fit("Nothing on its screen yet.", inner);
            used = text.chars().count();
            out.push_str(&format!("\x1b[2m{text}\x1b[0m"));
        } else if let Some(line) = preview.lines.get(skip + n) {
            let mut style = 0;
            for &(c, width, s) in line {
                if used + width as usize > inner {
                    break;
                }
                if s != style {
                    let sgr = preview.styles.get(s as usize).map_or("", String::as_str);
                    out.push_str(&format!("\x1b[0;{sgr}m"));
                    style = s;
                }
                out.push(c);
                used += width as usize;
            }
            out.push_str("\x1b[0m");
        }
        out.push_str(&" ".repeat(inner - used));
        out.push_str(&format!(" \x1b[{edge}m│\x1b[0m"));
    }

    at(out, start + height.saturating_sub(1));
    out.push_str(&format!("\x1b[{edge}m╰{}╯\x1b[0m", "─".repeat(cols.saturating_sub(2))));

    let (line, col) = preview.cursor.filter(|_| typing)?;
    (line >= skip && line < skip + body && col < inner).then_some((start + 1 + line - skip, 2 + col))
}

/// A folder, shorter: the home folder as `~`.
pub(crate) fn short_dir(dir: &str) -> String {
    let dir = dir.trim_end_matches(['/', '\\']);
    let parts: Vec<&str> = dir.split(['/', '\\']).collect();
    // /home/me/…, /Users/me/…, C:\Users\me\…
    let home = match parts.as_slice() {
        ["", "home" | "Users", _, ..] => Some(3),
        ["", "root", ..] => Some(2),
        [drive, users, _, ..] if drive.ends_with(':') && users.eq_ignore_ascii_case("users") => Some(3),
        _ => None,
    };
    match home {
        Some(n) => {
            let sep = if dir.contains('\\') { "\\" } else { "/" };
            std::iter::once("~").chain(parts[n..].iter().copied()).collect::<Vec<_>>().join(sep)
        }
        None => dir.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pane: PaneId, agent: &'static str, state: Option<State>, since: Instant) -> Row {
        Row {
            window: 0,
            pane,
            name: format!("t{pane}"),
            agent,
            state,
            since,
            window_number: None,
            tab: pane as usize,
            profile: "Debian".into(),
            dir: "~/x".into(),
            preview: Arc::new(Preview::text(&[&format!("screen of {pane}"), "", "> prompt"][..])),
        }
    }

    /// The screen as text, escape sequences dropped, one string per line.
    fn screen(text: &str, lines: usize) -> Vec<String> {
        let mut screen = vec![String::new(); lines];
        let mut line = 0;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                let mut seq = String::new();
                for c in chars.by_ref() {
                    seq.push(c);
                    if c.is_ascii_alphabetic() && seq.len() > 1 {
                        break;
                    }
                }
                if let Some(pos) = seq.strip_prefix('[').and_then(|s| s.strip_suffix('H')).and_then(|s| s.split(';').next()?.parse::<usize>().ok()) {
                    line = pos - 1;
                }
            } else {
                screen[line].push(c);
            }
        }
        screen
    }

    #[test]
    fn lists_agents() {
        let now = Instant::now();
        let rows = [
            row(1, "claude", Some(State::Blocked), now - Duration::from_secs(125)),
            row(2, "codex", Some(State::Working), now - Duration::from_secs(14)),
            row(3, "opencode", Some(State::Done), now - Duration::from_secs(4000)),
        ];
        let mut view = View::default();
        let text = view.draw(&rows, 70, 10, now, 0).unwrap();
        let lines = screen(&text, 10);
        assert_eq!(lines[0], " Agents", "no summary of their states");
        assert_eq!(lines[1], " STATE      NAME  HARNESS   WHERE           FOLDER               TIME ");
        assert_eq!(lines[2], " ! waiting  t1    claude    Tab 1 · Debian  ~/x                    2m ");
        assert_eq!(lines[3], " ⠋ working  t2    codex     Tab 2 · Debian  ~/x                   14s ");
        assert!(lines[1..5].iter().all(|l| l.chars().count() == 70), "rows fill the width");
        assert!(lines[4].starts_with(" ✓ done     t3    opencode  Tab 3") && lines[4].ends_with(" 1h "));
        assert!(lines.iter().all(|l| l.chars().count() <= 70));
        assert!(lines[9].contains("go to it"));
        // The same again: nothing to write.
        assert_eq!(view.draw(&rows, 70, 10, now, 0), None);
        assert!(view.draw(&rows, 70, 10, now, 1).is_some(), "the spinner turns");
    }

    #[test]
    fn narrow_drops_columns() {
        let now = Instant::now();
        let rows = [row(1, "claude", None, now)];
        let lines = screen(&View::default().draw(&rows, 44, 10, now, 0).unwrap(), 10);
        assert!(lines[1].contains("WHERE") && !lines[1].contains("FOLDER"), "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 44), "{lines:?}");
        let lines = screen(&View::default().draw(&rows, 30, 10, now, 0).unwrap(), 10);
        assert!(lines[1].contains("HARNESS") && !lines[1].contains("WHERE"), "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 30), "{lines:?}");
    }

    #[test]
    fn previews_the_selected_one() {
        let now = Instant::now();
        let rows: Vec<Row> = (1..=2).map(|p| row(p, "claude", Some(State::Working), now)).collect();
        let mut view = View::default();
        let lines = screen(&view.draw(&rows, 50, 20, now, 0).unwrap(), 20);
        let top = lines.iter().position(|l| l.starts_with("╭─")).expect("a preview");
        // The list (just its two rows), a gap, then the preview down to the help.
        assert_eq!(top, TOP + 2 + 1, "{lines:?}");
        assert!(lines[top].contains("t1 · claude · working") && lines[top].ends_with('╮'), "{lines:?}");
        assert!(lines[18].starts_with('╰') && lines[19].contains("go to it"), "{lines:?}");
        // Its screen, from the top.
        assert!(lines[top + 1].contains("screen of 1"));
        assert_eq!(lines[top + 3], format!("│ > prompt{} │", " ".repeat(38)));
        assert!(lines[top..19].iter().all(|l| l.chars().count() == 50), "{lines:?}");
        view.key(Key::Down, Mods::default());
        let lines = screen(&view.draw(&rows, 50, 20, now, 0).unwrap(), 20);
        assert!(lines[top + 1].contains("screen of 2"));
        // A click on it types into the selected one; a double click goes to it.
        assert_eq!(view.typing(), None);
        assert_eq!(view.click(12, false), Outcome::Moved);
        assert_eq!(view.typing(), Some((0, 2)));
        let text = view.draw(&rows, 50, 20, now, 0).unwrap();
        assert!(screen(&text, 20)[19].contains("Typing into it"));
        assert!(!text.contains("\x1b[?25h"), "the agent shows no cursor");
        assert_eq!(view.click(12, true), Outcome::Go(0, 2));
        // A click on the gap stops typing.
        view.click(12, false);
        assert_eq!(view.click(TOP + 2, false), Outcome::Moved);
        assert_eq!(view.typing(), None);
        assert_eq!(view.click(TOP + 2, false), Outcome::None);
        // A click on a row selects it (and stops typing).
        view.click(12, false);
        assert_eq!(view.click(TOP, false), Outcome::Moved);
        assert_eq!((view.typing(), view.index()), (None, Some(0)));
    }

    #[test]
    fn typing_shows_the_agents_cursor() {
        let now = Instant::now();
        let mut rows = vec![row(1, "claude", None, now)];
        Arc::make_mut(&mut rows[0].preview).cursor = Some((2, 2));
        let mut view = View::default();
        let text = view.draw(&rows, 50, 20, now, 0).unwrap();
        assert!(text.starts_with("\x1b[?25l") && !text.contains("\x1b[?25h"), "only while typing");
        view.click(10, false);
        let text = view.draw(&rows, 50, 20, now, 0).unwrap();
        // The preview's frame is on line 5 (titles, one row, a gap): its third
        // line is 8, and its column 2 is 5 (after "│ ").
        assert!(text.ends_with("\x1b[8;5H\x1b[1 q\x1b[?25h"), "{text:?}");
    }

    #[test]
    fn keys_and_clicks() {
        let now = Instant::now();
        let mut rows: Vec<Row> = (1..=3).map(|p| row(p, "codex", Some(State::Idle), now)).collect();
        let mut view = View::default();
        view.draw(&rows, 60, 10, now, 0);
        let plain = Mods::default();
        assert_eq!(view.key(Key::Up, plain), Outcome::None);
        assert_eq!(view.key(Key::Down, plain), Outcome::Moved);
        assert_eq!(view.key(Key::Char('j'), plain), Outcome::Moved);
        assert_eq!(view.key(Key::Down, plain), Outcome::None);
        assert_eq!(view.key(Key::Enter, plain), Outcome::Go(0, 3));
        assert_eq!(view.key(Key::Char('c'), Mods { ctrl: true, ..plain }), Outcome::None);
        // The selected one stays selected when one before it goes.
        rows.remove(0);
        view.draw(&rows, 60, 10, now, 0);
        assert_eq!(view.key(Key::Enter, plain), Outcome::Go(0, 3));
        // A click selects; a double click goes. Line 2 is the first agent;
        // the heading and titles aren't one.
        assert_eq!(view.click(2, false), Outcome::Moved);
        assert_eq!(view.key(Key::Enter, plain), Outcome::Go(0, 2));
        assert_eq!(view.click(2, true), Outcome::Go(0, 2));
        assert_eq!(view.click(1, false), Outcome::None);
        assert_eq!(view.click(9, true), Outcome::None);
    }

    #[test]
    fn renames() {
        let now = Instant::now();
        let rows: Vec<Row> = (1..=2).map(|p| row(p, "claude", None, now)).collect();
        let mut view = View::default();
        view.draw(&rows, 60, 10, now, 0);
        let plain = Mods::default();
        view.key(Key::Down, plain);
        assert_eq!(view.key(Key::Char('r'), plain), Outcome::Moved);
        // Typing edits the name; j and k are letters now.
        view.key(Key::Backspace, plain);
        for c in "jk x".chars() {
            assert_eq!(view.key(Key::Char(c), plain), Outcome::Moved);
        }
        let text = view.draw(&rows, 60, 10, now, 0).unwrap();
        let lines = screen(&text, 10);
        assert!(lines[3].contains("tjk x"), "{lines:?}");
        // The terminal's cursor, a blinking bar, right after what's typed.
        assert!(text.ends_with("\x1b[4;18H\x1b[5 q\x1b[?25h"), "{text:?}");
        assert_eq!(view.key(Key::Enter, plain), Outcome::Rename(0, 2, Some("tjk x".into())));
        // Back to moving and going.
        assert_eq!(view.key(Key::Up, plain), Outcome::Moved);
        // Esc cancels; an empty name goes back to the default.
        view.key(Key::F(2), plain);
        view.key(Key::Escape, plain);
        assert_eq!(view.key(Key::Enter, plain), Outcome::Go(0, 1));
        view.key(Key::Char('r'), plain);
        view.key(Key::Backspace, plain);
        view.key(Key::Backspace, plain);
        assert_eq!(view.key(Key::Enter, plain), Outcome::Rename(0, 1, None));
    }

    #[test]
    fn scrolls_to_the_selection() {
        let now = Instant::now();
        let rows: Vec<Row> = (1..=20).map(|p| row(p, "pi", None, now)).collect();
        let mut view = View::default();
        view.draw(&rows, 50, 6, now, 0);
        view.key(Key::End, Mods::default());
        let lines = screen(&view.draw(&rows, 50, 6, now, 0).unwrap(), 6);
        // Room for 3 rows (6 lines, heading 2, help 1): 18–20.
        assert!(lines[4].contains("Tab 20"), "{lines:?}");
        assert_eq!(view.click(2, true), Outcome::Go(0, 18));
    }

    #[test]
    fn nothing_running() {
        let mut view = View::default();
        let lines = screen(&view.draw(&[], 50, 8, Instant::now(), 0).unwrap(), 8);
        assert!(lines[3].contains("No coding agents running"));
        assert_eq!(view.key(Key::Enter, Mods::default()), Outcome::None);
    }

    #[test]
    fn short_dirs() {
        assert_eq!(short_dir("/home/mark/wtpp"), "~/wtpp");
        assert_eq!(short_dir("/home/mark"), "~");
        assert_eq!(short_dir("/Users/me/src/"), "~/src");
        assert_eq!(short_dir(r"C:\Users\me\code"), r"~\code");
        assert_eq!(short_dir("/opt/x"), "/opt/x");
    }
}
