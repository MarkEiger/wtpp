//! A terminal session: one child process on a PTY, plus the VT state it drives.
//!
//! PTY IO and VT parsing run on a background thread (alacritty_terminal's
//! event loop). The UI thread locks [`Session::term`] to read the grid when it
//! renders, and receives [`SessionEvent`]s through the callback it supplied.

pub mod profiles;

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

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

/// Bridges alacritty's events to our callback, and answers terminal queries
/// (DSR, color requests, …) that need to be written back to the PTY.
#[derive(Clone)]
pub struct Listener {
    callback: Arc<Callback>,
    pty: Arc<OnceLock<EventLoopSender>>,
    size: Arc<FairMutex<GridSize>>,
}

impl Listener {
    fn reply(&self, text: String) {
        if let Some(pty) = self.pty.get() {
            let _ = pty.send(Msg::Input(Cow::Owned(text.into_bytes())));
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
            Event::Bell => cb(SessionEvent::Bell),
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
}

pub struct Session {
    term: Arc<FairMutex<Term<Listener>>>,
    pty: EventLoopSender,
    size: Arc<FairMutex<GridSize>>,
    profile: Profile,
}

impl Session {
    pub fn spawn(
        profile: &Profile,
        size: GridSize,
        on_event: impl Fn(SessionEvent) + Send + Sync + 'static,
    ) -> anyhow::Result<Session> {
        let shared_size = Arc::new(FairMutex::new(size));
        let listener = Listener {
            callback: Arc::new(on_event),
            pty: Arc::new(OnceLock::new()),
            size: shared_size.clone(),
        };

        let config = Config {
            kitty_keyboard: true,
            scrolling_history: profile.history,
            default_cursor_style: CursorStyle { shape: profile.cursor_shape, blinking: false },
            semantic_escape_chars: profile.word_delimiters.clone(),
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));

        let mut env = profile.env.clone();
        env.entry("TERM".into()).or_insert_with(|| "xterm-256color".into());
        env.entry("COLORTERM".into()).or_insert_with(|| "truecolor".into());
        env.insert("TERM_PROGRAM".into(), "wtpp".into());

        let options = tty::Options {
            shell: Some(tty::Shell::new(profile.program.clone(), profile.args.clone())),
            working_directory: profile.cwd.clone(),
            drain_on_exit: true,
            env,
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, size.window_size(), 0)?;

        let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, true, false)?;
        let sender = event_loop.channel();
        let _ = listener.pty.set(sender.clone());
        event_loop.spawn();

        Ok(Session { term, pty: sender, size: shared_size, profile: profile.clone() })
    }

    pub fn term(&self) -> &Arc<FairMutex<Term<Listener>>> {
        &self.term
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
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
        if let Err(e) = self.pty.send(Msg::Input(bytes)) {
            log::warn!("write to pty failed: {e:?}");
        }
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
        let _ = self.pty.send(Msg::Resize(size.window_size()));
    }

    /// Key-encoding options reflecting the profile and the modes the running
    /// app has requested.
    pub fn encode_options(&self) -> EncodeOptions {
        let mode = *self.term.lock().mode();
        EncodeOptions {
            style: self.profile.key_style,
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            kitty_disambiguate: mode.contains(TermMode::DISAMBIGUATE_ESC_CODES),
            ..EncodeOptions::default()
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.pty.send(Msg::Shutdown);
    }
}
