//! What a program says it's doing, for its tab: working, waiting for you,
//! done, idle or failed.
//!
//! Two ways to say it:
//! - OSC 7501 (Claude Code's): `ESC ] 7501 ; state=working:app=claude-code:id=x:progress=40 ST`,
//!   fields separated by `:`; `state=clear` (all) or `state=clear:id=x`
//!   (one) takes a status back. `ESC ] 7501 ; ? ST` asks whether the
//!   terminal shows them; a program that gets no answer keeps its status in
//!   the title instead (Claude Code's `◐`/`✳` prefix).
//! - OSC 9;4 (ConEmu's and Windows Terminal's progress): `9;4;STATE;PERCENT`,
//!   STATE 0 none, 1 working, 2 error, 3 working (no percentage), 4 paused.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// The states a tab can show, least to most pressing: a tab shows its most
/// pressing pane's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    /// Nothing going on (Claude Code before its first prompt, or stopped).
    Idle,
    /// Finished what it was asked.
    Done,
    Working,
    Error,
    /// Waiting for you: a question, a permission prompt.
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// 0–100 when the program says how far along it is.
    pub progress: Option<u8>,
    /// What it's blocked on (`permission`, `question`, …), when it says.
    pub kind: Option<String>,
    /// The program's name for itself (`app=`).
    pub app: Option<String>,
}

