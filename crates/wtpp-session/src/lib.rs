//! A terminal session: one child process on a PTY, plus the VT state it drives.
//!
//! PTY IO and VT parsing run on background threads (alacritty_terminal's
//! event loop, or ours for a console handed over by Windows; see [`Pipes`]). The UI thread locks [`Session::term`] to read the grid when it
//! renders, and receives [`SessionEvent`]s through the callback it supplied.

pub mod cwd;
pub mod env;
pub mod profiles;
pub mod ssh;

use std::borrow::Cow;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::CursorStyle;
use wtpp_input::EncodeOptions;

pub use alacritty_terminal;
pub use profiles::Profile;

/// Terminal size in cells, plus the cell size in physical pixels (reported to
/// apps that ask for it, e.g. for sixel/image sizing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl GridSize {
    fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.cols,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }

    fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    fn columns(&self) -> usize {
        self.cols as usize
    }
}

/// Events the UI needs to react to. Delivered on the PTY thread.
#[derive(Debug)]
pub enum SessionEvent {
    /// The screen contents changed and should be redrawn.
    Wakeup,
    Title(Option<String>),
    Bell,
    ClipboardStore(String),
    /// The shell ended, with its exit code when known.
    Exited(Option<i32>),
}

type Callback = dyn Fn(SessionEvent) + Send + Sync;

/// A console session handed to us by Windows when wt++ is the default
/// terminal: the console host's pipes instead of a PTY we created.
pub struct Pipes {
    /// What the console host writes (VT output).
    pub output: File,
    /// Where we write input for it.
    pub input: File,
    /// Takes resize messages.
    pub signal: File,
    /// The client program's process id.
    pub pid: Option<u32>,
    /// Waits for the client program to exit and returns its exit code.
    pub wait_exit: Box<dyn FnOnce() -> Option<i32> + Send>,
    /// Handles to hold for the session's lifetime: closing them tells the
    /// console host the terminal is gone.
    pub keep_alive: Vec<File>,
}

/// Where input and resizes go.
#[derive(Clone)]
enum Io {
    Pty(EventLoopSender),
    /// `None` on the input channel (or an empty signal) means the session
    /// is closing: the console host exits when its pipes close.
    Pipes { input: mpsc::Sender<Option<Cow<'static, [u8]>>>, signal: Arc<Mutex<Option<File>>> },
    /// Nothing: the program couldn't be started (see [`Session::failed`]).
    None,
}

impl Io {
    fn input(&self, bytes: Cow<'static, [u8]>) {
        match self {
            Io::Pty(pty) => {
                if let Err(e) = pty.send(Msg::Input(bytes)) {
                    log::warn!("write to pty failed: {e:?}");
                }
            }
            Io::Pipes { input, .. } => {
                let _ = input.send(Some(bytes));
            }
            Io::None => {}
        }
    }

    fn resize(&self, size: GridSize) {
        match self {
            Io::Pty(pty) => {
                let _ = pty.send(Msg::Resize(size.window_size()));
            }
            Io::Pipes { signal, .. } => {
                // ConPTY's resize signal: PTY_SIGNAL_RESIZE_WINDOW (8), then
                // columns and rows, as little-endian u16s.
                let mut packet = [0u8; 6];
                packet[..2].copy_from_slice(&8u16.to_le_bytes());
                packet[2..4].copy_from_slice(&size.cols.to_le_bytes());
                packet[4..].copy_from_slice(&size.rows.to_le_bytes());
                if let Ok(mut signal) = signal.lock()
                    && let Some(signal) = signal.as_mut()
                {
                    let _ = signal.write_all(&packet);
                }
            }
            Io::None => {}
        }
    }
}

/// Bridges alacritty's events to our callback, and answers terminal queries
/// (DSR, color requests, …) that need to be written back to the PTY.
#[derive(Clone)]
pub struct Listener {
    callback: Arc<Callback>,
    pty: Arc<OnceLock<Io>>,
    size: Arc<FairMutex<GridSize>>,
    cwd_scanner: Arc<Mutex<cwd::CwdScanner>>,
    /// The last of the output, to say in the log what rang the bell.
    recent_output: Arc<Mutex<Vec<u8>>>,
    /// The directory the shell last reported (see [`cwd::parse_cwd_osc`]).
    reported_cwd: Arc<Mutex<cwd::Reported>>,
    /// When output last arrived.
    last_output: Arc<Mutex<Option<Instant>>>,
    /// See [`Profile::ssh_token`].
    ssh_token: Option<Arc<str>>,
    /// The arguments of the ssh the shell reported running, until it reports
    /// it's done (see `wtpp_config::ssh`).
    reported_ssh: Arc<Mutex<Option<Vec<String>>>>,
}

impl Listener {
    fn reply(&self, text: String) {
        if let Some(pty) = self.pty.get() {
            pty.input(Cow::Owned(text.into_bytes()));
        }
    }

