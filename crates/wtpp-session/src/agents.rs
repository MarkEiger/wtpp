//! Coding agents (Claude Code, Codex, OpenCode, Pi, Gemini, …) in a plain
//! tab, without herdr: which one is running, and what it's doing, for its
//! tab's status.
//!
//! Which agent: by the command typed at the shell's prompt (`codex`, `npx
//! @openai/codex`, `claude --resume`), which works in every shell with
//! wt++'s prompt hooks, WSL included; by the program a profile runs; and on
//! Linux, macOS and Windows by the programs the shell runs (see
//! [`crate::ssh::foreground_programs`]), which also finds one started by an
//! alias or a script.
//!
//! What it's doing: by its screen, its title and its progress (OSC 9;4),
//! matched against the agent's rules: herdr's detection manifests
//! (<https://github.com/herdrdev/herdr>, Apache-2.0, in `agents/`), read
//! the way herdr reads them. A rule names a part of the screen (the whole
//! of it, its last lines, …), what has to be there (text, regular
//! expressions, nested `all`/`any`/`not`), and the state that means:
//! working, blocked (waiting for you), or idle; the highest priority match
//! wins. Idle after working is done, as on a tab whose Claude Code finished.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Deserialize;

use crate::status::{State, Status};

/// An agent wt++ knows.
#[derive(Debug, PartialEq, Eq)]
pub struct Agent {
    /// herdr's name for it (`claude`, `codex`, …), shown as the status's app.
    pub id: &'static str,
    /// Its programs' names (lowercase, without `.exe`), and its npm packages'.
    names: &'static [&'static str],
    /// Its manifest (in `agents/`).
    manifest: &'static str,
}

macro_rules! agent {
    ($id:literal, [$($name:literal),*], $file:literal) => {
        Agent { id: $id, names: &[$($name),*], manifest: include_str!(concat!("../agents/", $file)) }
    };
}

/// Every agent, with herdr's names for each (`cursor-agent` is Cursor's,
/// `agy` Antigravity's). OMP (oh-my-pi) is a fork of Pi, and reads as Pi.
pub static AGENTS: &[Agent] = &[
    agent!("claude", ["claude", "claude-code"], "claude.toml"),
    agent!("codex", ["codex"], "codex.toml"),
    agent!("opencode", ["opencode", "opencode2", "open-code", "opencode-ai"], "opencode.toml"),
    agent!("pi", ["pi", "pi-coding-agent"], "pi.toml"),
    agent!("omp", ["omp", "oh-my-pi"], "pi.toml"),
    agent!("gemini", ["gemini", "gemini-cli"], "gemini.toml"),
    agent!("cursor", ["cursor-agent"], "cursor.toml"),
    agent!("copilot", ["copilot", "github-copilot", "ghcs"], "github-copilot.toml"),
    agent!("amp", ["amp", "amp-local"], "amp.toml"),
    agent!("droid", ["droid"], "droid.toml"),
    agent!("agy", ["agy", "antigravity", "antigravity-cli"], "antigravity.toml"),
    agent!("cline", ["cline"], "cline.toml"),
    agent!("devin", ["devin", "devin-cli"], "devin.toml"),
    agent!("grok", ["grok", "grok-build"], "grok.toml"),
    agent!("hermes", ["hermes", "hermes-agent"], "hermes.toml"),
    agent!("kilo", ["kilo", "kilo-code", "kilocode"], "kilo.toml"),
    agent!("kimi", ["kimi", "kimi-code", "kimi-cli"], "kimi.toml"),
    agent!("kiro", ["kiro-cli", "kiro"], "kiro.toml"),
    agent!("letta", ["letta", "letta-code"], "letta.toml"),
    agent!("maki", ["maki"], "maki.toml"),
    agent!("muse", ["muse", "muse-code", "muse-cli"], "muse.toml"),
    agent!("qodercli", ["qodercli", "qoderclicn", "qoder", "qodercn"], "qodercli.toml"),
    agent!("qwen", ["qwen", "qwen-code"], "qwen.toml"),
];