impl Status {
    /// Just a state, nothing more said.
    pub fn of(state: State) -> Status {
        Status { state, progress: None, kind: None, app: None }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Report {
    /// `7501;?`: does the terminal show statuses?
    Query,
    /// A status, by the id the program gave it (`""` without one).
    Set(String, Status),
    /// Take back the status with this id, or all of them.
    Clear(Option<String>),
}

/// The id OSC 9;4 progress goes under: not one OSC 7501 can name (those are
/// letters, digits and `_.+-`), so the two don't overwrite each other.
const PROGRESS_ID: &str = "9;4";

/// The reply to [`Report::Query`].
pub const QUERY_REPLY: &str = "\x1b]7501;?\x1b\\";

/// The status report in an OSC's text (between `ESC ]` and its terminator),
/// if it is one.
pub fn parse(osc: &[u8]) -> Option<Report> {
    let text = std::str::from_utf8(osc).ok()?;
    if let Some(fields) = text.strip_prefix("7501;") {
        return parse_7501(fields);
    }
    if let Some(progress) = text.strip_prefix("9;4") {
        return parse_progress(progress);
    }
    None
}

fn parse_7501(fields: &str) -> Option<Report> {
    if fields.starts_with('?') {
        return Some(Report::Query);
    }
    let mut state = None;
    let mut status = Status::of(State::Idle);
    let mut id = String::new();
    for field in fields.split(':') {
        let (key, value) = field.split_once('=').unwrap_or((field, ""));
        match key {
            "state" => state = Some(value),
            "id" => id = value.chars().filter(|c| c.is_ascii_alphanumeric() || "_.+-".contains(*c)).take(32).collect(),
            "progress" => status.progress = value.parse::<u8>().ok().map(|p| p.min(100)),
            "kind" if !value.is_empty() => status.kind = Some(value.chars().take(32).collect()),
            "app" if !value.is_empty() => status.app = Some(value.chars().take(32).collect()),
            _ => {}
        }
    }
    status.state = match state? {
        "clear" => return Some(Report::Clear(Some(id).filter(|id| !id.is_empty()))),
        "working" | "busy" => State::Working,
        "blocked" | "waiting" => State::Blocked,
        "done" => State::Done,
        "idle" => State::Idle,
        "error" => State::Error,
        _ => return None,
    };
    Some(Report::Set(id, status))
}

/// After `9;4`: `;STATE;PERCENT`, both optional (WT reads a missing one as 0).
fn parse_progress(rest: &str) -> Option<Report> {
    if !(rest.is_empty() || rest.starts_with(';')) {
        return None;
    }
    let mut parts = rest.split(';').skip(1);
    let state = parts.next().unwrap_or("");
    let percent = parts.next().and_then(|p| p.parse::<u32>().ok()).map(|p| p.min(100) as u8);
    let state = match state {
        "" | "0" => return Some(Report::Clear(Some(PROGRESS_ID.into()))),
        "1" | "4" => State::Working,
        "2" => State::Error,
        "3" => return Some(Report::Set(PROGRESS_ID.into(), Status::of(State::Working))),
        _ => return None,
    };
    Some(Report::Set(PROGRESS_ID.into(), Status { progress: percent, ..Status::of(state) }))
}

/// A pane's statuses, by id.
#[derive(Default, Debug)]
pub struct Statuses(BTreeMap<String, Status>);

impl Statuses {
    /// Take in a report; true if what the pane shows may have changed.
    pub fn apply(&mut self, report: Report) -> bool {
        match report {
            Report::Query => false,
            Report::Set(id, status) => self.0.insert(id, status.clone()).as_ref() != Some(&status),
            Report::Clear(Some(id)) => self.0.remove(&id).is_some(),
            Report::Clear(None) => {
                let had = !self.0.is_empty();
                self.0.clear();
                had
            }
        }
    }

    /// The most pressing status, the first one of those if several are.
    pub fn current(&self) -> Option<Status> {
        self.0.values().rev().max_by_key(|s| s.state).cloned()
    }
}

/// A command running in a shell, as its prompt hooks tell it (OSC 133, the
/// FinalTerm marks): `133;D;CODE` when a command ended and the prompt is
/// back (wt++'s hooks send it at every prompt, the first one too), `133;A`
/// for a prompt alone, `133;C` when a command starts. Shells that can't say
/// when a command starts get [`Commands::entered`] when Enter is pressed at
/// their prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum Mark {
    Prompt,
    Started,
    Ended(Option<i32>),
}

/// The OSC 133 mark in an OSC's text, if it is one.
pub fn parse_mark(osc: &[u8]) -> Option<Mark> {
    let text = std::str::from_utf8(osc).ok()?;
    let rest = text.strip_prefix("133;")?;
    let mut parts = rest.split(';');
    match parts.next()? {
        "A" | "B" => Some(Mark::Prompt),
        "C" => Some(Mark::Started),
        "D" => Some(Mark::Ended(parts.next().and_then(|c| c.trim().parse().ok()))),
        _ => None,
    }
}

/// A command shows as working once it has run this long (a quick `ls`
/// doesn't flash the spinner). How it ended shows until its tab is seen
/// (wt++ decides that).
pub const SHOW_RUNNING_AFTER: Duration = Duration::from_millis(500);

/// What the shell's commands are doing.
#[derive(Debug, Default)]
pub struct Commands {
    /// The shell is at its prompt (it said so, and nothing was entered since).
    at_prompt: bool,
    running_since: Option<Instant>,
    /// How the last command ended.
    ended: Option<State>,
    /// How many prompts there have been: one that's newer than something
    /// means that something (a command, a program) has ended.
    prompts: u64,
}

impl Commands {
    /// Take in a mark; true if what the pane shows may have changed.
    pub fn apply(&mut self, mark: Mark, now: Instant) -> bool {
        match mark {
            Mark::Started => {
                self.at_prompt = false;
                if self.running_since.is_none() {
                    self.running_since = Some(now);
                    self.ended = None;
                }
                true
            }
            Mark::Prompt | Mark::Ended(_) => {
                let was = (self.at_prompt, self.running_since, self.ended);
                if !self.at_prompt || self.running_since.is_some() {
                    self.prompts += 1;
                }
                self.at_prompt = true;
                if self.running_since.take().is_some() {
                    let code = match mark {
                        Mark::Ended(code) => code,
                        _ => None,
                    };
                    self.ended = match code {
                        None | Some(0) => Some(State::Done),
                        // Ctrl+C: you stopped it, there's nothing to tell you.
                        Some(130) | Some(-1073741510) => None,
                        Some(_) => Some(State::Error),
                    };
                }
                was != (self.at_prompt, self.running_since, self.ended)
            }
        }
    }

    /// Enter was pressed: at the prompt, that runs a command. True if it did.
    pub fn entered(&mut self, now: Instant) -> bool {
        if !self.at_prompt {
            return false;
        }
        self.apply(Mark::Started, now)
    }