    fn scan_output(&self, bytes: &[u8]) {
        if let Ok(mut last) = self.last_output.lock() {
            *last = Some(Instant::now());
        }
        const KEPT: usize = 160;
        if let Ok(mut recent) = self.recent_output.lock() {
            recent.extend_from_slice(&bytes[bytes.len().saturating_sub(KEPT)..]);
            let extra = recent.len().saturating_sub(KEPT);
            recent.drain(..extra);
        }
        let found = self.cwd_scanner.lock().map(|mut scanner| scanner.feed_osc(bytes)).unwrap_or_default();
        for osc in found {
            if let Some((kind, dir)) = cwd::parse_cwd_report(&osc) {
                if let Ok(mut reported) = self.reported_cwd.lock() {
                    reported.record(kind, dir);
                }
            } else if let Some(token) = &self.ssh_token
                && let Some(report) = wtpp_config::ssh::parse_report(&osc, token)
                && let Ok(mut ssh) = self.reported_ssh.lock()
            {
                *ssh = match report {
                    wtpp_config::ssh::Report::Started(args) => Some(args),
                    wtpp_config::ssh::Report::Ended => None,
                };
            }
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let cb = &self.callback;
        match event {
            Event::Wakeup => cb(SessionEvent::Wakeup),
            Event::Title(t) => cb(SessionEvent::Title(Some(t))),
            Event::ResetTitle => cb(SessionEvent::Title(None)),
            Event::Bell => {
                // Which program rings, and why, is otherwise anyone's guess.
                let recent = self.recent_output.lock().map(|r| r.escape_ascii().to_string()).unwrap_or_default();
                log::info!("bell; the output up to here ends: {recent}");
                cb(SessionEvent::Bell)
            }
            Event::ClipboardStore(_, text) => cb(SessionEvent::ClipboardStore(text)),
            Event::PtyWrite(text) => self.reply(text),
            Event::TextAreaSizeRequest(format) => {
                let size = *self.size.lock();
                self.reply(format(size.window_size()));
            }
            Event::ChildExit(status) => cb(SessionEvent::Exited(status.code())),
            Event::Exit => cb(SessionEvent::Exited(None)),
            // TODO(config): answer color queries from the active scheme and
            // support clipboard reads once there's a clipboard permission setting.
            Event::ColorRequest(..) | Event::ClipboardLoad(..) => {}
            Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
        }
    }

    fn pty_output(&self, bytes: &[u8]) {
        self.scan_output(bytes);
    }
}

type SharedTerm = Arc<FairMutex<Term<Listener>>>;

pub struct Session {
    term: SharedTerm,
    io: Io,
    size: Arc<FairMutex<GridSize>>,
    profile: Profile,
    /// The process the session started (or was handed): the shell.
    pid: Option<u32>,
    /// See [`Listener::reported_cwd`].
    reported_cwd: Arc<Mutex<cwd::Reported>>,
    /// See [`Listener::last_output`].
    last_output: Arc<Mutex<Option<Instant>>>,
    /// See [`Listener::reported_ssh`].
    reported_ssh: Arc<Mutex<Option<Vec<String>>>>,
    /// For a handed-over console (see [`Pipes::keep_alive`]).
    _keep_alive: Vec<File>,
}

impl Session {
    fn new_term(
        profile: &Profile,
        size: GridSize,
        on_event: impl Fn(SessionEvent) + Send + Sync + 'static,
    ) -> (SharedTerm, Listener, Arc<FairMutex<GridSize>>) {
        let shared_size = Arc::new(FairMutex::new(size));
        let listener = Listener {
            callback: Arc::new(on_event),
            pty: Arc::new(OnceLock::new()),
            size: shared_size.clone(),
            cwd_scanner: Default::default(),
            recent_output: Default::default(),
            reported_cwd: Default::default(),
            last_output: Default::default(),
            ssh_token: profile.ssh_token.as_deref().filter(|t| !t.is_empty()).map(Arc::from),
            reported_ssh: Default::default(),
        };

        let config = Config {
            kitty_keyboard: true,
            scrolling_history: profile.history,
            default_cursor_style: CursorStyle { shape: profile.cursor_shape, blinking: false },
            semantic_escape_chars: profile.word_delimiters.clone(),
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        (term, listener, shared_size)
    }

    pub fn spawn(
        profile: &Profile,
        size: GridSize,
        on_event: impl Fn(SessionEvent) + Send + Sync + 'static,
    ) -> anyhow::Result<Session> {
        let (term, listener, shared_size) = Self::new_term(profile, size, on_event);

        // The profile's own variables over the base, if there is one.
        let mut env: std::collections::HashMap<String, String> = match &profile.base_env {
            Some(base) => {
                let ours = |k: &str| profile.env.keys().any(|o| o.eq_ignore_ascii_case(k));
                base.iter().filter(|(k, _)| !ours(k)).cloned().chain(profile.env.clone()).collect()
            }
            None => profile.env.clone(),
        };
        let has = |env: &std::collections::HashMap<String, String>, k: &str| env.keys().any(|e| e.eq_ignore_ascii_case(k));
        if !has(&env, "TERM") {
            env.insert("TERM".into(), "xterm-256color".into());
        }
        if !has(&env, "COLORTERM") {
            env.insert("COLORTERM".into(), "truecolor".into());
        }
        env.retain(|k, _| !k.eq_ignore_ascii_case("TERM_PROGRAM"));
        env.insert("TERM_PROGRAM".into(), "wtpp".into());
        // CreateProcess looks the program up in wt++'s PATH, which may be out
        // of date: look it up in the new pane's.
        #[cfg(windows)]
        let program = match env.iter().find(|(k, _)| k.eq_ignore_ascii_case("PATH")) {
            Some((_, path)) if profile.base_env.is_some() => {
                profiles::find_program(&profile.program, path).unwrap_or_else(|| profile.program.clone())
            }
            _ => profile.program.clone(),
        };
        #[cfg(not(windows))]
        let program = profile.program.clone();

        let options = tty::Options {
            shell: Some(tty::Shell::new(program, profile.args.clone())),
            working_directory: profile.cwd.clone(),
            drain_on_exit: true,
            #[cfg(windows)]
            inherit_env: profile.base_env.is_none(),
            env,
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, size.window_size(), 0)?;
        #[cfg(windows)]
        let pid = pty.child_watcher().pid().map(|p| p.get());
        #[cfg(not(windows))]
        let pid = Some(pty.child().id());

        let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, true, false)?;
        let io = Io::Pty(event_loop.channel());
        let _ = listener.pty.set(io.clone());
        event_loop.spawn();

        let reported_cwd = listener.reported_cwd.clone();
        let reported_ssh = listener.reported_ssh.clone();
        Ok(Session { term, io, size: shared_size, profile: profile.clone(), pid, reported_cwd, last_output: listener.last_output.clone(), reported_ssh, _keep_alive: Vec::new() })
    }

    /// A session on a console Windows handed over (see [`Pipes`]): a thread
    /// parses its output, another writes input.
    pub fn from_pipes(
        profile: &Profile,
        size: GridSize,
        pipes: Pipes,
        on_event: impl Fn(SessionEvent) + Send + Sync + 'static,
    ) -> anyhow::Result<Session> {
        let (term, listener, shared_size) = Self::new_term(profile, size, on_event);
        let Pipes { mut output, mut input, signal, pid, wait_exit, keep_alive } = pipes;

        let (tx, rx) = mpsc::channel::<Option<Cow<'static, [u8]>>>();
        std::thread::Builder::new().name("handoff-input".into()).spawn(move || {
            // Ends (closing the pipe) on `None`, when the session closes.
            while let Ok(Some(bytes)) = rx.recv() {
                if input.write_all(&bytes).is_err() {
                    break;
                }
            }
        })?;

        let reader_term = term.clone();
        let callback = listener.callback.clone();
        let scanner = listener.clone();
        std::thread::Builder::new().name("handoff-output".into()).spawn(move || {
            let mut parser: alacritty_terminal::vte::ansi::Processor = Default::default();
            let mut buf = vec![0u8; 64 * 1024];
            while let Ok(n @ 1..) = output.read(&mut buf) {
                scanner.scan_output(&buf[..n]);
                parser.advance(&mut *reader_term.lock(), &buf[..n]);
                callback(SessionEvent::Wakeup);
            }
        })?;

        // The program ending is the session ending (the console host may
        // outlive it); its last output gets time to arrive: until it has been
        // quiet for a moment (see the PTY's `drain`).
        let callback = listener.callback.clone();
        let last_output = listener.last_output.clone();
        std::thread::Builder::new().name("handoff-exit".into()).spawn(move || {
            let code = wait_exit();
            let exited = Instant::now();
            let quiet = Duration::from_millis(150);
            loop {
                std::thread::sleep(Duration::from_millis(10));
                let last = last_output.lock().ok().and_then(|l| *l).map_or(exited, |l| l.max(exited));
                if last.elapsed() >= quiet || exited.elapsed() >= Duration::from_secs(2) {
                    break;
                }
            }
            callback(SessionEvent::Exited(code));
        })?;

        let io = Io::Pipes { input: tx, signal: Arc::new(Mutex::new(Some(signal))) };
        let _ = listener.pty.set(io.clone());
        // The console host starts at its own default size.
        io.resize(size);
        let reported_cwd = listener.reported_cwd.clone();
        Ok(Session { term, io, size: shared_size, profile: profile.clone(), pid, reported_cwd, last_output: listener.last_output.clone(), reported_ssh: listener.reported_ssh.clone(), _keep_alive: keep_alive })
    }

    /// A session for a program that couldn't be started: a terminal to say
    /// so in (see [`Session::feed`]), with no process behind it.
    pub fn failed(profile: &Profile, size: GridSize, on_event: impl Fn(SessionEvent) + Send + Sync + 'static) -> Session {
        let (term, listener, shared_size) = Self::new_term(profile, size, on_event);
        let reported_cwd = listener.reported_cwd.clone();
        let last_output = listener.last_output.clone();
        Session { term, io: Io::None, size: shared_size, profile: profile.clone(), pid: None, reported_cwd, last_output, reported_ssh: listener.reported_ssh.clone(), _keep_alive: Vec::new() }
    }

    /// When the program's output last arrived, if it has printed anything.
    pub fn last_output(&self) -> Option<Instant> {
        self.last_output.lock().ok().and_then(|l| *l)
    }

