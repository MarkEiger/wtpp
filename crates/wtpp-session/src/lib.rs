//! A terminal session: one child process on a PTY, plus the VT state it drives.
//!
//! PTY IO and VT parsing run on background threads (alacritty_terminal's
//! event loop, or ours for a console handed over by Windows; see [`Pipes`]). The UI thread locks [`Session::term`] to read the grid when it
//! renders, and receives [`SessionEvent`]s through the callback it supplied.

pub mod cwd;
pub mod env;
pub mod profiles;

use std::borrow::Cow;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

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
    reported_cwd: Arc<Mutex<Option<String>>>,
}

impl Listener {
    fn reply(&self, text: String) {
        if let Some(pty) = self.pty.get() {
            pty.input(Cow::Owned(text.into_bytes()));
        }
    }

    fn scan_output(&self, bytes: &[u8]) {
        const KEPT: usize = 160;
        if let Ok(mut recent) = self.recent_output.lock() {
            recent.extend_from_slice(&bytes[bytes.len().saturating_sub(KEPT)..]);
            let extra = recent.len().saturating_sub(KEPT);
            recent.drain(..extra);
        }
        let found = self.cwd_scanner.lock().ok().and_then(|mut scanner| scanner.feed(bytes));
        if let Some(dir) = found
            && let Ok(mut reported) = self.reported_cwd.lock()
        {
            *reported = Some(dir);
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
    reported_cwd: Arc<Mutex<Option<String>>>,
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
        Ok(Session { term, io, size: shared_size, profile: profile.clone(), pid, reported_cwd, _keep_alive: Vec::new() })
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
        // outlive it); its last output gets a moment to arrive.
        let callback = listener.callback.clone();
        std::thread::Builder::new().name("handoff-exit".into()).spawn(move || {
            let code = wait_exit();
            std::thread::sleep(std::time::Duration::from_millis(150));
            callback(SessionEvent::Exited(code));
        })?;

        let io = Io::Pipes { input: tx, signal: Arc::new(Mutex::new(Some(signal))) };
        let _ = listener.pty.set(io.clone());
        // The console host starts at its own default size.
        io.resize(size);
        let reported_cwd = listener.reported_cwd.clone();
        Ok(Session { term, io, size: shared_size, profile: profile.clone(), pid, reported_cwd, _keep_alive: keep_alive })
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
        self.reported_cwd.lock().ok()?.clone()
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
            wsl.env.extend(wtpp_config::cwd::wsl_prompt_env(false, None));
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