    /// What to show, and when to look again (a command not yet running long
    /// enough to show).
    pub fn current(&self, now: Instant) -> (Option<State>, Option<Duration>) {
        match self.running_since {
            Some(since) => {
                let ran = now.saturating_duration_since(since);
                if ran >= SHOW_RUNNING_AFTER { (Some(State::Working), None) } else { (None, Some(SHOW_RUNNING_AFTER - ran)) }
            }
            None => (self.ended, None),
        }
    }

    /// Forget it all: the shell is gone.
    pub fn clear(&mut self) {
        *self = Commands { prompts: self.prompts + 1, ..Commands::default() };
    }

    /// Is the shell at its prompt (it said so, and nothing was entered since)?
    pub fn at_prompt(&self) -> bool {
        self.at_prompt
    }

    /// How many times the shell came back to its prompt (see `prompts`).
    pub fn prompts(&self) -> u64 {
        self.prompts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks() {
        assert_eq!(parse_mark(b"133;A"), Some(Mark::Prompt));
        assert_eq!(parse_mark(b"133;B"), Some(Mark::Prompt));
        assert_eq!(parse_mark(b"133;C"), Some(Mark::Started));
        assert_eq!(parse_mark(b"133;D;0"), Some(Mark::Ended(Some(0))));
        assert_eq!(parse_mark(b"133;D;127"), Some(Mark::Ended(Some(127))));
        assert_eq!(parse_mark(b"133;D"), Some(Mark::Ended(None)));
        assert_eq!(parse_mark(b"133;D;"), Some(Mark::Ended(None)));
        assert_eq!(parse_mark(b"133;A;cl=m;aid=1"), Some(Mark::Prompt));
        assert_eq!(parse_mark(b"133;Z"), None);
        assert_eq!(parse_mark(b"1337;D"), None);
    }

    #[test]
    fn commands_by_enter_at_the_prompt() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut c = Commands::default();
        // No prompt seen: Enter means nothing (a shell without the hooks).
        assert!(!c.entered(t0));
        assert_eq!(c.current(t0), (None, None));
        assert!(c.apply(Mark::Ended(Some(0)), t0));
        assert_eq!(c.current(t0), (None, None));
        // `sleep 3`: nothing at first, then working, then done.
        assert!(c.entered(at(0)));
        assert_eq!(c.current(at(100)), (None, Some(Duration::from_millis(400))));
        assert_eq!(c.current(at(600)), (Some(State::Working), None));
        // Enter while it runs is typed into it, not a new command.
        assert!(!c.entered(at(700)));
        c.apply(Mark::Ended(Some(0)), at(3000));
        assert_eq!(c.current(at(3000)), (Some(State::Done), None));
        // A quick one too: it never showed running, but it ended.
        c.entered(at(4000));
        assert_eq!(c.current(at(4010)), (None, Some(Duration::from_millis(490))));
        c.apply(Mark::Ended(Some(0)), at(4050));
        assert_eq!(c.current(at(4050)), (Some(State::Done), None));
        // A slow failure is an error; one stopped with Ctrl+C isn't.
        c.entered(at(5000));
        c.apply(Mark::Ended(Some(2)), at(7000));
        assert_eq!(c.current(at(7000)), (Some(State::Error), None));
        c.entered(at(8000));
        c.apply(Mark::Ended(Some(130)), at(9000));
        assert_eq!(c.current(at(9000)), (None, None));
        // cmd: no exit code.
        c.entered(at(10000));
        c.apply(Mark::Ended(None), at(12000));
        assert_eq!(c.current(at(12000)), (Some(State::Done), None));
    }

    #[test]
    fn commands_by_start_mark() {
        let t0 = Instant::now();
        let mut c = Commands::default();
        c.apply(Mark::Started, t0);
        assert_eq!(c.current(t0 + Duration::from_secs(1)), (Some(State::Working), None));
        c.apply(Mark::Prompt, t0 + Duration::from_secs(2));
        assert_eq!(c.current(t0 + Duration::from_secs(2)), (Some(State::Done), None));
        c.clear();
        assert_eq!(c.current(t0), (None, None));
        assert!(!c.entered(t0));
    }