    pub fn term(&self) -> &Arc<FairMutex<Term<Listener>>> {
        &self.term
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// The shell's process id (the program a handed-over console started).
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// The directory the shell last reported, if it does (OSC 7 or 9;9).
    pub fn reported_cwd(&self) -> Option<String> {
        self.reported_cwd.lock().ok()?.last.clone()
    }

    /// Everything the shell reported about its directory.
    pub fn reported_dirs(&self) -> cwd::Reported {
        self.reported_cwd.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// The arguments of the ssh the shell says it's running (with the pane's
    /// token, see `wtpp_config::ssh`), as the user gave them.
    pub fn reported_ssh(&self) -> Option<Vec<String>> {
        self.reported_ssh.lock().ok()?.clone()
    }

    /// The ssh the shell runs in the foreground, found among its processes
    /// (see [`ssh::foreground_ssh`]).
    pub fn foreground_ssh(&self) -> Option<ssh::Found> {
        ssh::foreground_ssh(self.pid?)
    }

    /// The shell process's working directory (see [`cwd::process_cwd`]).
    pub fn process_cwd(&self) -> Option<std::path::PathBuf> {
        cwd::process_cwd(self.pid?)
    }

    pub fn size(&self) -> GridSize {
        *self.size.lock()
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return;
        }
        // Typing snaps the viewport back to the prompt, like every terminal.
        self.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        self.io.input(bytes);
    }

    /// Show text in the terminal as if the shell had printed it (used for
    /// wt++'s own messages, like "[process exited]").
    pub fn feed(&self, bytes: &[u8]) {
        let mut parser: alacritty_terminal::vte::ansi::Processor = Default::default();
        parser.advance(&mut *self.term.lock(), bytes);
    }

    pub fn resize(&self, size: GridSize) {
        {
            let mut current = self.size.lock();
            if *current == size {
                return;
            }
            *current = size;
        }
        self.term.lock().resize(size);
        self.io.resize(size);
    }

    /// Key-encoding options reflecting the profile and the modes the running
    /// app has requested.
    pub fn encode_options(&self) -> EncodeOptions {
        let mode = *self.term.lock().mode();
        EncodeOptions {
            style: self.profile.key_style,
            // Only once the console host has asked for it: until then (or if
            // it never does) it gets VT, which all of them translate to keys.
            win32_input: self.profile.force_win32_input || mode.contains(TermMode::WIN32_INPUT),
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            kitty_disambiguate: mode.contains(TermMode::DISAMBIGUATE_ESC_CODES),
            ..EncodeOptions::default()
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        match &self.io {
            Io::Pty(pty) => {
                let _ = pty.send(Msg::Shutdown);
            }
            // Closing the input and signal pipes makes the console host send
            // the program a close event and exit (like ClosePseudoConsole).
            Io::Pipes { input, signal } => {
                let _ = input.send(None);
                if let Ok(mut signal) = signal.lock() {
                    signal.take();
                }
            }
            Io::None => {}
        }
    }
}

#[cfg(test)]
mod win32_input_tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::vte::ansi::Processor;

    fn term() -> Term<VoidListener> {
        Term::new(Config::default(), &GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 }, VoidListener)
    }

    fn asked_for(chunks: &[&[u8]]) -> bool {
        let mut term = term();
        let mut parser: Processor = Default::default();
        for chunk in chunks {
            parser.advance(&mut term, chunk);
        }
        term.mode().contains(TermMode::WIN32_INPUT)
    }

    /// A console host asks before anything else (see `real_console_host`).
    const STARTUP: &[u8] = b"\x1b[?9001h\x1b[?1004h\x1b[?25l\x1b[2J\x1b[m\x1b[H";

    #[test]
    fn asked_at_the_start_of_the_stream() {
        assert!(!asked_for(&[]));
        assert!(!asked_for(&[b"\x1b[?25h\x1b[?1004h\x1b[2J"]));
        assert!(asked_for(&[STARTUP]));
        assert!(asked_for(&[STARTUP, b"C:\\>"]));
        assert!(asked_for(&[b"\x1b[?1004;9001h"]));
        // Neither does anything before it that isn't text.
        assert!(asked_for(&[b"\x1b[2J\x1b]0;title\x07\r\n\x1b[?9001h"]));
    }

    /// Windows 10 passes on what a program prints, but can't take key events.
    #[test]
    fn programs_cannot_ask() {
        assert!(!asked_for(&[b"C:\\>type file\r\n\x1b[?9001h"]));
        assert!(!asked_for(&[b"x", b"\x1b[?9001h"]));
        assert!(!asked_for(&[b"\x1b[?25l\x1b[2J\x1b[Hx\x1b[?9001h"]));
    }

    /// Nor can they take it away, which would be harmless, but it's one rule.
    #[test]
    fn programs_cannot_turn_it_off() {
        assert!(asked_for(&[STARTUP, b"C:\\>type file\r\n\x1b[?9001l"]));
        assert!(asked_for(&[b"\x1b[?9001h\x1b[?9001l"]));
        assert!(!asked_for(&[b"\x1b[?9001l\x1b[?9001h"]));
    }

    #[test]
    fn split_across_reads() {
        let bytes = b"\x1b[?25l\x1b[?9001hafter";
        for at in 0..=bytes.len() {
            assert!(asked_for(&[&bytes[..at], &bytes[at..]]), "split at {at}");
        }
        let one_by_one: Vec<&[u8]> = bytes.chunks(1).collect();
        assert!(asked_for(&one_by_one));
        let late = b"text\x1b[?9001h";
        for at in 0..=late.len() {
            assert!(!asked_for(&[&late[..at], &late[at..]]), "split at {at}");
        }
        // Nor is it fooled by parts of it, or other modes.
        assert!(!asked_for(&[b"\x1b[?900", b"1"]));
        assert!(!asked_for(&[b"\x1b[9001h", b"\x1b[?90011h"]));
    }

    /// Programs that push keyboard modes and never pop them (or die first)
    /// fill the stack: the oldest go, nothing breaks.
    #[test]
    fn keyboard_mode_stack_overflow() {
        let mut term = Term::new(
            Config { kitty_keyboard: true, ..Config::default() },
            &GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 },
            VoidListener,
        );
        let mut parser: Processor = Default::default();
        for _ in 0..5000 {
            parser.advance(&mut term, b"\x1b[>1u");
        }
        assert!(term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
        parser.advance(&mut term, b"\x1b[<10000u");
        assert!(!term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
    }

    /// It's the console host's request, not the program's state: a reset
    /// neither takes it away nor lets a program ask again.
    #[test]
    fn survives_a_terminal_reset() {
        assert!(asked_for(&[b"\x1b[?9001h\x1bc"]));
        assert!(!asked_for(&[b"text\x1bc\x1b[?9001h"]));
    }

    /// What the app's retry relies on: no process, an error. (On Unix the
    /// shell just starts somewhere else, so there's nothing to retry.)
    #[cfg(windows)]
    #[test]
    fn missing_starting_directory_fails_the_spawn() {
        let size = GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 };
        let mut profile = Profile::new("t", "cmd.exe", &[]);
        profile.cwd = Some(std::path::PathBuf::from(r"C:\wtpp-no-such-folder"));
        assert!(Session::spawn(&profile, size, |_| {}).is_err());
    }