/// The agent by this program or package name (`codex`, `/usr/bin/codex`,
/// `C:\…\claude.exe`, `@openai/codex@latest`, `cli.js` in its package).
pub fn agent_named(name: &str) -> Option<&'static Agent> {
    let name = name.trim_matches(['"', '\'']);
    // A package with a version: `@openai/codex@0.5`, `opencode-ai@latest`.
    let name = match name.strip_prefix('@') {
        Some(rest) => rest.split_once('@').map_or(rest, |(n, _)| n),
        None => name.split_once('@').map_or(name, |(n, _)| n),
    };
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    let mut stem = file.as_str();
    for ext in [".exe", ".cmd", ".bat", ".ps1", ".js", ".mjs", ".cjs", ".sh"] {
        if let Some(s) = stem.strip_suffix(ext) {
            stem = s;
            break;
        }
    }
    AGENTS.iter().find(|a| a.names.contains(&stem))
}

/// Programs that run another one named by an argument.
fn is_runner(word: &str) -> bool {
    let word = word.rsplit(['/', '\\']).next().unwrap_or(word).to_ascii_lowercase();
    let word = word.strip_suffix(".exe").or_else(|| word.strip_suffix(".cmd")).unwrap_or(&word).to_string();
    matches!(word.as_str(), "npx" | "bunx" | "pnpx" | "uvx" | "node" | "bun" | "deno" | "tsx" | "python" | "python3" | "sh" | "bash" | "dash" | "zsh" | "env" | "sudo" | "exec" | "command" | "builtin" | "nohup" | "time" | "nice" | "stdbuf" | "caffeinate" | "&")
}

/// The agent a command line typed at a prompt starts, if it does: its
/// program, past `VAR=value`s and what only runs another program (`sudo`,
/// `npx`, `pnpm dlx`, `node`, …).
pub fn agent_in_command(line: &str) -> Option<&'static Agent> {
    let words: Vec<&str> = line.split_whitespace().map(|w| w.trim_matches(['"', '\''])).collect();
    let mut i = 0;
    while let Some(&word) = words.get(i) {
        let next = words.get(i + 1).copied();
        if word.contains('=') && !word.starts_with('-') && !word.contains(['/', '\\']) {
            i += 1; // VAR=value
        } else if word.starts_with('-') {
            i += 1; // a runner's option
        } else if is_runner(word) {
            i += 1;
        } else if matches!(word.rsplit(['/', '\\']).next().map(str::to_ascii_lowercase).as_deref(), Some("wsl" | "wsl.exe")) {
            // What it runs comes after `--` or `-e` (`wsl -d Debian -- codex`).
            let run = words[i..].iter().position(|w| matches!(*w, "--" | "-e" | "--exec"));
            i += run? + 1;
        } else if matches!((word, next), ("pnpm" | "yarn", Some("dlx" | "exec")) | ("npm", Some("exec" | "x")) | ("pipx" | "uv", Some("run" | "tool")) | ("deno", Some("run"))) {
            i += 2;
        } else {
            return agent_named(word);
        }
    }
    None
}

/// The agent a running program is, by its command line (program first): its
/// name, or for a runtime (node, bun, python) the script or package it runs.
pub fn agent_in_argv(argv: &[String]) -> Option<&'static Agent> {
    let (program, args) = argv.split_first()?;
    if let Some(agent) = agent_named(program) {
        return Some(agent);
    }
    if !is_runner(program) && !program.to_ascii_lowercase().contains("node") {
        return None;
    }
    for arg in args.iter().filter(|a| !a.starts_with('-')).take(3) {
        // A package's script: `…/node_modules/@openai/codex/bin/codex.js`.
        let parts: Vec<&str> = arg.split(['/', '\\']).collect();
        if let Some(at) = parts.iter().rposition(|p| *p == "node_modules") {
            let package = parts.get(at + 1).filter(|p| !p.starts_with('@')).or_else(|| parts.get(at + 2));
            if let Some(agent) = package.and_then(|p| agent_named(p)) {
                return Some(agent);
            }
        }
        if let Some(agent) = agent_named(arg) {
            return Some(agent);
        }
    }
    None
}

// The manifests (herdr's format).

#[derive(Deserialize)]
struct ManifestFile {
    id: String,
    #[serde(default)]
    rules: Vec<RuleFile>,
}