    fn set(id: &str, state: State) -> Option<Report> {
        Some(Report::Set(id.into(), Status::of(state)))
    }

    #[test]
    fn claude_code_reports() {
        assert_eq!(parse(b"7501;?"), Some(Report::Query));
        assert_eq!(
            parse(b"7501;state=working:app=claude-code:progress=40:msg=aGk="),
            Some(Report::Set("".into(), Status { state: State::Working, progress: Some(40), kind: None, app: Some("claude-code".into()) }))
        );
        assert_eq!(
            parse(b"7501;state=blocked:app=claude-code:id=a1:kind=permission:title=eA=="),
            Some(Report::Set("a1".into(), Status { state: State::Blocked, progress: None, kind: Some("permission".into()), app: Some("claude-code".into()) }))
        );
        assert_eq!(parse(b"7501;state=done:app=claude-code"), Some(Report::Set("".into(), Status { app: Some("claude-code".into()), ..Status::of(State::Done) })));
        assert_eq!(parse(b"7501;state=idle"), set("", State::Idle));
        assert_eq!(parse(b"7501;state=error"), set("", State::Error));
        assert_eq!(parse(b"7501;state=clear"), Some(Report::Clear(None)));
        assert_eq!(parse(b"7501;state=clear:id=a1"), Some(Report::Clear(Some("a1".into()))));
        // Unknown states and garbage are ignored, not guessed at.
        assert_eq!(parse(b"7501;state=dancing"), None);
        assert_eq!(parse(b"7501;"), None);
        assert_eq!(parse(b"75011;state=done"), None);
        // An id can't pose as OSC 9;4's.
        assert_eq!(parse(b"7501;state=done:id=9;4"), set("94", State::Done));
    }

    #[test]
    fn progress_reports() {
        assert_eq!(parse(b"9;4;1;50"), Some(Report::Set(PROGRESS_ID.into(), Status { progress: Some(50), ..Status::of(State::Working) })));
        assert_eq!(parse(b"9;4;3;"), set(PROGRESS_ID, State::Working));
        assert_eq!(parse(b"9;4;3"), set(PROGRESS_ID, State::Working));
        assert_eq!(parse(b"9;4;2;100"), Some(Report::Set(PROGRESS_ID.into(), Status { progress: Some(100), ..Status::of(State::Error) })));
        assert_eq!(parse(b"9;4;1;250"), Some(Report::Set(PROGRESS_ID.into(), Status { progress: Some(100), ..Status::of(State::Working) })));
        assert_eq!(parse(b"9;4;0;0"), Some(Report::Clear(Some(PROGRESS_ID.into()))));
        assert_eq!(parse(b"9;4"), Some(Report::Clear(Some(PROGRESS_ID.into()))));
        // Other OSC 9s aren't progress.
        assert_eq!(parse(b"9;9;/home/me"), None);
        assert_eq!(parse(b"9;Build done"), None);
        assert_eq!(parse(b"9;42"), None);
        assert_eq!(parse(b"9;4;7"), None);
    }

    #[test]
    fn most_pressing_wins() {
        let mut s = Statuses::default();
        assert_eq!(s.current(), None);
        assert!(s.apply(parse(b"7501;state=working").unwrap()));
        assert!(!s.apply(parse(b"7501;state=working").unwrap()));
        assert!(s.apply(parse(b"7501;state=blocked:id=sub").unwrap()));
        assert_eq!(s.current().unwrap().state, State::Blocked);
        assert!(s.apply(parse(b"7501;state=clear:id=sub").unwrap()));
        assert_eq!(s.current().unwrap().state, State::Working);
        assert!(s.apply(parse(b"9;4;2").unwrap()));
        assert_eq!(s.current().unwrap().state, State::Error);
        assert!(s.apply(parse(b"7501;state=clear").unwrap()));
        assert_eq!(s.current(), None);
        assert!(!s.apply(parse(b"7501;state=clear").unwrap()));
        assert!(!s.apply(Report::Query));
    }
}