    /// Typing at a prompt rings no bell: the shells as wt++ starts them (the
    /// environment built afresh, the directory reports), keys one by one.
    /// With WTPP_TEST_WSL_DISTRO set, that distro's shell too.
    #[cfg(windows)]
    #[test]
    fn typing_rings_no_bell() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};
        let size = GridSize { cols: 120, rows: 30, cell_width: 8, cell_height: 16 };
        let mut shells = vec![Profile::new("cmd", "cmd.exe", &[]), Profile::new("powershell", "powershell.exe", &["-NoLogo", "-NoProfile"])];
        shells[1].args = wtpp_config::cwd::powershell_args_with_hook("powershell.exe", &shells[1].args).unwrap();
        if let Ok(distro) = std::env::var("WTPP_TEST_WSL_DISTRO") {
            let mut wsl = Profile::new("wsl", "wsl.exe", &["~", "-d", &distro]);
            wsl.env.extend(wtpp_config::cwd::wsl_prompt_env(false, None, None, true, &[]));
            shells.push(wsl);
        }
        for mut profile in shells {
            profile.base_env = env::pane_environment();
            assert!(profile.base_env.is_some());
            profile.cwd = std::env::var("SystemRoot").ok().map(Into::into);
            let bells = Arc::new(AtomicUsize::new(0));
            let counter = bells.clone();
            let session = Session::spawn(&profile, size, move |e| {
                if matches!(e, SessionEvent::Bell) {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })
            .unwrap();
            let shown = |text: &str| {
                let term = session.term().lock();
                let grid = term.grid();
                (0..30).any(|l| {
                    let line: String =
                        (0..120).map(|c| grid[alacritty_terminal::index::Line(l)][alacritty_terminal::index::Column(c)].c).collect();
                    line.trim_end() == text
                })
            };
            // A prompt first: the directory report is how we know there's one.
            let start = Instant::now();
            let ready = |s: &Session| s.reported_cwd().is_some() || (profile.name == "cmd" && start.elapsed() > Duration::from_secs(3));
            while !ready(&session) && start.elapsed() < Duration::from_secs(30) {
                std::thread::sleep(Duration::from_millis(100));
            }
            for key in "echo wtpp-typed\r".chars() {
                session.write(key.to_string().into_bytes());
                std::thread::sleep(Duration::from_millis(60));
            }
            let start = Instant::now();
            while !shown("wtpp-typed") && start.elapsed() < Duration::from_secs(20) {
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(shown("wtpp-typed"), "{}: the keys didn't arrive", profile.name);
            assert_eq!(bells.load(Ordering::SeqCst), 0, "{}", profile.name);
        }
    }

    /// The bell itself still rings, once per BEL outside a title or other OSC.
    #[test]
    fn bells_are_counted_outside_osc_only() {
        use alacritty_terminal::event::{Event, EventListener};
        use std::sync::atomic::{AtomicUsize, Ordering};
        #[derive(Clone)]
        struct Count(Arc<AtomicUsize>);
        impl EventListener for Count {
            fn send_event(&self, event: Event) {
                if matches!(event, Event::Bell) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        let bells = Arc::new(AtomicUsize::new(0));
        let size = GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 };
        let mut term = Term::new(Config::default(), &size, Count(bells.clone()));
        let mut parser: Processor = Default::default();
        // What the shells wt++ sets up send at every prompt, in any pieces.
        let prompt = b"\x1b]9;9;/home/me\x1b\\\x1b]0;me@pc: ~\x07\x1b]9;9;C:\\Users\\me\x1b\\PS C:\\Users\\me> \x1b]9;4;3;\x07\x1b]7;file://pc/home/me\x07";
        for at in 0..=prompt.len() {
            parser.advance(&mut term, &prompt[..at]);
            parser.advance(&mut term, &prompt[at..]);
        }
        assert_eq!(bells.load(Ordering::SeqCst), 0);
        parser.advance(&mut term, b"a\x07b\x07");
        assert_eq!(bells.load(Ordering::SeqCst), 2);
    }

    /// The real console host of the machine running the tests: on Windows 11
    /// it asks (and everything that worked there depends on it), and the bytes
    /// for when it doesn't do what they should.
    #[cfg(windows)]
    #[test]
    fn real_console_host() {
        use std::time::{Duration, Instant};
        let size = GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 };
        let session = Session::spawn(&Profile::new("cmd", "cmd.exe", &["/q", "/d"]), size, |_| {}).unwrap();
        let screen = |session: &Session| {
            let term = session.term().lock();
            let grid = term.grid();
            let lines = (0..24).map(|l| {
                (0..80)
                    .map(|c| grid[alacritty_terminal::index::Line(l)][alacritty_terminal::index::Column(c)].c)
                    .collect::<String>()
            });
            lines.collect::<Vec<_>>().join("\n")
        };
        let wait_for = |what: &dyn Fn() -> bool| {
            let start = Instant::now();
            while !what() && start.elapsed() < Duration::from_secs(15) {
                std::thread::sleep(Duration::from_millis(50));
            }
            what()
        };
        let asked = wait_for(&|| session.encode_options().win32_input);
        println!("console host asked for win32-input-mode: {asked}");
        if profiles::windows_build().is_some_and(|b| b >= 26100) {
            assert!(asked);
        }

        // Backspace, Left, Delete, Home, End, Enter as VT.
        session.write(&b"echo wtpp-oneX\x7f three\x1b[D\x1b[3~e\x1b[H\x1b[Fz\r"[..]);
        let shown = wait_for(&|| screen(&session).lines().any(|l| l.trim_end() == "wtpp-one threez"));
        assert!(shown, "{}", screen(&session));

        // A program printing the sequences changes nothing, either way.
        for mode in ["l", "h"] {
            session.write(format!("prompt $E[?9001{mode}$G\r").into_bytes());
            session.write(format!("echo wtpp-printed-{mode}\r").into_bytes());
            let line = format!("wtpp-printed-{mode}");
            assert!(wait_for(&|| screen(&session).lines().any(|l| l.trim_end() == line)), "{}", screen(&session));
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(session.encode_options().win32_input, asked);
        }

        // The same keys as they're really sent here, key events or not.
        let key = |key| wtpp_input::encode(key, wtpp_input::Mods::NONE, &session.encode_options()).unwrap();
        session.write(&b"echo wtpp-encodedX"[..]);
        session.write(key(wtpp_input::Key::Backspace));
        session.write(key(wtpp_input::Key::Enter));
        let shown = wait_for(&|| screen(&session).lines().any(|l| l.trim_end() == "wtpp-encoded"));
        assert!(shown, "{}", screen(&session));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::fd::OwnedFd;

    fn pipe() -> (File, File) {
        let (r, w) = std::io::pipe().unwrap();
        (File::from(OwnedFd::from(r)), File::from(OwnedFd::from(w)))
    }

    #[test]
    fn handed_over_console() {
        let (output, mut host_output) = pipe();
        let (mut host_input, input) = pipe();
        let (mut host_signal, signal) = pipe();
        let size = GridSize { cols: 80, rows: 24, cell_width: 8, cell_height: 16 };
        let (tx, rx) = mpsc::channel();
        let pipes = Pipes { output, input, signal, pid: None, wait_exit: Box::new(|| Some(3)), keep_alive: Vec::new() };
        let session = Session::from_pipes(&Profile::new("cmd", "cmd.exe", &[]), size, pipes, move |e| {
            if let SessionEvent::Exited(code) = e {
                tx.send(code).unwrap();
            }
        })
        .unwrap();

        // The initial size goes out as a resize signal.
        let mut packet = [0u8; 6];
        host_signal.read_exact(&mut packet).unwrap();
        assert_eq!(packet, [8, 0, 80, 0, 24, 0]);

        session.write(&b"dir\r"[..]);
        let mut typed = [0u8; 4];
        host_input.read_exact(&mut typed).unwrap();
        assert_eq!(&typed, b"dir\r");

        host_output.write_all(b"hello").unwrap();
        drop(host_output);
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), Some(3));
        let term = session.term().lock();
        let line: String = (0..5).map(|c| term.grid()[alacritty_terminal::index::Line(0)][alacritty_terminal::index::Column(c)].c).collect();
        assert_eq!(line, "hello");
    }
}

/// A pane opened from another shows the directory by the same name, symbolic
/// links and all: real shells, through the real spawn path.
#[cfg(test)]
mod shown_name_tests {
    use super::*;
    use std::time::{Duration, Instant};

    const SIZE: GridSize = GridSize { cols: 160, rows: 30, cell_width: 8, cell_height: 16 };