#[derive(Deserialize)]
struct RuleFile {
    id: String,
    state: Option<String>,
    #[serde(default)]
    priority: i32,
    #[serde(default = "whole_recent")]
    region: String,
    #[serde(default)]
    visible_idle: bool,
    #[serde(default)]
    skip_state_update: bool,
    #[serde(flatten)]
    gate: GateFile,
}

fn whole_recent() -> String {
    "whole_recent".into()
}

#[derive(Deserialize, Default)]
struct GateFile {
    #[serde(default)]
    all: Vec<GateFile>,
    #[serde(default)]
    any: Vec<GateFile>,
    #[serde(default, rename = "not")]
    not_gate: Vec<GateFile>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

/// What a rule says the agent is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Idle,
    Working,
    Blocked,
    /// It's there, doing something that says nothing (a transcript viewer).
    Unknown,
}

struct Rule {
    id: String,
    state: Seen,
    priority: i32,
    region: String,
    visible_idle: bool,
    skip: bool,
    gate: Gate,
}

struct Gate {
    all: Vec<Gate>,
    any: Vec<Gate>,
    not_gate: Vec<Gate>,
    /// Lowercase: matched without regard to case.
    contains: Vec<String>,
    regex: Vec<Regex>,
    line_regex: Vec<Regex>,
}

fn compile(gate: GateFile) -> Result<Gate, regex::Error> {
    let regexes = |list: Vec<String>| list.iter().map(|r| Regex::new(r)).collect::<Result<Vec<_>, _>>();
    let gates = |list: Vec<GateFile>| list.into_iter().map(compile).collect::<Result<Vec<_>, _>>();
    Ok(Gate {
        all: gates(gate.all)?,
        any: gates(gate.any)?,
        not_gate: gates(gate.not_gate)?,
        contains: gate.contains.iter().map(|c| c.to_lowercase()).collect(),
        regex: regexes(gate.regex)?,
        line_regex: regexes(gate.line_regex)?,
    })
}

fn parse_manifest(text: &str) -> Result<Vec<Rule>, String> {
    let file: ManifestFile = toml::from_str(text).map_err(|e| e.to_string())?;
    file.rules
        .into_iter()
        .map(|rule| {
            let state = match rule.state.as_deref() {
                Some("idle") => Seen::Idle,
                Some("working") => Seen::Working,
                Some("blocked") => Seen::Blocked,
                _ => Seen::Unknown,
            };
            let gate = compile(rule.gate).map_err(|e| format!("{}: rule {}: {e}", file.id, rule.id))?;
            Ok(Rule { id: rule.id, state, priority: rule.priority, region: rule.region, visible_idle: rule.visible_idle, skip: rule.skip_state_update, gate })
        })
        .collect()
}

/// Each agent's rules, read once.
fn rules(agent: &Agent) -> &'static [Rule] {
    static RULES: OnceLock<Vec<Vec<Rule>>> = OnceLock::new();
    let all = RULES.get_or_init(|| {
        AGENTS
            .iter()
            .map(|a| {
                parse_manifest(a.manifest).unwrap_or_else(|e| {
                    log::warn!("couldn't read {}'s detection rules: {e}", a.id);
                    Vec::new()
                })
            })
            .collect()
    });
    let at = AGENTS.iter().position(|a| a == agent).unwrap_or(0);
    &all[at]
}

impl Gate {
    fn matches(&self, text: &str, lower: &str) -> bool {
        self.contains.iter().all(|c| lower.contains(c.as_str()))
            && self.regex.iter().all(|r| r.is_match(text))
            && self.line_regex.iter().all(|r| text.lines().any(|l| r.is_match(l)))
            && self.all.iter().all(|g| g.matches(text, lower))
            && (self.any.is_empty() || self.any.iter().any(|g| g.matches(text, lower)))
            && !self.not_gate.iter().any(|g| g.matches(text, lower))
    }
}

/// What the agent's screen says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detection {
    /// What the best rule said; `None` if none matched.
    pub seen: Option<Seen>,
    /// It says outright it's at its prompt (not merely nothing else).
    pub visible_idle: bool,
    /// The screen isn't its live one (a transcript): keep what was known.
    pub skip: bool,
}