    fn lines(session: &Session) -> Vec<String> {
        let term = session.term().lock();
        let grid = term.grid();
        (0..SIZE.rows as i32)
            .map(|l| {
                (0..SIZE.cols as usize)
                    .map(|c| grid[alacritty_terminal::index::Line(l)][alacritty_terminal::index::Column(c)].c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn wait(what: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while !what() && start.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(100));
        }
        what()
    }

    fn type_line(session: &Session, line: &str) {
        session.write(format!("{line}\r").into_bytes());
    }

    /// A program that prints and exits at once: all it printed is on the
    /// screen by the time the session says it exited (that's when wt++
    /// writes "[process exited]" under it), and stays there.
    #[test]
    fn output_before_exit_is_kept() {
        let text = "wtpp-last-words: the quick brown fox jumps over the lazy dog";
        #[cfg(windows)]
        let profiles = vec![
            Profile::new("cmd", "cmd.exe", &["/d", "/c", &format!("echo {text}& exit 3")]),
            Profile::new("cmd", "cmd.exe", &["/d", "/c", &format!("echo {text}& echo {text}2& exit 3")]),
        ];
        #[cfg(unix)]
        let profiles = vec![
            Profile::new("sh", "sh", &["-c", &format!("echo '{text}'; exit 3")]),
            Profile::new("sh", "sh", &["-c", &format!("echo '{text}'; echo '{text}2'; exit 3")]),
        ];
        for (n, mut profile) in profiles.into_iter().enumerate() {
            profile.cwd = Some(if cfg!(windows) { std::env::var("SystemRoot").unwrap().into() } else { "/".into() });
            for round in 0..5 {
                let (tx, rx) = mpsc::channel();
                let tx = Mutex::new(tx);
                let session = Session::spawn(&profile, SIZE, move |e| {
                    if let SessionEvent::Exited(code) = e {
                        let _ = tx.lock().unwrap().send(code);
                    }
                })
                .unwrap();
                let code = rx.recv_timeout(Duration::from_secs(20)).expect("exit");
                assert_eq!(code, Some(3));
                let at_exit = lines(&session);
                let shown = |lines: &[String]| lines.iter().filter(|l| l.starts_with(text)).count();
                assert_eq!(shown(&at_exit), n + 1, "round {round}: {at_exit:?}");
                std::thread::sleep(Duration::from_millis(300));
                assert_eq!(shown(&lines(&session)), n + 1, "round {round}");
            }
        }
    }

    /// A directory and a symbolic link to it, with awkward names.
    #[cfg(unix)]
    fn linked_dirs() -> (std::path::PathBuf, String, String) {
        let base = std::env::temp_dir().join(format!("wtpp-shown-{}-{}", std::process::id(), std::thread::current().name().unwrap_or("t").len()));
        let real = base.join("real dir");
        std::fs::create_dir_all(&real).unwrap();
        let link = base.join("link %41 ü");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let real = real.canonicalize().unwrap().to_string_lossy().into_owned();
        (base, real, link.to_string_lossy().into_owned())
    }

    /// The parent reports `link` (its process says `real`); a pane started
    /// as wt++ starts it from there shows `link` too.
    #[cfg(unix)]
    fn check(parent: Profile, child: impl Fn(&str) -> Profile, link: &str, real: &str) {
        let session = Session::spawn(&parent, SIZE, |_| {}).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        type_line(&session, &format!("cd '{link}'"));
        assert!(wait(|| session.reported_dirs().last.as_deref() == Some(link)), "{:?} {:?}", session.reported_dirs(), lines(&session));
        let process = session.process_cwd().unwrap();
        assert_eq!(process.to_string_lossy(), real);
        let shown = cwd::same_directory_name(&process, session.reported_dirs().last.as_deref());
        assert_eq!(shown.as_deref(), Some(link));
        let mut profile = child(link);
        profile.cwd = Some(link.into());
        profile.env.insert("PWD".into(), link.into());
        let new = Session::spawn(&profile, SIZE, |_| {}).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        type_line(&new, "pwd");
        assert!(wait(|| lines(&new).iter().any(|l| l == link)), "{:?}", lines(&new));
    }

    #[cfg(unix)]
    #[test]
    fn bash() {
        let (base, real, link) = linked_dirs();
        let bash = |_: &str| {
            let mut p = Profile::new("bash", "bash", &["--norc", "--noprofile", "-i"]);
            p.env.insert("PROMPT_COMMAND".into(), wtpp_config::cwd::UNIX_BASH_PROMPT_COMMAND.into());
            p.cwd = Some("/".into());
            p
        };
        check(bash(""), bash, &link, &real);
        std::fs::remove_dir_all(&base).ok();
    }

    /// zsh with wt++'s startup files, and the user's own still run. Set
    /// WTPP_TEST_ZSH to a zsh to run it.
    #[cfg(unix)]
    #[test]
    fn zsh() {
        let Ok(zsh) = std::env::var("WTPP_TEST_ZSH") else { return };
        let (base, real, link) = linked_dirs();
        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".zshenv"), "WTPP_TEST_ENV=yes\n").unwrap();
        std::fs::write(home.join(".zshrc"), "PS1='%~ %# '\nWTPP_TEST_RC=yes\nprecmd() { WTPP_TEST_PRECMD=yes }\n").unwrap();
        std::fs::write(home.join(".zlogin"), "WTPP_TEST_LOGIN=yes\n").unwrap();
        let integration = base.join("integration");
        wtpp_config::cwd::write_zsh_integration(&integration).unwrap();
        let home_text = home.to_string_lossy().into_owned();
        for args in [&["-i"][..], &["-l", "-i"]] {
            let make = |_: &str| {
                let mut p = Profile::new("zsh", &zsh, args);
                p.env.insert("HOME".into(), home_text.clone());
                p.env.remove("ZDOTDIR");
                let var = |n: &str| (n == "HOME").then(|| home_text.clone());
                p.env.extend(wtpp_config::cwd::unix_prompt_env("zsh", Some(&integration.to_string_lossy()), var, true, &[]));
                p.cwd = Some("/".into());
                p
            };
            check(make(""), make, &link, &real);
            // The user's files ran, their precmd too, and nothing of wt++'s is left.
            let session = Session::spawn(&make(""), SIZE, |_| {}).unwrap();
            std::thread::sleep(Duration::from_millis(500));
            type_line(&session, "echo \"r=$WTPP_TEST_RC e=$WTPP_TEST_ENV l=${WTPP_TEST_LOGIN-n} p=$WTPP_TEST_PRECMD z=${ZDOTDIR-unset} u=${WTPP_USER_ZDOTDIR-unset}\"");
            let login = if args.contains(&"-l") { "yes" } else { "n" };
            let want = format!("r=yes e=yes l={login} p=yes z=unset u=unset");
            assert!(wait(|| lines(&session).contains(&want)), "{args:?} {:?}", lines(&session));
        }
        std::fs::remove_dir_all(&base).ok();
    }

    /// bash in WSL: started where Windows can (`/usr/bin`, through \\wsl$),
    /// the new pane shows `/bin` from its first prompt. Set
    /// WTPP_TEST_WSL_DISTRO to a distro to run it.
    #[cfg(windows)]
    #[test]
    fn wsl_bash() {
        let Ok(distro) = std::env::var("WTPP_TEST_WSL_DISTRO") else { return };
        let commandline = format!("wsl.exe ~ -d {distro}");
        let mut parent = Profile::new("wsl", "wsl.exe", &["~", "-d", &distro]);
        parent.env.extend(wtpp_config::cwd::wsl_prompt_env(false, None, None, true, &[]));
        let session = Session::spawn(&parent, SIZE, |_| {}).unwrap();
        assert!(wait(|| session.reported_cwd().is_some()));
        type_line(&session, "cd /bin");
        assert!(wait(|| session.reported_dirs().logical.as_deref() == Some("/bin")), "{:?}", session.reported_dirs());
        let reported = session.reported_dirs();
        assert_eq!(reported.osc99.as_deref(), Some("/usr/bin"));
        let dir = wtpp_config::cwd::starting_directory_from("/usr/bin", &commandline, &commandline).unwrap();
        assert_eq!(dir, format!(r"\\wsl$\{distro}\usr\bin"));
        let started = wtpp_config::settings::commandline_for_start(&commandline, Some(&dir));
        let (program, args) = wtpp_config::settings::split_commandline(&started);
        let mut child = Profile::new("wsl", &program, &[]);
        child.args = args;
        child.cwd = Some(dir.into());
        child.env.extend(wtpp_config::cwd::wsl_prompt_env(false, None, reported.logical.as_deref(), true, &[]));
        let new = Session::spawn(&child, SIZE, |_| {}).unwrap();
        assert!(wait(|| new.reported_cwd().is_some()));
        type_line(&new, "pwd");
        assert!(wait(|| lines(&new).iter().any(|l| l == "/bin")), "{:?}", lines(&new));
        assert_eq!(new.reported_dirs().logical.as_deref(), Some("/bin"));
    }
}

/// A pane in ssh: its shell reports it (and nothing else can), a new pane
/// runs the same ssh, sharing the connection. Real shells through the real
/// spawn path, real ssh (to a test address that never answers, so nothing
/// logs in anywhere; see `real_sharing` for a real server).
#[cfg(test)]
mod ssh_tests {
    use super::*;
    use std::time::{Duration, Instant};

    const SIZE: GridSize = GridSize { cols: 200, rows: 40, cell_width: 8, cell_height: 16 };