/// What an agent is doing, by its rules: the screen as text (its rows, top
/// to bottom), its title and its last progress report (OSC 9's text after
/// `9;`, e.g. `4;3`).
pub fn detect(agent: &Agent, screen: &str, title: &str, progress: &str) -> Detection {
    let mut best: Option<&Rule> = None;
    for rule in rules(agent) {
        let text = region(&rule.region, screen, title, progress);
        if rule.gate.matches(text, &text.to_lowercase()) && best.is_none_or(|b| rule.priority > b.priority) {
            best = Some(rule);
        }
    }
    if let Some(rule) = best {
        log::trace!("{}: rule {} ({:?})", agent.id, rule.id, rule.state);
    }
    Detection {
        seen: best.map(|r| r.state),
        visible_idle: best.is_some_and(|r| r.visible_idle && r.state == Seen::Idle),
        skip: best.is_some_and(|r| r.skip),
    }
}

/// The part of the screen (or title, or progress) a rule looks at; herdr's
/// regions.
fn region<'a>(spec: &str, screen: &'a str, title: &'a str, progress: &'a str) -> &'a str {
    let spec = spec.trim();
    let count = |name: &str| spec.strip_prefix(name)?.strip_prefix('(')?.strip_suffix(')')?.parse::<usize>().ok();
    match spec {
        "osc_title" => title,
        "osc_progress" => progress,
        "whole_recent" => screen,
        "after_last_prompt_marker" => after_last_prompt_marker(screen),
        "before_current_prompt_marker" => before_current_prompt_marker(screen),
        "whole_recent_without_current_prompt_marker" => {
            if current_prompt_index(&screen.lines().collect::<Vec<_>>()).is_some() { "" } else { screen }
        }
        "current_prompt_block_marker" => current_prompt_block_marker(screen).unwrap_or(""),
        "after_current_prompt_block_marker" => after_current_prompt_block_marker(screen).unwrap_or(""),
        "prompt_box_body" => prompt_box_body(screen).unwrap_or(""),
        "above_prompt_box" => above_prompt_box(screen),
        "last_non_empty_above_prompt_box" => last_non_empty_line(above_prompt_box(screen)),
        "after_last_horizontal_rule" => after_last_horizontal_rule(screen),
        _ => {
            if let Some(n) = count("bottom_lines") {
                bottom_lines(screen, n)
            } else if let Some(n) = count("bottom_non_empty_lines") {
                bottom_non_empty_lines(screen, n)
            } else if let Some(n) = count("top_non_empty_lines").filter(|&n| n > 0) {
                top_non_empty_lines(screen, n)
            } else {
                ""
            }
        }
    }
}

fn line_start(content: &str, lines: &[&str], index: usize) -> usize {
    lines[..index.min(lines.len())].iter().map(|l| l.len() + 1).sum::<usize>().min(content.len())
}

fn from_line<'a>(content: &'a str, lines: &[&str], index: usize) -> &'a str {
    &content[line_start(content, lines, index)..]
}

fn bottom_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    from_line(content, &lines, lines.len().saturating_sub(count))
}

fn bottom_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.iter().enumerate().rev().filter(|(_, l)| !l.trim().is_empty()).take(count).last().map(|(i, _)| i);
    start.map_or("", |i| from_line(content, &lines, i))
}

fn top_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let end = lines.iter().enumerate().filter(|(_, l)| !l.trim().is_empty()).take(count).last().map(|(i, _)| i);
    end.map_or("", |i| &content[..line_start(content, &lines, i + 1)])
}

/// Codex's prompt line (`› …`), and its turn's blocks (`•`, `■`, `✗`, `✓`).
fn prompt_line(line: &str) -> bool {
    line == "›" || line.starts_with("› ")
}

fn block_marker_line(line: &str) -> bool {
    line.starts_with(['•', '■', '✗', '✓'])
}

fn current_prompt_index(lines: &[&str]) -> Option<usize> {
    let at = lines.iter().rposition(|l| prompt_line(l))?;
    (!lines[at + 1..].iter().any(|l| block_marker_line(l))).then_some(at)
}

fn after_last_prompt_marker(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    match lines.iter().rposition(|l| prompt_line(l)) {
        Some(at) => from_line(content, &lines, at + 1),
        None => content,
    }
}

fn before_current_prompt_marker(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    match current_prompt_index(&lines) {
        Some(at) => &content[..line_start(content, &lines, at)],
        None => content,
    }
}