    fn screen(session: &Session) -> Vec<String> {
        let term = session.term().lock();
        let grid = term.grid();
        (0..SIZE.rows as i32)
            .map(|l| {
                (0..SIZE.cols as usize)
                    .map(|c| grid[alacritty_terminal::index::Line(l)][alacritty_terminal::index::Column(c)].c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn wait(secs: u64, what: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while !what() && start.elapsed() < Duration::from_secs(secs) {
            std::thread::sleep(Duration::from_millis(100));
        }
        what()
    }

    fn type_line(session: &Session, line: &str) {
        session.write(format!("{line}\r").into_bytes());
    }

    fn shows(session: &Session, text: &str) -> bool {
        screen(session).iter().any(|l| l == text)
    }

    /// A place for the test's files, short enough for socket paths.
    #[cfg(unix)]
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(format!("/tmp/wtpp-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A shell as wt++ starts it, with the ssh function, and `ssh` to run.
    #[cfg(unix)]
    fn shell(program: &str, args: &[&str], dir: &std::path::Path, zsh_dir: Option<&str>, ssh: Option<&[String]>) -> Profile {
        let mut p = Profile::new("t", program, args);
        let token = wtpp_config::ssh::new_token();
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        // A zsh unpacked somewhere (see `zsh`) finds its modules by this.
        if let Ok(modules) = std::env::var("WTPP_TEST_ZSH_MODULES") {
            std::fs::write(home.join(".zshenv"), format!("module_path=({modules})\n")).unwrap();
        }
        p.env.insert("HOME".into(), home.to_string_lossy().into_owned());
        p.env.insert("XDG_RUNTIME_DIR".into(), dir.join("run").to_string_lossy().into_owned());
        p.env.remove("ZDOTDIR");
        let home_text = home.to_string_lossy().into_owned();
        let var = |n: &str| (n == "HOME").then(|| home_text.clone());
        let file = program.rsplit('/').next().unwrap();
        p.env.extend(wtpp_config::cwd::unix_prompt_env(file, zsh_dir, var, true, &wtpp_config::ssh::shell_env(&token, ssh.map(|a| (a, wtpp_config::ssh::Run::Function)))));
        p.ssh_token = Some(token);
        p.cwd = Some(dir.to_path_buf());
        p
    }

    #[cfg(unix)]
    fn check_shell(program: &str, shell_args: &[&str], zsh_dir: Option<&str>) {
        let dir = scratch(&format!("ssht-{}", program.rsplit('/').next().unwrap()));
        let config = dir.join("config");
        std::fs::write(&config, "Host *\n  ConnectTimeout 30\n  BatchMode yes\n  UserKnownHostsFile /dev/null\n").unwrap();
        let theirs = dir.join("theirs");
        std::fs::write(&theirs, format!("Host *\n  ControlMaster auto\n  ControlPath {}/their-%C\n", dir.display())).unwrap();
        let parent = Session::spawn(&shell(program, shell_args, &dir, zsh_dir, None), SIZE, |_| {}).unwrap();
        assert!(wait(20, || parent.reported_cwd().is_some()), "{:?}", screen(&parent));

        // The token is the shell's alone.
        type_line(&parent, "echo \"t=${WTPP_TOKEN-unset} n=$(env | grep -c ^WTPP_TOKEN=) f=$(type ssh | head -1)\"");
        assert!(wait(10, || screen(&parent).iter().any(|l| l.starts_with("t=unset n=0 f=ssh is"))), "{:?}", screen(&parent));

        // A forged report, with no token or a wrong one: nothing.
        type_line(&parent, "printf '\\033]6973;ssh;%s;-oProxyCommand=x;h\\033\\\\' \"${WTPP_TOKEN-}\" 0123456789abcdef0123456789abcdef; echo forged");
        assert!(wait(10, || shows(&parent, "forged")));
        assert_eq!(parent.reported_ssh(), None);

        // ssh, with a forward and a command that aren't carried over.
        let typed = ["-F", &config.to_string_lossy(), "-p", "2222", "-L", "8022:localhost:22", "me@192.0.2.1", "uptime"].map(String::from);
        type_line(&parent, &format!("ssh {}", typed.join(" ")));
        assert!(wait(10, || parent.reported_ssh().is_some()), "{:?}", screen(&parent));
        assert_eq!(parent.reported_ssh().unwrap(), typed);
        let socket_dir = dir.join("run/wtpp-ssh");
        let control_path = format!("ControlPath={}/%C", socket_dir.display());
        // Found among the shell's processes too: with the connection shared.
        let found = parent.foreground_ssh().unwrap();
        assert!(found.program.ends_with("ssh"), "{found:?}");
        let mut shared = ["-o", "ControlMaster=auto", "-o", &control_path, "-o", "ControlPersist=10m"].map(String::from).to_vec();
        shared.extend(typed.iter().cloned());
        assert_eq!(found.args, shared);
        assert_eq!(std::fs::metadata(&socket_dir).unwrap().permissions().mode() & 0o777, 0o700);

        // The new pane runs it, through its own function.
        let args = wtpp_config::ssh::session_args(&parent.reported_ssh().unwrap()).unwrap();
        assert_eq!(args, ["-F", &config.to_string_lossy(), "-p", "2222", "me@192.0.2.1"]);
        let child = Session::spawn(&shell(program, shell_args, &dir, zsh_dir, Some(&args)), SIZE, |_| {}).unwrap();
        assert!(wait(20, || child.reported_ssh().is_some()), "{:?}", screen(&child));
        assert_eq!(child.reported_ssh().unwrap(), args);
        let found = child.foreground_ssh().unwrap();
        assert!(found.args.contains(&control_path), "{found:?}");
        assert!(child.reported_cwd().is_some());

        // Done (Ctrl+C): it says so, and it's in the history.
        parent.write(&b"\x03"[..]);
        assert!(wait(10, || parent.reported_ssh().is_none()));
        child.write(&b"\x03"[..]);
        assert!(wait(10, || child.reported_ssh().is_none()));
        assert!(wait(5, || child.foreground_ssh().is_none()));
        child.write(&b"\x1b[A"[..]);
        let recalled = format!("ssh {}", args.join(" "));
        assert!(wait(10, || screen(&child).iter().any(|l| l.ends_with(&recalled))), "{:?}", screen(&child));
        child.write(&b"\x15"[..]);

        // The user's own ControlMaster wins, and -V isn't a session.
        type_line(&parent, &format!("ssh -F {} me@192.0.2.1", theirs.display()));
        assert!(wait(10, || parent.reported_ssh().is_some()));
        let found = parent.foreground_ssh().unwrap();
        assert_eq!(found.args, ["-F", &theirs.to_string_lossy(), "me@192.0.2.1"]);
        parent.write(&b"\x03"[..]);
        assert!(wait(10, || parent.reported_ssh().is_none()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// The user's own `alias ssh='ssh -o …'` (so there's no function, and the
    /// split found the ssh among the pane's processes, the alias's options
    /// included): the new pane runs it past the alias, which would add them
    /// again. Given by the function's report instead, it goes through `ssh`
    /// as the shell knows it. A stand-in ssh on PATH prints what it gets.
    #[cfg(unix)]
    fn check_alias(program: &str, shell_args: &[&str], zsh_dir: Option<&str>) {
        let dir = scratch(&format!("ssha-{}", program.rsplit('/').next().unwrap()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("ssh"), "#!/bin/sh\necho \"ARGV[$*]\"\n").unwrap();
        std::fs::set_permissions(bin.join("ssh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let alias = "alias ssh='ssh -o ServerAliveInterval=30'\n";
        let found = ["-o", "ServerAliveInterval=30", "-p", "22", "me@host"].map(String::from);
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let run = |args: &[String], how: wtpp_config::ssh::Run| {
            let mut p = shell(program, shell_args, &dir, zsh_dir, None);
            let home = std::path::PathBuf::from(&p.env["HOME"]);
            std::fs::write(home.join(".bashrc"), alias).unwrap();
            std::fs::write(home.join(".zshrc"), alias).unwrap();
            let token = p.ssh_token.clone().unwrap();
            p.env.retain(|k, _| k != "WTPP_SSH" && k != "WTPP_SSH_DIRECT");
            p.env.extend(wtpp_config::ssh::shell_env(&token, Some((args, how))).into_iter().skip(1));
            p.env.insert("PATH".into(), path.clone());
            let session = Session::spawn(&p, SIZE, |_| {}).unwrap();
            assert!(wait(20, || screen(&session).iter().any(|l| l.starts_with("ARGV["))), "{:?}", screen(&session));
            std::thread::sleep(Duration::from_millis(300));
            let lines: Vec<String> = screen(&session).into_iter().filter(|l| l.starts_with("ARGV[")).collect();
            (session, lines)
        };
        // Found running: as it was, once.
        let (session, lines) = run(&found, wtpp_config::ssh::Run::Command);
        assert_eq!(lines, ["ARGV[-o ServerAliveInterval=30 -p 22 me@host]"], "{program}");
        // In the history the same way, and the variable is gone.
        session.write(&b"\x1b[A"[..]);
        assert!(wait(10, || screen(&session).iter().any(|l| l.ends_with("command ssh -o ServerAliveInterval=30 -p 22 me@host"))), "{:?}", screen(&session));
        session.write(&b"\x15"[..]);
        type_line(&session, "echo \"d=${WTPP_SSH_DIRECT-unset}\"");
        assert!(wait(10, || shows(&session, "d=unset")), "{:?}", screen(&session));
        // Reported by the function: through `ssh` as the shell has it (here
        // the alias, which then adds its option, as it would when typed).
        let (_, lines) = run(&found[2..], wtpp_config::ssh::Run::Function);
        assert_eq!(lines, ["ARGV[-o ServerAliveInterval=30 -p 22 me@host]"], "{program}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn bash_alias() {
        check_alias("bash", &["--noprofile", "-i"], None);
    }

    /// Set WTPP_TEST_ZSH to a zsh to run it.
    #[cfg(unix)]
    #[test]
    fn zsh_alias() {
        let Ok(zsh) = std::env::var("WTPP_TEST_ZSH") else { return };
        let dir = scratch("sshz-alias-files");
        wtpp_config::cwd::write_zsh_integration(&dir).unwrap();
        check_alias(&zsh, &["-i"], Some(&dir.to_string_lossy()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn bash() {
        if std::process::Command::new("ssh").arg("-V").output().is_err() {
            return;
        }
        check_shell("bash", &["--norc", "--noprofile", "-i"], None);
    }

    /// Set WTPP_TEST_ZSH to a zsh to run it (and WTPP_TEST_ZSH_MODULES to
    /// its modules' directory, if it's not where it was built for).
    #[cfg(unix)]
    #[test]
    fn zsh() {
        let Ok(zsh) = std::env::var("WTPP_TEST_ZSH") else { return };
        let dir = scratch("sshz-files");
        wtpp_config::cwd::write_zsh_integration(&dir).unwrap();
        check_shell(&zsh, &["-i"], Some(&dir.to_string_lossy()));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A real server (an sshd of the test's own, on a high port, with keys
    /// made for it): the new pane gets in without logging in again, the
    /// first one can go, and the connection stays a while after the last.
    /// Set WTPP_TEST_SSHD to an sshd to run it.
    #[cfg(unix)]
    #[test]
    fn real_sharing() {
        let Ok(sshd) = std::env::var("WTPP_TEST_SSHD") else { return };
        use std::process::Command;
        let dir = scratch("sshd");
        let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
        for key in ["host", "id"] {
            let made = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f", &path(key)]).status().unwrap();
            assert!(made.success());
        }
        std::fs::copy(path("id.pub"), path("authorized_keys")).unwrap();
        let port = 20000 + std::process::id() % 20000;
        let user = std::env::var("USER").unwrap();
        std::fs::write(
            path("sshd_config"),
            format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\nPidFile none\nUsePAM no\nStrictModes no\n\
                 PasswordAuthentication no\nKbdInteractiveAuthentication no\nLogLevel VERBOSE\n",
                path("host"),
                path("authorized_keys")
            ),
        )
        .unwrap();
        std::fs::write(
            path("config"),
            format!(
                "Host wtpptest\n  HostName 127.0.0.1\n  Port {port}\n  User {user}\n  IdentityFile {}\n  IdentitiesOnly yes\n  IdentityAgent none\n\
                 UserKnownHostsFile {}\n  GlobalKnownHostsFile /dev/null\n  StrictHostKeyChecking accept-new\n  BatchMode yes\n",
                path("id"),
                path("known_hosts")
            ),
        )
        .unwrap();
        let mut server = Command::new(&sshd).args(["-D", "-f", &path("sshd_config"), "-E", &path("sshd.log")]).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let logins = || std::fs::read_to_string(path("sshd.log")).unwrap_or_default().matches("Accepted publickey").count();

        let first = Session::spawn(&shell("bash", &["--norc", "--noprofile", "-i"], &dir, None, None), SIZE, |_| {}).unwrap();
        assert!(wait(20, || first.reported_cwd().is_some()));
        type_line(&first, &format!("ssh -F {} wtpptest", path("config")));
        assert!(wait(10, || first.reported_ssh().is_some()));
        type_line(&first, "echo remote-$((40+2))");
        assert!(wait(20, || shows(&first, "remote-42")), "{:?} {}", screen(&first), std::fs::read_to_string(path("sshd.log")).unwrap_or_default());
        assert_eq!(logins(), 1);

        let args = wtpp_config::ssh::session_args(&first.reported_ssh().unwrap()).unwrap();
        let second = Session::spawn(&shell("bash", &["--norc", "--noprofile", "-i"], &dir, None, Some(&args)), SIZE, |_| {}).unwrap();
        assert!(wait(20, || second.reported_ssh().is_some()));
        type_line(&second, "echo second-$((1+1))");
        assert!(wait(20, || shows(&second, "second-2")), "{:?}", screen(&second));
        // Over the first one's connection: no new login.
        assert_eq!(logins(), 1);
        let control = format!("ControlPath={}/run/wtpp-ssh/%C", dir.display());
        let check = |command: &str| {
            let out = Command::new("ssh").args(["-F", &path("config"), "-o", &control, "-O", command, "wtpptest"]).output().unwrap();
            String::from_utf8_lossy(&out.stderr).into_owned()
        };
        assert!(check("check").starts_with("Master running"), "{}", check("check"));

        // The first pane closes: the second stays connected.
        drop(first);
        std::thread::sleep(Duration::from_millis(500));
        type_line(&second, "echo still-$((2+1))");
        assert!(wait(20, || shows(&second, "still-3")), "{:?}", screen(&second));
        // Its ssh ends as ever, and the connection stays for the next one.
        type_line(&second, "exit");
        assert!(wait(20, || second.reported_ssh().is_none()));
        std::thread::sleep(Duration::from_millis(300));
        println!("after exit: {:?}", screen(&second).iter().filter(|l| !l.is_empty()).collect::<Vec<_>>());
        assert!(check("check").starts_with("Master running"));
        let third = Session::spawn(&shell("bash", &["--norc", "--noprofile", "-i"], &dir, None, Some(&args)), SIZE, |_| {}).unwrap();
        type_line(&third, "echo third-$((3+1))");
        assert!(wait(20, || shows(&third, "third-4")), "{:?}", screen(&third));
        assert_eq!(logins(), 1);
        drop(third);
        drop(second);
        check("exit");
        server.kill().ok();
        server.wait().ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// bash in WSL: Windows doesn't see its ssh, the report says. The new
    /// pane (the same distro) runs it. Set WTPP_TEST_WSL_DISTRO to run it.
    #[cfg(windows)]
    #[test]
    fn wsl() {
        let Ok(distro) = std::env::var("WTPP_TEST_WSL_DISTRO") else { return };
        let pane = |ssh: Option<&[String]>| {
            let mut p = Profile::new("wsl", "wsl.exe", &["~", "-d", &distro]);
            let token = wtpp_config::ssh::new_token();
            p.env.extend(wtpp_config::cwd::wsl_prompt_env(false, None, None, true, &wtpp_config::ssh::shell_env(&token, ssh.map(|a| (a, wtpp_config::ssh::Run::Function)))));
            p.ssh_token = Some(token);
            Session::spawn(&p, SIZE, |_| {}).unwrap()
        };
        let parent = pane(None);
        assert!(wait(30, || parent.reported_cwd().is_some()), "{:?}", screen(&parent));
        type_line(&parent, "echo \"t=${WTPP_TOKEN-unset} f=$(type -t ssh)\"");
        assert!(wait(10, || shows(&parent, "t=unset f=function")), "{:?}", screen(&parent));
        let typed = ["-F", "/dev/null", "-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-p", "2222", "-L", "1:x:1", "me@192.0.2.1", "uptime"].map(String::from);
        type_line(&parent, &format!("ssh {}", typed.join(" ")));
        assert!(wait(10, || parent.reported_ssh().is_some()), "{:?}", screen(&parent));
        assert_eq!(parent.reported_ssh().unwrap(), typed);
        assert_eq!(parent.foreground_ssh(), None);
        let args = wtpp_config::ssh::session_args(&typed).unwrap();
        assert_eq!(args, ["-F", "/dev/null", "-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-p", "2222", "me@192.0.2.1"]);
        let child = pane(Some(&args));
        assert!(wait(30, || child.reported_ssh().is_some()), "{:?}", screen(&child));
        assert_eq!(child.reported_ssh().unwrap(), args);
        assert!(child.reported_cwd().is_some());
        for session in [&parent, &child] {
            session.write(&b"\x03"[..]);
            assert!(wait(10, || session.reported_ssh().is_none()));
        }
        // Back at the shell, where it was.
        type_line(&child, "echo back-$((1+1)) ${WTPP_SSH-gone}");
        assert!(wait(10, || shows(&child, "back-2 gone")), "{:?}", screen(&child));
    }

    /// Windows' own ssh.exe, which shares nothing: found among the shell's
    /// processes, and run again by PowerShell and cmd.
    #[cfg(windows)]
    #[test]
    fn windows_ssh() {
        let Some(ssh) = std::env::var("SystemRoot").ok().map(|r| format!(r"{r}\System32\OpenSSH\ssh.exe")).filter(|p| std::path::Path::new(p).exists()) else {
            return;
        };
        let typed = ["-F", "NUL", "-o", "BatchMode=yes", "-o", "ConnectTimeout=20", "-p", "2222", "me@192.0.2.1"].map(String::from);
        let ps = |run: Option<&[String]>| {
            let mut p = Profile::new("ps", "powershell.exe", &["-NoLogo", "-NoProfile"]);
            let mut command = wtpp_config::cwd::POWERSHELL_PROMPT_HOOK.to_string();
            if let Some(args) = run {
                command = format!("{command}; {}", wtpp_config::ssh::POWERSHELL_SSH_RUN);
                p.env.insert("WTPP_SSH".into(), wtpp_config::ssh::env_value(args));
                p.env.insert("WTPP_SSH_PROGRAM".into(), ssh.clone());
            }
            p.args = wtpp_config::cwd::powershell_args_with_command("powershell.exe", &p.args, &command).unwrap();
            p.cwd = std::env::var("SystemRoot").ok().map(Into::into);
            Session::spawn(&p, SIZE, |_| {}).unwrap()
        };
        let parent = ps(None);
        assert!(wait(30, || parent.reported_cwd().is_some()));
        type_line(&parent, &format!("ssh {}", typed.join(" ")));
        assert!(wait(10, || parent.foreground_ssh().is_some()), "{:?}", screen(&parent));
        let found = parent.foreground_ssh().unwrap();
        assert!(found.program.to_string_lossy().eq_ignore_ascii_case(&ssh), "{found:?}");
        assert_eq!(found.args, typed);
        let args = wtpp_config::ssh::session_args(&found.args).unwrap();

        let child = ps(Some(&args));
        assert!(wait(30, || child.foreground_ssh().is_some()), "{:?}", screen(&child));
        assert_eq!(child.foreground_ssh().unwrap().args, typed);
        let mut cmd = Profile::new("cmd", "cmd.exe", &[]);
        cmd.args = wtpp_config::ssh::cmd_args(&ssh, &args).unwrap();
        let cmd = Session::spawn(&cmd, SIZE, |_| {}).unwrap();
        assert!(wait(30, || cmd.foreground_ssh().is_some()), "{:?}", screen(&cmd));
        assert_eq!(cmd.foreground_ssh().unwrap().args, typed);
        for session in [&parent, &child, &cmd] {
            session.write(&b"\x03"[..]);
            assert!(wait(10, || session.foreground_ssh().is_none()), "{:?}", screen(session));
        }
        // Back at their shells.
        type_line(&child, "echo back-$(1+1)");
        assert!(wait(10, || shows(&child, "back-2")), "{:?}", screen(&child));
    }

    /// An `ssh` alias or function of the user's stays theirs.
    #[cfg(unix)]
    #[test]
    fn users_own_ssh() {
        let dir = scratch("sshown");
        let rc = dir.join("rc");
        std::fs::write(&rc, "alias ssh='echo mine'\n").unwrap();
        let rc_text = rc.to_string_lossy().into_owned();
        let session = Session::spawn(&shell("bash", &["--noprofile", "--rcfile", &rc_text, "-i"], &dir, None, None), SIZE, |_| {}).unwrap();
        assert!(wait(20, || session.reported_cwd().is_some()));
        type_line(&session, "echo \"k=$(type -t ssh)\"");
        assert!(wait(10, || shows(&session, "k=alias")), "{:?}", screen(&session));
        std::fs::remove_dir_all(&dir).ok();
    }
}