fn current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let at = current_prompt_index(&lines)?;
    lines[..at].iter().rev().find(|l| block_marker_line(l)).copied()
}

fn after_current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let at = current_prompt_index(&lines)?;
    let block = lines[..at].iter().rposition(|l| block_marker_line(l))?;
    Some(from_line(content, &lines, block))
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    let rule = trimmed.chars().take_while(|&c| c == '─').count();
    if rule == 0 {
        return false;
    }
    let rest = trimmed.char_indices().nth(rule).map_or("", |(i, _)| &trimmed[i..]);
    rest.trim_start().is_empty() || rule >= 3
}

/// The input box at the bottom (between the last two rules).
fn prompt_box_top(lines: &[&str]) -> Option<usize> {
    (0..lines.len()).rev().filter(|&i| is_horizontal_rule(lines[i])).nth(1)
}

fn prompt_box_body(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let top = prompt_box_top(&lines)?;
    let end = lines[top + 1..].iter().position(|l| is_horizontal_rule(l)).map_or(lines.len(), |r| top + 1 + r);
    Some(&content[line_start(content, &lines, top + 1)..line_start(content, &lines, end)])
}

fn above_prompt_box(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    match prompt_box_top(&lines) {
        Some(top) => &content[..line_start(content, &lines, top)],
        None => content,
    }
}

fn after_last_horizontal_rule(content: &str) -> &str {
    let mut end = 0;
    let mut offset = 0;
    for line in content.lines() {
        offset += line.len() + 1;
        if is_horizontal_rule(line) {
            end = offset.min(content.len());
        }
    }
    &content[end..]
}

fn last_non_empty_line(content: &str) -> &str {
    content.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("")
}

/// How long the screen has to keep saying idle, after it said working,
/// before the agent counts as finished (unless it shows its prompt outright):
/// screens flicker between a turn's steps.
const IDLE_AFTER_WORK: Duration = Duration::from_millis(900);

/// One pane's agent, and what it's doing (see the module).
#[derive(Debug, Default)]
pub struct Tracker {
    agent: Option<&'static Agent>,
    state: Option<State>,
    /// Since when the screen has said idle while it was working.
    idle_since: Option<Instant>,
}

impl Tracker {
    /// The agent that's running now; true if that's news.
    pub fn start(&mut self, agent: &'static Agent) -> bool {
        if self.agent == Some(agent) {
            return false;
        }
        log::info!("{} is running", agent.id);
        *self = Tracker { agent: Some(agent), ..Default::default() };
        true
    }

    /// It's gone; true if it was there.
    pub fn stop(&mut self) -> bool {
        let had = self.agent.is_some();
        *self = Tracker::default();
        had
    }

    pub fn agent(&self) -> Option<&'static Agent> {
        self.agent
    }

    /// What it's doing, for its tab. Nothing until its screen matched one of
    /// its rules: a program that only shares an agent's name (an editor
    /// called `kilo`) shows nothing.
    pub fn status(&self) -> Option<Status> {
        let state = self.state?;
        Some(Status { state, progress: None, kind: None, app: Some(self.agent?.id.to_string()) })
    }

    /// Look at its screen again; true if what it's doing changed.
    pub fn update(&mut self, screen: &str, title: &str, progress: &str, now: Instant) -> bool {
        let Some(agent) = self.agent else { return false };
        let detection = detect(agent, screen, title, progress);
        let new = match detection.seen {
            _ if detection.skip => return false,
            Some(Seen::Unknown) => return false,
            // Nothing matched: idle (as herdr has it), once it showed itself.
            None if self.state.is_none() => return false,
            None => Some(Seen::Idle),
            seen => seen,
        };
        let state = match new {
            Some(Seen::Working) => State::Working,
            Some(Seen::Blocked) => State::Blocked,
            _ => {
                let was_working = self.state == Some(State::Working);
                if was_working && !detection.visible_idle {
                    let since = *self.idle_since.get_or_insert(now);
                    if now.saturating_duration_since(since) < IDLE_AFTER_WORK {
                        return false;
                    }
                }
                // Finished what it was working on: done until it starts again.
                if was_working || self.state == Some(State::Done) { State::Done } else { State::Idle }
            }
        };
        self.idle_since = None;
        let changed = self.state != Some(state);
        self.state = Some(state);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_read() {
        for agent in AGENTS {
            let rules = parse_manifest(agent.manifest).unwrap_or_else(|e| panic!("{}: {e}", agent.id));
            assert!(!rules.is_empty(), "{}", agent.id);
            // Every region is one we read: a non-empty screen has something in it.
            let screen = "a\n────\n› x\n• y\n────\nz";
            for rule in &rules {
                let known = !region(&rule.region, screen, "t", "p").is_empty() || rule.region.contains("prompt_marker");
                assert!(known, "{}: {} has region {}", agent.id, rule.id, rule.region);
            }
        }
    }

    fn id(agent: Option<&Agent>) -> Option<&'static str> {
        agent.map(|a| a.id)
    }

    #[test]
    fn agents_by_name() {
        assert_eq!(id(agent_named("codex")), Some("codex"));
        assert_eq!(id(agent_named("/home/me/.local/bin/claude")), Some("claude"));
        assert_eq!(id(agent_named(r"C:\Users\me\AppData\Roaming\npm\codex.cmd")), Some("codex"));
        assert_eq!(id(agent_named("Claude.EXE")), Some("claude"));
        assert_eq!(id(agent_named("@openai/codex@latest")), Some("codex"));
        assert_eq!(id(agent_named("@anthropic-ai/claude-code")), Some("claude"));
        assert_eq!(id(agent_named("opencode-ai@latest")), Some("opencode"));
        assert_eq!(id(agent_named("cursor-agent")), Some("cursor"));
        assert_eq!(id(agent_named("omp")), Some("omp"));
        assert_eq!(id(agent_named("agy")), Some("agy"));
        // The Cursor editor's launcher isn't its agent.
        assert_eq!(id(agent_named("cursor")), None);
        assert_eq!(id(agent_named("ls")), None);
        assert_eq!(id(agent_named("")), None);
    }

    #[test]
    fn agents_by_command() {
        let of = |line: &str| id(agent_in_command(line));
        assert_eq!(of("codex"), Some("codex"));
        assert_eq!(of("  claude --resume  "), Some("claude"));
        assert_eq!(of("npx -y @openai/codex"), Some("codex"));
        assert_eq!(of("bunx opencode-ai@latest"), Some("opencode"));
        assert_eq!(of("pnpm dlx @google/gemini-cli"), Some("gemini"));
        assert_eq!(of("ANTHROPIC_MODEL=x sudo -E claude"), Some("claude"));
        assert_eq!(of("& 'C:\\tools\\codex.exe' --full-auto"), Some("codex"));
        assert_eq!(of("node ./node_modules/.bin/pi"), Some("pi"));
        assert_eq!(of("omp"), Some("omp"));
        assert_eq!(of("wsl.exe ~ -d Debian -- codex"), Some("codex"));
        assert_eq!(of("wsl -d Ubuntu -e claude"), Some("claude"));
        assert_eq!(of("wsl.exe ~ -d codex"), None);
        // Not the agent: something else, with it as an argument.
        assert_eq!(of("cat codex.md"), None);
        assert_eq!(of("git commit -m claude"), None);
        assert_eq!(of("vim"), None);
        assert_eq!(of(""), None);
    }

    #[test]
    fn agents_by_process() {
        let of = |argv: &[&str]| id(agent_in_argv(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>()));
        assert_eq!(of(&["/usr/local/bin/codex", "--full-auto"]), Some("codex"));
        assert_eq!(of(&["claude"]), Some("claude"));
        assert_eq!(of(&["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"]), Some("claude"));
        assert_eq!(of(&["node", "/home/me/.npm/_npx/1/node_modules/@openai/codex/bin/codex.js"]), Some("codex"));
        assert_eq!(of(&[r"C:\Program Files\nodejs\node.exe", r"C:\Users\me\AppData\Roaming\npm\node_modules\@google\gemini-cli\dist\index.js"]), Some("gemini"));
        assert_eq!(of(&["bun", "/home/me/.bun/bin/opencode"]), Some("opencode"));
        // A launcher script, run by its interpreter.
        assert_eq!(of(&["/bin/sh", "/home/me/.local/bin/kimi"]), Some("kimi"));
        assert_eq!(of(&["bash", "./build.sh"]), None);
        assert_eq!(of(&["node", "server.js"]), None);
        assert_eq!(of(&["vim", "codex.md"]), None);
        assert_eq!(of(&[]), None);
    }

    fn agent(id: &str) -> &'static Agent {
        AGENTS.iter().find(|a| a.id == id).unwrap()
    }

    #[test]
    fn states_from_screens() {
        let seen = |a: &str, screen: &str, title: &str| detect(agent(a), screen, title, "").seen;
        // Codex says it in its title: a spinner while working, "Action Required" for you.
        assert_eq!(seen("codex", "", "⠋ my-project"), Some(Seen::Working));
        assert_eq!(seen("codex", "", "Action Required"), Some(Seen::Blocked));
        assert_eq!(seen("codex", "", "my-project"), Some(Seen::Idle));
        // OpenCode on screen.
        assert_eq!(seen("opencode", "Thinking...\n esc to interrupt", ""), Some(Seen::Working));
        assert_eq!(seen("opencode", "△ Permission required\nbash: rm -rf build", ""), Some(Seen::Blocked));
        // Nothing of its own on screen: no rule.
        assert_eq!(seen("opencode", "$ ls\nfile", ""), None);
    }

    /// Claude Code asking with AskUserQuestion (v2.1.331's screens, with
    /// and without previews beside the options): waiting for you, though its
    /// title says `✳`.
    #[test]
    fn claude_asking() {
        for screen in [include_str!("../tests/screens/claude-question.txt"), include_str!("../tests/screens/claude-question-preview.txt")] {
            assert_eq!(detect(agent("claude"), screen, "✳ AskUserQuestion color and fruit test", "").seen, Some(Seen::Blocked));
        }
    }

    #[test]
    fn regions() {
        let screen = "one\n\ntwo\nthree\n\n";
        assert_eq!(bottom_non_empty_lines(screen, 2), "two\nthree\n\n");
        assert_eq!(bottom_lines(screen, 1), "\n");
        assert_eq!(top_non_empty_lines(screen, 2), "one\n\ntwo\n");
        assert_eq!(after_last_horizontal_rule("a\n────\nb\n"), "b\n");
        let boxed = "out\n────\n> hi\n────\nfooter";
        assert_eq!(prompt_box_body(boxed), Some("> hi\n"));
        assert_eq!(above_prompt_box(boxed), "out\n");
        let codex = "• did it\n› next\n";
        assert_eq!(after_last_prompt_marker(codex), "");
        assert_eq!(before_current_prompt_marker(codex), "• did it\n");
        assert_eq!(current_prompt_block_marker(codex), Some("• did it"));
    }

    #[test]
    fn working_then_done() {
        let codex = agent("codex");
        let mut t = Tracker::default();
        let now = Instant::now();
        assert!(t.start(codex));
        // Not shown until its screen shows it.
        assert!(!t.update("$ ", "", "", now));
        assert_eq!(t.status(), None);
        assert!(t.update("", "⠋ proj", "", now));
        assert_eq!(t.status().map(|s| (s.state, s.app)), Some((State::Working, Some("codex".into()))));
        // Nothing said for a moment: still working; for longer, done.
        let later = now + Duration::from_millis(100);
        assert!(!t.update("", "", "", later));
        assert_eq!(t.status().unwrap().state, State::Working);
        assert!(t.update("", "", "", later + IDLE_AFTER_WORK));
        assert_eq!(t.status().unwrap().state, State::Done);
        // Its idle title says so at once.
        assert!(t.update("", "⠙ proj", "", later + IDLE_AFTER_WORK));
        assert!(t.update("", "proj", "", later + IDLE_AFTER_WORK));
        assert_eq!(t.status().unwrap().state, State::Done);
        // Done until it works again; waiting for you shows at once.
        assert!(!t.update("", "proj", "", later + IDLE_AFTER_WORK * 2));
        assert!(t.update("", "Action Required", "", later + IDLE_AFTER_WORK * 3));
        assert_eq!(t.status().unwrap().state, State::Blocked);
        assert!(t.stop());
        assert_eq!(t.status(), None);
    }
}
