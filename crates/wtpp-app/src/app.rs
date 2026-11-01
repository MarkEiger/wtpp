//! Window state: tabs, their pane trees, and the sessions behind each pane.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use slint::wgpu_30::wgpu;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use wtpp_config::settings::{DiscoveredProfile, expand_env, join_commandline, split_commandline};
use wtpp_config::{Action, BellStyle, CloseOnExit, CursorShape, KeyEncoding, Keymap, ResolvedProfile, Settings};
use wtpp_input::{Key, KeyStyle, Mods};
use wtpp_render::{Frame, Highlight, HighlightKind, Renderer, SharedFonts, Theme};
use wtpp_session::alacritty_terminal::grid::{Dimensions, Scroll};
use wtpp_session::alacritty_terminal::index::{Column, Point, Side};
use wtpp_session::alacritty_terminal::selection::{Selection, SelectionType};
use wtpp_session::alacritty_terminal::term::{TermMode, viewport_to_point};
use wtpp_session::alacritty_terminal::vte::ansi::{self, CursorShape as TermCursor};
use wtpp_session::{GridSize, Session, SessionEvent};

use crate::pane_tree::{self, Direction, Node, PaneId, Rect};
use crate::settings_ui::Draft;
use crate::{DividerData, MainWindow, MenuEntry, PaneData, TabData, keys};

/// Space between split panes, in logical pixels.
const PANE_GAP: f32 = 2.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Renderers are shared by panes with the same font face and pixel size.
/// Face, size (1/64 px), weight, line height (1/100), ligatures.
type FontKey = (String, u32, u16, u32, bool);

pub(crate) struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

pub(crate) struct Pane {
    session: Session,
    pub(crate) profile: ResolvedProfile,
    theme: Theme,
    title: Option<String>,
    /// Logical rectangle inside the content area.
    rect: Rect,
    /// Render target; recreated when the grid size changes.
    target: Option<(wgpu::Texture, u32, u32)>,
    selecting: bool,
    /// The shell is cmd, whose line editor lacks Ctrl+Delete.
    emulate_kill_word: bool,
    /// The shell has exited and the pane stays open (see `closeOnExit`).
    exited: bool,
    /// Time, cell and count of the last left click, for double/triple click.
    last_click: Option<(Instant, Point, u8)>,
    /// Sub-line remainder of wheel scrolling.
    scroll_remainder: f32,
    /// Lines scrolled in a mouse-mode app not yet reported (see on_scroll).
    wheel_lines: i32,
}

/// A tab on its way to another window, with its live panes.
pub(crate) struct MovedTab {
    tab: Tab,
    panes: Vec<(PaneId, Pane)>,
}

/// What the open confirmation dialog is about.
pub(crate) enum Dialog {
    CloseWindow,
    Paste(String),
}

pub(crate) enum TabKind {
    /// `zoomed`: the focused pane fills the tab (togglePaneZoom).
    Terminal { root: Node, focused: PaneId, zoomed: bool },
    Settings,
}

pub(crate) struct Tab {
    pub(crate) kind: TabKind,
    /// Set by "Rename tab"; overrides the shell's title.
    pub(crate) title: Option<String>,
    pub(crate) renaming: bool,
    /// A bell rang in this tab while it wasn't being looked at.
    pub(crate) bell: bool,
    /// Set from the tab menu; overrides the profile's `tabColor`.
    pub(crate) color: Option<wtpp_config::Color>,
}

impl Tab {
    pub(crate) fn new(kind: TabKind) -> Tab {
        Tab { kind, title: None, renaming: false, bell: false, color: None }
    }
}

pub struct App {
    /// This window's id (see `crate::with_window`).
    pub(crate) id: crate::WindowId,
    /// Tab row geometry for the Windows title bar subclass.
    geometry: Arc<crate::titlebar::Geometry>,
    pub(crate) ui: slint::Weak<MainWindow>,
    pub(crate) gpu: Option<Gpu>,
    /// Slint's device and queue, held until the fonts finish loading.
    pub(crate) gpu_pending: Option<(wgpu::Device, wgpu::Queue)>,
    pub(crate) fonts: Option<SharedFonts>,
    pub(crate) font_families: Vec<String>,
    pub(crate) renderers: HashMap<FontKey, Renderer>,
    pub(crate) settings: Settings,
    pub(crate) settings_path: PathBuf,
    /// Visible profiles, in new-tab menu order.
    pub(crate) profiles: Vec<ResolvedProfile>,
    pub(crate) keymap: Keymap,
    pub(crate) panes: HashMap<PaneId, Pane>,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    /// Panes of the active tab, in the order of the UI's pane model.
    pub(crate) visible: Vec<PaneId>,
    /// Ctrl+=/Ctrl+- adjustment applied on top of every profile's font size.
    pub(crate) font_zoom: f32,
    pub(crate) clipboard: Option<arboard::Clipboard>,
    pub(crate) tabs_model: Rc<VecModel<TabData>>,
    pub(crate) panes_model: Rc<VecModel<PaneData>>,
    /// Set while a redraw is queued on the UI thread, so a flood of PTY
    /// output coalesces into one frame.
    pub(crate) redraw_pending: Arc<AtomicBool>,
    pub(crate) draft: Option<Draft>,
    /// Cursor blink phase for the focused pane.
    pub(crate) cursor_on: bool,
    /// Index of the tab being dragged in the tab row.
    pub(crate) dragging_tab: Option<usize>,
    /// The dragged tab left the tab row (see tabs.rs).
    pub(crate) tab_lift: Option<crate::tabs::TabLift>,
    /// Command palette entries currently listed.
    pub(crate) palette: Vec<Action>,
    pub(crate) find: crate::find::FindState,
    pub(crate) dialog: Option<Dialog>,
    /// A tab moved here from another window, opened instead of a new one.
    pub(crate) adopt: Option<MovedTab>,
    /// A console Windows handed over, opened instead of a new tab.
    #[cfg(windows)]
    pub(crate) handoff: Option<crate::default_terminal::Handoff>,
    /// Keyboard selection in progress (mark mode).
    pub(crate) mark: Option<crate::mark::MarkState>,
    /// Profile icons by their source (icon setting or command line).
    icon_cache: HashMap<String, (slint::Image, String)>,
    /// The window size chosen at startup, re-applied once the title bar is
    /// installed (see `titlebar::set_client_size`).
    pub(crate) launch_size: Option<slint::WindowSize>,
    /// The window has keyboard focus.
    pub(crate) window_active: bool,
    /// Pane currently flashing for a visual bell.
    pub(crate) flashing: Option<PaneId>,
    /// Link under the pointer while Ctrl is held: pane and range.
    pub(crate) hover_link: Option<(PaneId, Point, Point)>,
    /// Command-line arguments, used for the first tab.
    pub(crate) startup_args: Option<crate::cli::Args>,
    /// Dividers of the active tab, in the UI model's order.
    pub(crate) dividers: Vec<crate::pane_tree::Divider>,
    dividers_model: Rc<VecModel<DividerData>>,
    /// Allow the window to close without asking again.
    pub(crate) close_confirmed: bool,
}

impl App {
    pub fn new(ui: &MainWindow, settings: Settings, settings_path: PathBuf, id: crate::WindowId) -> App {
        let tabs_model = Rc::new(VecModel::default());
        let panes_model = Rc::new(VecModel::default());
        let dividers_model = Rc::new(VecModel::default());
        ui.set_dividers(ModelRc::from(dividers_model.clone()));
        ui.set_tabs(ModelRc::from(tabs_model.clone()));
        ui.set_panes(ModelRc::from(panes_model.clone()));
        let (keymap, warnings) = settings.keymap();
        for w in warnings {
            log::warn!("settings: {w}");
        }
        let app = App {
            id,
            geometry: Arc::new(crate::titlebar::Geometry::new(id)),
            ui: ui.as_weak(),
            gpu: None,
            gpu_pending: None,
            fonts: None,
            font_families: Vec::new(),
            renderers: HashMap::new(),
            profiles: settings.visible_profiles(),
            keymap,
            settings,
            settings_path,
            panes: HashMap::new(),
            tabs: Vec::new(),
            active: 0,
            visible: Vec::new(),
            font_zoom: 0.0,
            clipboard: arboard::Clipboard::new().map_err(|e| log::warn!("no clipboard: {e}")).ok(),
            tabs_model,
            panes_model,
            redraw_pending: Arc::new(AtomicBool::new(false)),
            draft: None,
            cursor_on: true,
            dragging_tab: None,
            tab_lift: None,
            palette: Vec::new(),
            find: Default::default(),
            dialog: None,
            adopt: None,
            #[cfg(windows)]
            handoff: None,
            mark: None,
            icon_cache: HashMap::new(),
            launch_size: None,
            window_active: true,
            flashing: None,
            hover_link: None,
            startup_args: None,
            dividers: Vec::new(),
            dividers_model,
            close_confirmed: false,
        };
        app.apply_chrome_settings();
        app
    }

    /// Theme and other window-level appearance settings.
    fn apply_chrome_settings(&self) {
        let light = match self.settings.theme {
            wtpp_config::AppTheme::Light => true,
            wtpp_config::AppTheme::Dark => false,
            wtpp_config::AppTheme::System => crate::platform::system_prefers_light(),
        };
        let ui = self.ui();
        ui.invoke_apply_theme(light);
        crate::platform::native_menu_theme(!light);
        ui.set_terminal_menu(self.settings.right_click_context_menu);
        ui.set_tab_width_mode(match self.settings.tab_width_mode {
            wtpp_config::TabWidthMode::Equal => 0,
            wtpp_config::TabWidthMode::TitleLength => 1,
            wtpp_config::TabWidthMode::Compact => 2,
        });
    }

    /// Cache key of a profile's icon.
    fn icon_key(profile: &ResolvedProfile) -> String {
        profile.icon.clone().unwrap_or_else(|| profile.commandline.clone())
    }

    /// Load a profile's icon into the cache: the `icon` setting (an image
    /// file, or short text like an emoji), else the program's own icon.
    fn ensure_icon(&mut self, profile: &ResolvedProfile) {
        let key = App::icon_key(profile);
        if self.icon_cache.contains_key(&key) {
            return;
        }
        let from_program = || {
            let (program, _) = split_commandline(&expand_env(&profile.commandline));
            crate::platform::exe_icon(&program).unwrap_or_default()
        };
        let icon = match &profile.icon {
            Some(icon) if icon.chars().count() <= 4 && !icon.contains(['/', '\\', '.']) => {
                (slint::Image::default(), icon.clone())
            }
            Some(icon) => {
                let path = expand_env(icon);
                // Slint can't decode .ico; Windows' icon extractor can.
                let image = slint::Image::load_from_path(std::path::Path::new(&path))
                    .ok()
                    .or_else(|| crate::platform::exe_icon(&path))
                    .unwrap_or_else(from_program);
                (image, String::new())
            }
            None => (from_program(), String::new()),
        };
        self.icon_cache.insert(key, icon);
    }

    fn icon(&self, profile: &ResolvedProfile) -> (slint::Image, String) {
        self.icon_cache.get(&App::icon_key(profile)).cloned().unwrap_or_default()
    }

    pub(crate) fn ui(&self) -> MainWindow {
        self.ui.upgrade().expect("UI outlives the app state")
    }

    fn scale(&self) -> f32 {
        self.ui().window().scale_factor()
    }

    // ---- startup ------------------------------------------------------------

    pub fn on_gpu_ready(&mut self, device: wgpu::Device, queue: wgpu::Queue) {
        self.gpu_pending = Some((device, queue));
        self.try_start();
    }

    /// Fonts and shell discovery finished on the background thread.
    /// The (shared) font database is ready. `discovered` is the shell list,
    /// merged into the settings by the first window only.
    pub fn on_assets_loaded(
        &mut self,
        fonts: SharedFonts,
        families: Vec<String>,
        discovered: Option<&[wtpp_session::Profile]>,
    ) {
        self.font_families = families;
        self.fonts = Some(fonts);
        if let Some(discovered) = discovered {
            self.merge_discovered(discovered);
        }
        self.profiles = self.settings.visible_profiles();
        self.update_menu();
        self.try_start();
    }

    fn merge_discovered(&mut self, discovered: &[wtpp_session::Profile]) {
        let found: Vec<DiscoveredProfile> = discovered
            .iter()
            .map(|p| DiscoveredProfile {
                name: p.name.clone(),
                commandline: join_commandline(&p.program, &p.args),
                icon: p.icon.clone(),
            })
            .collect();
        let first_run = !self.settings_path.exists();
        if (self.settings.merge_discovered(&found) || first_run)
            && let Err(e) = self.settings.save(&self.settings_path)
        {
            log::warn!("could not write {}: {e}", self.settings_path.display());
        }
    }

    /// Open the first tab once both the GPU and the background-loaded fonts
    /// and profiles are available.
    fn try_start(&mut self) {
        if self.gpu.is_some() || self.fonts.is_none() || self.gpu_pending.is_none() {
            return;
        }
        let (device, queue) = self.gpu_pending.take().unwrap();
        self.gpu = Some(Gpu { device, queue });
        self.ui().set_loading(false);
        if let Some(moved) = self.adopt.take() {
            self.adopt_tab(moved);
            return;
        }
        #[cfg(windows)]
        if let Some(handoff) = self.handoff.take() {
            self.open_handoff(handoff);
            return;
        }
        let args = self.startup_args.take().unwrap_or_default();
        let wanted = args.profile.as_deref().and_then(|name| {
            let found = self.profiles.iter().find(|p| p.name.eq_ignore_ascii_case(name) || p.guid.eq_ignore_ascii_case(name));
            if found.is_none() {
                log::warn!("no profile named {name:?}; using the default");
            }
            found
        });
        let Some(mut profile) = wanted.or(self.profiles.get(self.settings.default_profile_index())).cloned() else {
            log::error!("no profiles");
            crate::close_window(self.id);
            return;
        };
        if let Some(dir) = args.starting_directory {
            profile.starting_directory = Some(dir.to_string_lossy().into_owned());
            // WSL profiles open in the Linux home unless told otherwise.
            profile.commandline = profile.commandline.replace(" --cd ~", "");
        }
        if let Some(commandline) = args.commandline {
            profile.commandline = commandline;
        }
        self.new_tab_with(profile);
        if self.tabs.is_empty() {
            log::error!("could not start any shell");
            crate::close_window(self.id);
        }
    }

    pub(crate) fn update_menu(&mut self) {
        for p in self.profiles.clone() {
            self.ensure_icon(&p);
        }
        let mut entries: Vec<MenuEntry> = self
            .profiles
            .iter()
            .enumerate()
            .map(|(i, p)| MenuEntry {
                icon: self.icon(p).0,
                icon_text: self.icon(p).1.into(),
                label: p.name.clone().into(),
                shortcut: self.keymap.chord_label(&Action::NewTab(Some(i))).unwrap_or_default().into(),
                enabled: true,
            })
            .collect();
        entries.push(MenuEntry {
            icon: Default::default(),
            icon_text: "⚙".into(),
            label: "Settings".into(),
            shortcut: self.keymap.chord_label(&Action::OpenSettings).unwrap_or_default().into(),
            enabled: true,
        });
        entries.push(MenuEntry {
            icon: Default::default(),
            icon_text: "⌨".into(),
            label: "Keyboard shortcuts".into(),
            shortcut: Default::default(),
            enabled: true,
        });
        self.ui().set_menu_entries(ModelRc::new(VecModel::from(entries)));
    }

    /// Apply new settings to everything that's running.
    pub(crate) fn apply_settings(&mut self, settings: Settings) {
        let (keymap, warnings) = settings.keymap();
        for w in warnings {
            log::warn!("settings: {w}");
        }
        self.keymap = keymap;
        self.settings = settings;
        self.profiles = self.settings.visible_profiles();
        let all: Vec<ResolvedProfile> = self.settings.profiles.list.iter().map(|p| self.settings.resolve(p)).collect();
        for pane in self.panes.values_mut() {
            if let Some(updated) = all.iter().find(|p| p.guid == pane.profile.guid) {
                pane.profile = updated.clone();
                pane.theme = theme_for(&self.settings, updated);
            }
        }
        self.icon_cache.clear();
        for pane in self.panes.values().map(|p| p.profile.clone()).collect::<Vec<_>>() {
            self.ensure_icon(&pane);
        }
        // Linux draws its own title bar or not by a window flag: switch now.
        #[cfg(all(unix, not(target_os = "macos")))]
        self.ui().set_linux_titlebar(self.settings.show_tabs_in_titlebar);
        self.apply_chrome_settings();
        self.update_menu();
        self.relayout();
    }

    // ---- tabs & panes ---------------------------------------------------------

    fn content_area(&self) -> Rect {
        let ui = self.ui();
        Rect { x: 0.0, y: 0.0, w: ui.get_content_width(), h: ui.get_content_height() }
    }

    fn font_key(&self, profile: &ResolvedProfile) -> FontKey {
        let px = (profile.font_size + self.font_zoom).clamp(4.0, 96.0) * 96.0 / 72.0 * self.scale();
        (
            profile.font_face.clone(),
            (px * 64.0).round() as u32,
            profile.font_weight,
            (profile.cell_height * 100.0).round() as u32,
            profile.ligatures,
        )
    }

    /// The renderer for a profile's font, created on first use.
    fn renderer(&mut self, profile: &ResolvedProfile) -> Option<&mut Renderer> {
        let key = self.font_key(profile);
        let (gpu, fonts) = (self.gpu.as_ref()?, self.fonts.as_ref()?);
        Some(self.renderers.entry(key.clone()).or_insert_with(|| {
            let spec = wtpp_render::FontSpec {
                face: key.0.clone(),
                size: key.1 as f32 / 64.0,
                weight: key.2,
                cell_height: key.3 as f32 / 100.0,
                ligatures: key.4,
            };
            Renderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, &spec, fonts.clone())
        }))
    }

    fn grid_size(&mut self, profile: &ResolvedProfile, rect: Rect) -> Option<GridSize> {
        let scale = self.scale();
        let cell = self.renderer(profile)?.cell_metrics();
        let [left, top, right, bottom] = profile.padding;
        let width = ((rect.w - left - right) * scale).max(0.0);
        let height = ((rect.h - top - bottom) * scale).max(0.0);
        Some(GridSize {
            cols: ((width / cell.width) as u16).max(2),
            rows: ((height / cell.height) as u16).max(1),
            cell_width: cell.width as u16,
            cell_height: cell.height as u16,
        })
    }

    fn spawn_pane(&mut self, id: PaneId, profile: ResolvedProfile, rect: Rect) -> bool {
        self.spawn_pane_on(id, profile, rect, None)
    }

    /// Start a pane: the profile's shell, or with `pipes`, a console Windows
    /// handed over (wt++ as the default terminal).
    fn spawn_pane_on(
        &mut self,
        id: PaneId,
        profile: ResolvedProfile,
        rect: Rect,
        pipes: Option<wtpp_session::Pipes>,
    ) -> bool {
        let Some(size) = self.grid_size(&profile, rect) else { return false };
        let session_profile = self.session_profile(&profile);
        let pending = self.redraw_pending.clone();
        let on_event = move |event| match event {
            SessionEvent::Wakeup => {
                if !pending.swap(true, Ordering::AcqRel) {
                    // Clear this flag, not the current window's: the tab may
                    // have moved to another window since.
                    let pending = pending.clone();
                    slint::invoke_from_event_loop(move || {
                        // Draw once per frame, on a timer: redrawing on every
                        // burst of output kept the event loop so busy under a
                        // flood (`cat` of a big file) that keys arrived seconds
                        // late on X11.
                        slint::Timer::single_shot(FRAME, move || {
                            pending.store(false, Ordering::Release);
                            crate::with_pane(id, |app| app.render());
                        });
                    })
                    .ok();
                }
            }
            SessionEvent::Title(title) => {
                slint::invoke_from_event_loop(move || {
                    crate::with_pane(id, |app| app.on_title(id, title));
                })
                .ok();
            }
            SessionEvent::Exited(code) => {
                slint::invoke_from_event_loop(move || {
                    crate::with_pane(id, |app| app.on_pane_exited(id, code));
                })
                .ok();
            }
            SessionEvent::Bell => {
                slint::invoke_from_event_loop(move || {
                    crate::with_pane(id, |app| app.on_bell(id));
                })
                .ok();
            }
            SessionEvent::ClipboardStore(text) => {
                slint::invoke_from_event_loop(move || {
                    crate::with_pane(id, |app| app.on_osc52(text));
                })
                .ok();
            }
        };
        let session = match pipes {
            Some(pipes) => Session::from_pipes(&session_profile, size, pipes, on_event),
            None => Session::spawn(&session_profile, size, on_event),
        };
        match session {
            Ok(session) => {
                log::info!("started {} ({})", profile.name, profile.commandline);
                self.ensure_icon(&profile);
                let theme = theme_for(&self.settings, &profile);
                let emulate_kill_word = cfg!(windows) && is_cmd(&session_profile.program);
                self.panes.insert(
                    id,
                    Pane {
                        session,
                        profile,
                        theme,
                        title: None,
                        rect,
                        target: None,
                        selecting: false,
                        emulate_kill_word,
                        exited: false,
                        last_click: None,
                        scroll_remainder: 0.0,
                        wheel_lines: 0,
                    },
                );
                true
            }
            Err(e) => {
                log::error!("failed to start {}: {e:#}", profile.commandline);
                false
            }
        }
    }

    /// Everything the session needs to start a profile's shell.
    fn session_profile(&self, p: &ResolvedProfile) -> wtpp_session::Profile {
        let (program, args) = split_commandline(&expand_env(&p.commandline));
        let mut profile = wtpp_session::Profile::new(&p.name, &program, &[]);
        profile.args = args;
        profile.cwd = Some(match &p.starting_directory {
            Some(dir) if !dir.trim().is_empty() => PathBuf::from(expand_env(dir)),
            _ => home_dir(),
        });
        profile.key_style = match p.key_encoding {
            KeyEncoding::Auto => wtpp_session::profiles::key_style_for(&program),
            KeyEncoding::Unix => KeyStyle::Unix,
            KeyEncoding::Windows => KeyStyle::Windows,
        };
        profile.history = p.history_size as usize;
        profile.cursor_shape = match p.cursor_shape {
            CursorShape::Bar => TermCursor::Beam,
            CursorShape::Underscore | CursorShape::DoubleUnderscore | CursorShape::Vintage => TermCursor::Underline,
            CursorShape::FilledBox => TermCursor::Block,
            CursorShape::EmptyBox => TermCursor::HollowBlock,
        };
        profile.word_delimiters = self.settings.word_delimiters.clone();
        profile.env = p.environment.iter().map(|(k, v)| (k.clone(), expand_env(v))).collect();
        profile
    }

    /// Pane ids are unique across windows, so tabs can move between them.
    fn alloc_id(&mut self) -> PaneId {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn active_terminal(&self) -> Option<(&Node, PaneId)> {
        match &self.tabs.get(self.active)?.kind {
            TabKind::Terminal { root, focused, .. } => Some((root, *focused)),
            TabKind::Settings => None,
        }
    }

    pub fn new_tab(&mut self, profile_index: usize) {
        let Some(profile) = self.profiles.get(profile_index).cloned() else { return };
        self.new_tab_with(profile);
    }

    /// A console program Windows handed to us (wt++ is the default
    /// terminal): a tab for it, styled like the profile that runs the same
    /// program (cmd.exe → Command Prompt), or else the default profile.
    #[cfg(windows)]
    pub(crate) fn open_handoff(&mut self, handoff: crate::default_terminal::Handoff) {
        let program = handoff.program().unwrap_or_else(|| "console".into());
        let file = |s: &str| s.rsplit(['/', '\\']).next().unwrap_or(s).to_ascii_lowercase();
        let name = file(&program);
        let matching = self.profiles.iter().find(|p| file(&split_commandline(&expand_env(&p.commandline)).0) == name);
        let Some(mut profile) = matching.or(self.profiles.get(self.settings.default_profile_index())).cloned() else { return };
        if matching.is_none() {
            profile.name = name.trim_end_matches(".exe").to_string();
            profile.icon = None;
        }
        profile.commandline = format!("\"{program}\"");
        profile.key_encoding = KeyEncoding::Windows;
        let id = self.alloc_id();
        if !self.spawn_pane_on(id, profile, self.content_area(), Some(handoff.into_pipes())) {
            return;
        }
        self.tabs.push(Tab::new(TabKind::Terminal { root: Node::Leaf(id), focused: id, zoomed: false }));
        self.active = self.tabs.len() - 1;
        self.relayout();
    }

    fn new_tab_with(&mut self, profile: ResolvedProfile) {
        let id = self.alloc_id();
        if !self.spawn_pane(id, profile, self.content_area()) {
            return;
        }
        self.tabs.push(Tab::new(TabKind::Terminal { root: Node::Leaf(id), focused: id, zoomed: false }));
        self.active = self.tabs.len() - 1;
        self.relayout();
    }

    fn split(&mut self, side: Option<Direction>, duplicate: bool) {
        let Some((_, target)) = self.active_terminal() else { return };
        let Some(pane) = self.panes.get(&target) else { return };
        let side = side.unwrap_or(if pane.rect.w >= pane.rect.h { Direction::Right } else { Direction::Down });
        let new_first = matches!(side, Direction::Left | Direction::Up);
        let profile = if duplicate {
            pane.profile.clone()
        } else {
            match self.profiles.get(self.settings.default_profile_index()) {
                Some(p) => p.clone(),
                None => return,
            }
        };

        let id = self.alloc_id();
        let area = self.content_area();
        let TabKind::Terminal { root, zoomed, .. } = &mut self.tabs[self.active].kind else { return };
        *zoomed = false;
        root.split(target, id, side.axis(), new_first);
        let rect = root.layout(area, PANE_GAP).into_iter().find(|(p, _)| *p == id).unwrap().1;
        let spawned = self.spawn_pane(id, profile, rect);
        if let TabKind::Terminal { root, focused, .. } = &mut self.tabs[self.active].kind {
            if spawned {
                *focused = id;
            } else {
                root.remove(id);
            }
        }
        self.relayout();
    }

    pub fn close_pane(&mut self, id: PaneId) {
        let Some(tab_index) = self.tabs.iter().position(|t| match &t.kind {
            TabKind::Terminal { root, .. } => root.contains(id),
            TabKind::Settings => false,
        }) else {
            return;
        };
        self.panes.remove(&id);
        let TabKind::Terminal { root, focused, zoomed } = &mut self.tabs[tab_index].kind else { return };
        if *root == Node::Leaf(id) {
            self.remove_tab(tab_index);
            return;
        }
        root.remove(id);
        *zoomed = false;
        if *focused == id {
            *focused = root.panes()[0];
        }
        self.relayout();
    }

    fn remove_tab(&mut self, index: usize) {
        let tab = self.tabs.remove(index);
        match tab.kind {
            TabKind::Terminal { root, .. } => {
                for id in root.panes() {
                    self.panes.remove(&id);
                }
            }
            TabKind::Settings => self.draft = None,
        }
        if self.tabs.is_empty() {
            self.close_confirmed = true;
            crate::close_window(self.id);
            return;
        }
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.relayout();
    }

    /// Detach tab `index` (keeping its shells running) to move it to another
    /// window. Closes this window if it was the last tab.
    pub(crate) fn take_tab(&mut self, index: usize) -> Option<MovedTab> {
        if index >= self.tabs.len() || !matches!(self.tabs[index].kind, TabKind::Terminal { .. }) {
            return None;
        }
        let tab = self.tabs.remove(index);
        let TabKind::Terminal { root, .. } = &tab.kind else { unreachable!() };
        let panes = root
            .panes()
            .into_iter()
            .filter_map(|id| {
                let mut pane = self.panes.remove(&id)?;
                // Textures belong to this window's GPU device.
                pane.target = None;
                Some((id, pane))
            })
            .collect();
        if self.tabs.is_empty() {
            self.close_confirmed = true;
            crate::close_window(self.id);
        } else {
            self.active = self.active.min(self.tabs.len() - 1);
            self.relayout();
        }
        Some(MovedTab { tab, panes })
    }

    pub(crate) fn adopt_tab(&mut self, moved: MovedTab) {
        self.adopt_tab_at(moved, usize::MAX);
    }

    /// Add a tab from another window before tab `slot` (or last), and
    /// select it.
    pub(crate) fn adopt_tab_at(&mut self, moved: MovedTab, slot: usize) {
        for (id, pane) in moved.panes {
            self.ensure_icon(&pane.profile);
            self.panes.insert(id, pane);
        }
        let slot = slot.min(self.tabs.len());
        self.tabs.insert(slot, moved.tab);
        self.active = slot;
        self.relayout();
    }

    /// A click in the new-tab dropdown: profiles first, then "Settings" and
    /// "Keyboard shortcuts".
    pub fn on_menu(&mut self, index: usize) {
        if index < self.profiles.len() {
            self.new_tab(index);
        } else if index == self.profiles.len() {
            self.open_settings();
        } else {
            self.open_shortcuts();
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.remove_tab(index);
        }
    }

    pub fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() && index != self.active {
            self.active = index;
            self.tabs[index].bell = false;
            self.relayout();
        }
    }

    /// A pane rang the bell: sound and/or flash per its profile's
    /// `bellStyle`, and mark its tab if it isn't the one being looked at.
    pub fn on_bell(&mut self, id: PaneId) {
        let Some(pane) = self.panes.get(&id) else { return };
        let style = pane.profile.bell_style;
        if matches!(style, BellStyle::Audible | BellStyle::All) {
            crate::platform::bell();
        }
        if !matches!(style, BellStyle::Visual | BellStyle::All) {
            return;
        }
        let tab = self.tabs.iter().position(|t| matches!(&t.kind, TabKind::Terminal { root, .. } if root.contains(id)));
        if let Some(tab) = tab
            && (tab != self.active || !self.window_active)
        {
            self.tabs[tab].bell = true;
            self.refresh_chrome();
        }
        self.flashing = Some(id);
        self.render();
        let win = self.id;
        slint::Timer::single_shot(std::time::Duration::from_millis(120), move || {
            crate::with_window(win, |app| {
                app.flashing = None;
                app.render();
            });
        });
    }

    /// An app asked to set the clipboard (OSC 52).
    pub fn on_osc52(&mut self, text: String) {
        if !self.settings.allow_osc52 {
            log::info!("ignored an OSC 52 clipboard write (disabled in settings)");
            return;
        }
        if let Some(clipboard) = &mut self.clipboard
            && let Err(e) = clipboard.set_text(text)
        {
            log::warn!("OSC 52 copy failed: {e}");
        }
    }

    /// The window gained or lost keyboard focus.
    pub fn on_window_focus(&mut self, active: bool) {
        if self.window_active == active {
            return;
        }
        self.window_active = active;
        self.ui().set_window_active(active);
        if active && let Some(tab) = self.tabs.get_mut(self.active) {
            tab.bell = false;
            self.refresh_chrome();
        }
        self.render();
    }

    fn focus_pane(&mut self, id: PaneId) {
        if let Some(Tab { kind: TabKind::Terminal { focused, .. }, .. }) = self.tabs.get_mut(self.active)
            && *focused != id
        {
            *focused = id;
            self.refresh_chrome();
            self.render();
        }
    }

    fn focused_pane(&self) -> Option<&Pane> {
        self.panes.get(&self.active_terminal()?.1)
    }

    /// The focused pane's terminal, shareable while `self` is mutated.
    pub(crate) fn focused_term(
        &self,
    ) -> Option<Arc<wtpp_session::alacritty_terminal::sync::FairMutex<wtpp_session::alacritty_terminal::Term<wtpp_session::Listener>>>> {
        Some(self.focused_pane()?.session.term().clone())
    }

    pub(crate) fn run_named(&mut self, action: Action) {
        self.run_action(action);
    }

    /// The pane's shell exited. Depending on `closeOnExit`, close the pane
    /// or keep its output with a note, like WT.
    pub fn on_pane_exited(&mut self, id: PaneId, code: Option<i32>) {
        let Some(pane) = self.panes.get_mut(&id) else { return };
        if pane.exited {
            return;
        }
        pane.exited = true;
        let success = code.is_none_or(|c| c == 0);
        let close = match pane.profile.close_on_exit {
            CloseOnExit::Always => true,
            CloseOnExit::Graceful => success,
            CloseOnExit::Never => false,
        };
        if close {
            self.close_pane(id);
            return;
        }
        let code = code.unwrap_or(0);
        pane.session.feed(
            format!(
                "\r\n\x1b[0m\x1b[90m[process exited with code {code} (0x{:08x})]\x1b[0m\r\n\
                 You can now close this terminal with Ctrl+D, or press Enter to restart.\r\n",
                code as u32
            )
            .as_bytes(),
        );
        self.render();
    }

    /// Start the pane's profile again in the same place.
    fn restart_pane(&mut self, id: PaneId) {
        let Some(old) = self.panes.remove(&id) else { return };
        let (profile, rect) = (old.profile.clone(), old.rect);
        drop(old);
        if !self.spawn_pane(id, profile, rect) {
            self.close_pane(id);
            return;
        }
        self.relayout();
    }

    pub fn on_title(&mut self, id: PaneId, title: Option<String>) {
        if let Some(pane) = self.panes.get_mut(&id)
            && !pane.profile.suppress_application_title
        {
            pane.title = title;
            self.refresh_chrome();
        }
    }

    fn pane_title(&self, id: PaneId) -> String {
        let Some(pane) = self.panes.get(&id) else { return String::new() };
        match &pane.title {
            // cmd and PowerShell set their own exe path as the title; WT shows
            // the profile name instead, which is far more useful in a tab.
            Some(title) if !is_exe_path(title) => title.clone(),
            _ => pane.profile.tab_title.clone().unwrap_or_else(|| pane.profile.name.clone()),
        }
    }

    pub(crate) fn tab_title(&self, tab: &Tab) -> String {
        if let Some(title) = &tab.title {
            return title.clone();
        }
        match &tab.kind {
            TabKind::Terminal { focused, .. } => self.pane_title(*focused),
            TabKind::Settings => "Settings".into(),
        }
    }

    /// Recompute pane rectangles for the active tab, resize their sessions,
    /// and redraw.
    pub fn relayout(&mut self) {
        // Linux, own title bar: the maximize button shows the real state.
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use slint::winit_030::WinitWindowAccessor;
            let ui = self.ui();
            if ui.get_linux_titlebar() {
                ui.set_window_maximized(ui.window().with_winit_window(|w| w.is_maximized()).unwrap_or(false));
            }
        }
        // macOS fullscreen (green button or ⌃⌘F) hides the window buttons.
        #[cfg(target_os = "macos")]
        {
            use slint::winit_030::WinitWindowAccessor;
            let ui = self.ui();
            let fullscreen = ui.window().with_winit_window(|w| w.fullscreen().is_some()).unwrap_or(false);
            ui.set_window_fullscreen(fullscreen);
        }
        let show_settings = matches!(self.tabs.get(self.active), Some(Tab { kind: TabKind::Settings, .. }));
        let ui = self.ui();
        // Leaving Settings (⌘T, Ctrl+Tab, closing it): the terminal gets the
        // keys back, not a Settings control that's no longer shown.
        if ui.get_show_settings() && !show_settings {
            ui.invoke_focus_keys();
        }
        ui.set_show_settings(show_settings);
        let layout = self.active_layout();
        self.visible = layout.iter().map(|(id, _)| *id).collect();
        self.update_dividers();
        for (id, rect) in layout {
            let Some(profile) = self.panes.get(&id).map(|p| p.profile.clone()) else { continue };
            let size = self.grid_size(&profile, rect);
            if let (Some(pane), Some(size)) = (self.panes.get_mut(&id), size) {
                pane.rect = rect;
                pane.session.resize(size);
            }
        }
        self.refresh_chrome();
        self.render();
    }

    /// Replace the native title bar with our tab row (Windows only).
    pub fn install_titlebar(&mut self) {
        let ui = self.ui();
        let on_hover: crate::titlebar::OnHover = |win, hovered| {
            slint::invoke_from_event_loop(move || {
                crate::with_window(win, |app| app.ui().set_max_hovered(hovered));
            })
            .ok();
        };
        let on_max_click: crate::titlebar::OnMaxClick = |win| {
            slint::invoke_from_event_loop(move || {
                crate::with_window(win, |app| {
                    let ui = app.ui();
                    ui.window().set_maximized(!ui.window().is_maximized());
                });
            })
            .ok();
        };
        if crate::titlebar::install(ui.window(), &self.geometry, on_hover, on_max_click) {
            ui.set_custom_titlebar(true);
            let window = ui.window();
            if let Some(size) = self.launch_size.take()
                && !window.is_maximized()
                && !window.is_fullscreen()
            {
                let size = size.to_physical(window.scale_factor());
                crate::titlebar::set_client_size(window, size.width, size.height);
            }
        }
    }

    /// Tell the window subclass where the tab row's parts are. Also called
    /// after each frame: Slint lays out new tabs only when it draws, so
    /// right after a tab is added these can still be the old values.
    pub(crate) fn update_titlebar_geometry(&self) {
        use std::sync::atomic::Ordering::Release;
        let ui = self.ui();
        let scale = ui.window().scale_factor();
        let px = |v: f32| (v * scale).round() as i32;
        let g = &self.geometry;
        g.titlebar_height.store(px(ui.get_titlebar_height()), Release);
        g.drag_start.store(px(ui.get_drag_start()), Release);
        g.buttons_start.store(px(ui.get_buttons_start()), Release);
        g.button_width.store(px(46.0), Release);
        ui.set_window_maximized(ui.window().is_maximized());
    }

    /// Publish the active tab's dividers so they can be dragged.
    fn update_dividers(&mut self) {
        self.dividers = match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Terminal { root, zoomed: false, .. }) => root.dividers(self.content_area(), PANE_GAP),
            _ => Vec::new(),
        };
        let rows: Vec<DividerData> = self
            .dividers
            .iter()
            .map(|d| DividerData {
                x: d.rect.x,
                y: d.rect.y,
                width: d.rect.w,
                height: d.rect.h,
                vertical: d.axis == crate::pane_tree::Axis::Horizontal,
            })
            .collect();
        // In place while dragging: replacing the model would recreate the
        // divider being dragged and end the drag.
        if self.dividers_model.row_count() == rows.len() {
            for (i, row) in rows.into_iter().enumerate() {
                self.dividers_model.set_row_data(i, row);
            }
        } else {
            self.dividers_model.set_vec(rows);
        }
    }

    /// A divider is dragged to `(x, y)` in content coordinates.
    pub fn on_divider_drag(&mut self, index: usize, x: f32, y: f32) {
        let Some(d) = self.dividers.get(index).cloned() else { return };
        let ratio = match d.axis {
            crate::pane_tree::Axis::Horizontal => (x - d.area.x) / d.area.w.max(1.0),
            crate::pane_tree::Axis::Vertical => (y - d.area.y) / d.area.h.max(1.0),
        };
        if let Some(Tab { kind: TabKind::Terminal { root, .. }, .. }) = self.tabs.get_mut(self.active) {
            root.set_ratio(&d.path, ratio);
            self.relayout();
        }
    }

    /// Pane rectangles of the active tab (just the focused pane when zoomed).
    pub(crate) fn active_layout(&self) -> Vec<(PaneId, Rect)> {
        match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Terminal { focused, zoomed: true, .. }) => vec![(*focused, self.content_area())],
            Some(TabKind::Terminal { root, .. }) => root.layout(self.content_area(), PANE_GAP),
            _ => Vec::new(),
        }
    }

    /// Update the tab strip and window title.
    pub(crate) fn refresh_chrome(&self) {
        let tabs: Vec<TabData> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| TabData {
                title: self.tab_title(t).into(),
                active: i == self.active,
                is_settings: matches!(t.kind, TabKind::Settings),
                renaming: t.renaming,
                zoomed: matches!(t.kind, TabKind::Terminal { zoomed: true, .. }),
                color: tab_color(self, t).map_or(slint::Color::default(), |c| slint::Color::from_rgb_u8(c.0, c.1, c.2)),
                has_color: tab_color(self, t).is_some(),
                bell: t.bell,
                icon: tab_profile(self, t).map(|p| self.icon(p).0).unwrap_or_default(),
                icon_text: tab_profile(self, t).map(|p| self.icon(p).1).unwrap_or_default().into(),
            })
            .collect();
        // Update rows in place when the count is unchanged: replacing the
        // model would recreate the tab widgets and cancel a drag in progress.
        if self.tabs_model.row_count() == tabs.len() {
            for (i, tab) in tabs.into_iter().enumerate() {
                if self.tabs_model.row_data(i).as_ref() != Some(&tab) {
                    self.tabs_model.set_row_data(i, tab);
                }
            }
        } else {
            self.tabs_model.set_vec(tabs);
        }
        if let Some(tab) = self.tabs.get(self.active) {
            self.ui().set_session_title(self.tab_title(tab).into());
        }
        let ui = self.ui();
        ui.set_show_tab_row(
            ui.get_custom_titlebar()
                || ui.get_mac_titlebar()
                || ui.get_linux_titlebar()
                || self.settings.always_show_tabs
                || self.tabs.len() > 1,
        );
        self.update_titlebar_geometry();
    }

    // ---- rendering ------------------------------------------------------------

    pub fn render(&mut self) {
        let Some(focused) = self.active_terminal().map(|(_, f)| f) else {
            self.panes_model.set_vec(Vec::new());
            return;
        };
        let scale = self.scale();
        let mut rows = Vec::with_capacity(self.visible.len());
        for id in self.visible.clone() {
            let Some(profile) = self.panes.get(&id).map(|p| p.profile.clone()) else { continue };
            let key = self.font_key(&profile);
            if self.renderer(&profile).is_none() {
                return;
            }
            let (Some(gpu), Some(renderer), Some(pane)) =
                (&self.gpu, self.renderers.get_mut(&key), self.panes.get_mut(&id))
            else {
                continue;
            };
            let cell = renderer.cell_metrics();
            let size = pane.session.size();
            let width = (size.cols as f32 * cell.width) as u32;
            let height = (size.rows as f32 * cell.height) as u32;
            if !matches!(&pane.target, Some((_, w, h)) if *w == width && *h == height) {
                let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("wtpp pane"),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                pane.target = Some((texture, width, height));
            }
            let (texture, ..) = pane.target.as_ref().unwrap();
            let view = texture.create_view(&Default::default());
            let has_focus = id == focused && self.window_active;
            let (history, offset) = {
                let term = pane.session.term().lock();
                let mut highlights = Vec::new();
                if id == focused {
                    highlights.extend(crate::find::visible_matches(&self.find, &term));
                }
                if let Some((link_pane, start, end)) = self.hover_link
                    && link_pane == id
                {
                    highlights.push(Highlight { start, end, kind: HighlightKind::Link });
                }
                let frame = Frame {
                    term: &term,
                    theme: &pane.theme,
                    focused: has_focus,
                    cursor_visible: self.cursor_on,
                    highlights: &highlights,
                };
                renderer.render(&gpu.device, &gpu.queue, &view, width, height, &frame);
                (term.grid().history_size(), term.grid().display_offset())
            };
            let bg = pane.theme.background;
            let [left, top, ..] = pane.profile.padding;
            // Re-setting the image (even the same texture) is what tells
            // Slint the pane needs repainting.
            let image = slint::Image::try_from(texture.clone()).unwrap_or_default();
            rows.push(PaneData {
                x: pane.rect.x,
                y: pane.rect.y,
                width: pane.rect.w,
                height: pane.rect.h,
                image,
                image_width: width as f32 / scale,
                image_height: height as f32 / scale,
                padding_left: left,
                padding_top: top,
                background: slint::Color::from_rgb_u8(bg.r, bg.g, bg.b),
                focused: id == focused,
                flash: self.flashing == Some(id),
                link_hover: self.hover_link.is_some_and(|(p, ..)| p == id),
                scroll_visible: history > 0,
                thumb_top: if history == 0 { 1.0 } else { (history - offset) as f32 / history as f32 },
                thumb_size: size.rows as f32 / (size.rows as usize + history) as f32,
            });
        }

        if self.panes_model.row_count() == rows.len() {
            for (i, row) in rows.into_iter().enumerate() {
                self.panes_model.set_row_data(i, row);
            }
        } else {
            self.panes_model.set_vec(rows);
        }
    }

    // ---- keyboard ---------------------------------------------------------------

    /// Text an input method composed (e.g. a macOS dead key: ⌥E E → é).
    pub fn on_text_input(&mut self, text: &str) {
        let Some(pane) = self.focused_pane() else { return };
        if pane.exited || text.is_empty() {
            return;
        }
        pane.session.write(text.as_bytes().to_vec());
        self.wake_cursor();
    }

    /// A key that bubbled up from a control in Settings: only shortcuts.
    pub fn on_shortcut_key(&mut self, text: &str, mods: Mods) -> bool {
        let Some((key, mods)) = keys::from_slint(text, mods) else { return false };
        match self.keymap.lookup(key, mods).cloned() {
            Some(action) => self.run_action(action),
            None => false,
        }
    }

    pub fn on_key(&mut self, text: &str, mods: Mods) -> bool {
        let key = keys::from_slint(text, mods);
        // A non-Latin layout (Hebrew, Russian, ...) turns Ctrl+Shift+W into
        // Ctrl+Shift+ц. Like WT, shortcuts and Ctrl+letter control codes go
        // by the physical key; plain typing keeps the layout's text.
        let latin = match key {
            Some((Key::Char(c), m)) if !c.is_ascii() && (m.ctrl || m.alt || m.meta) => {
                crate::platform::physical_key_char().map(|l| (Key::Char(l), m))
            }
            _ => None,
        };
        if let Some((key, mods)) = key {
            if self.draft.as_ref().is_some_and(|d| d.recording.is_some()) {
                self.record_key(key, mods);
                return true;
            }
            if let Some((_, focused)) = self.active_terminal()
                && self.mark_active_for(focused)
            {
                match self.keymap.lookup(key, mods) {
                    Some(Action::MarkMode) => self.toggle_mark_mode(),
                    _ => self.mark_key(key, mods),
                }
                return true;
            }
            let action = self.keymap.lookup(key, mods).or_else(|| latin.and_then(|(k, m)| self.keymap.lookup(k, m)));
            if let Some(action) = action.cloned()
                && self.run_action(action)
            {
                return true;
            }
        }
        // Ctrl+letter sends its control code; Ctrl+Alt is AltGr, which types
        // the layout's character.
        let key = match latin {
            Some((k, m)) if m.ctrl && !m.alt => Some((k, m)),
            _ => key,
        };

        // Escape with a selection clears it (like WT), rather than reaching
        // the shell.
        if key == Some((Key::Escape, Mods::NONE))
            && let Some(term) = self.focused_term()
            && term.lock().selection.take().is_some()
        {
            self.render();
            return true;
        }
        // macOS: ⌘ shortcuts we don't bind belong to the system (⌘Q, ⌘H, …),
        // never to the shell; ⌘M minimizes, as everywhere on a Mac.
        if cfg!(target_os = "macos") && mods.meta {
            if key == Some((Key::Char('m'), Mods { meta: true, ..Mods::NONE })) {
                self.ui().window().set_minimized(true);
                return true;
            }
            return false;
        }
        let Some(pane) = self.focused_pane() else { return false };
        if pane.exited {
            let id = self.active_terminal().map(|(_, f)| f).unwrap_or_default();
            match key {
                Some((Key::Enter, m)) if m == Mods::NONE => self.restart_pane(id),
                Some((Key::Char('d'), m)) if m == Mods::CTRL => self.close_pane(id),
                _ => {}
            }
            return true;
        }
        // cmd's line editor has no "delete word forward": emulate Ctrl+Delete
        // by deleting up to the next word on the line we can see.
        if key == Some((Key::Delete, Mods::CTRL)) && pane.emulate_kill_word {
            let n = chars_to_next_word(&cursor_line(&pane.session));
            let delete = wtpp_input::encode_win32(Key::Delete, Mods::NONE).unwrap_or_default();
            pane.session.write(delete.repeat(n));
            return true;
        }
        let bytes = match key {
            // macOS: Option+arrows move by word, as in Terminal (ESC b / f).
            Some((Key::Left, m)) if cfg!(target_os = "macos") && m == Mods::ALT => Some(b"\x1bb".to_vec()),
            Some((Key::Right, m)) if cfg!(target_os = "macos") && m == Mods::ALT => Some(b"\x1bf".to_vec()),
            // macOS: Option types characters (⌥A → å), as in Terminal.
            Some((Key::Char(c), m)) if cfg!(target_os = "macos") && m.alt && !m.ctrl && !c.is_ascii() => {
                Some(text.as_bytes().to_vec())
            }
            Some((key, mods)) => wtpp_input::encode(key, mods, &pane.session.encode_options()),
            // Composed text (IME, dead keys) goes through verbatim.
            None if text.chars().count() > 1 => Some(text.as_bytes().to_vec()),
            None => None,
        };
        let Some(bytes) = bytes else { return false };
        pane.session.term().lock().selection = None;
        pane.session.write(bytes);
        self.wake_cursor();
        true
    }

    /// Returns false when the action doesn't apply (e.g. copy without a
    /// selection), so the key goes to the shell instead.
    fn run_action(&mut self, action: Action) -> bool {
        match action {
            Action::NewTab(index) => self.new_tab(index.unwrap_or_else(|| self.settings.default_profile_index())),
            Action::DuplicateTab => match self.focused_pane() {
                Some(pane) => self.new_tab_with(pane.profile.clone()),
                None => return false,
            },
            Action::CloseTab => self.close_tab(self.active),
            Action::CloseOtherTabs => {
                let keep = self.active;
                for i in (0..self.tabs.len()).rev() {
                    if i != keep {
                        self.remove_tab(i);
                    }
                }
            }
            Action::ClosePane => match self.active_terminal() {
                Some((_, focused)) => self.close_pane(focused),
                None => self.close_tab(self.active),
            },
            Action::NextTab | Action::PrevTab => {
                let n = self.tabs.len();
                if n > 1 {
                    let next = if action == Action::NextTab { (self.active + 1) % n } else { (self.active + n - 1) % n };
                    self.select_tab(next);
                }
            }
            Action::SwitchToTab(index) => self.select_tab(index),
            Action::SplitPane { side, duplicate } => self.split(side, duplicate),
            Action::MoveFocus(dir) => {
                let Some((root, focused)) = self.active_terminal() else { return false };
                let layout = root.layout(self.content_area(), PANE_GAP);
                if let Some(Tab { kind: TabKind::Terminal { zoomed: zoomed @ true, .. }, .. }) = self.tabs.get_mut(self.active) {
                    // Moving focus out of a zoomed pane un-zooms, like WT.
                    *zoomed = false;
                    self.relayout();
                }
                if let Some(id) = pane_tree::neighbor(&layout, focused, dir) {
                    self.focus_pane(id);
                }
            }
            Action::ResizePane(dir) => {
                if let Some(Tab { kind: TabKind::Terminal { root, focused, .. }, .. }) = self.tabs.get_mut(self.active) {
                    let focused = *focused;
                    if root.resize(focused, dir, 0.05) {
                        self.relayout();
                    }
                }
            }
            Action::SwapPane(dir) => {
                let Some((root, focused)) = self.active_terminal() else { return false };
                let layout = root.layout(self.content_area(), PANE_GAP);
                if let Some(other) = pane_tree::neighbor(&layout, focused, dir)
                    && let Some(Tab { kind: TabKind::Terminal { root, .. }, .. }) = self.tabs.get_mut(self.active)
                {
                    root.swap(focused, other);
                    self.relayout();
                }
            }
            Action::Copy => return self.copy_selection(true),
            Action::Paste => self.paste(),
            Action::ScrollUp => self.scroll(Scroll::Delta(1)),
            Action::ScrollDown => self.scroll(Scroll::Delta(-1)),
            Action::ScrollUpPage => self.scroll(Scroll::PageUp),
            Action::ScrollDownPage => self.scroll(Scroll::PageDown),
            Action::ScrollToTop => self.scroll(Scroll::Top),
            Action::ScrollToBottom => self.scroll(Scroll::Bottom),
            Action::AdjustFontSize(delta) => self.set_font_zoom(self.font_zoom + delta as f32),
            Action::ResetFontSize => self.set_font_zoom(0.0),
            Action::ToggleFullscreen => {
                let ui = self.ui();
                let window = ui.window();
                window.set_fullscreen(!window.is_fullscreen());
            }
            Action::OpenNewTabDropdown => self.ui().invoke_open_new_tab_menu(),
            Action::OpenSettings => self.open_settings(),
            Action::NewWindow => crate::request_window(None, crate::Placement::Default),
            Action::MarkMode => self.toggle_mark_mode(),
            Action::SelectAll => self.select_all(),
            Action::TogglePaneZoom => {
                let Some(Tab { kind: TabKind::Terminal { root, zoomed, .. }, .. }) = self.tabs.get_mut(self.active) else {
                    return false;
                };
                // A single pane has nothing to zoom out of.
                *zoomed = !*zoomed && !matches!(root, Node::Leaf(_));
                self.relayout();
            }
            Action::CommandPalette => self.open_palette(),
            Action::Find => self.open_find(),
            Action::RenameTab => self.tab_menu(self.active, "rename"),
            Action::SendInput(text) => match self.focused_pane() {
                Some(pane) => pane.session.write(text.into_bytes()),
                None => return false,
            },
        }
        true
    }

    fn set_font_zoom(&mut self, zoom: f32) {
        self.font_zoom = zoom.clamp(-8.0, 48.0);
        // Renderers are per size; drop ones no longer used.
        let in_use: Vec<FontKey> = self.panes.values().map(|p| self.font_key(&p.profile)).collect();
        self.renderers.retain(|k, _| in_use.contains(k));
        self.relayout();
    }

    fn scroll(&mut self, scroll: Scroll) {
        if let Some(pane) = self.focused_pane() {
            pane.session.term().lock().scroll_display(scroll);
            self.render();
        }
    }

    // ---- clipboard ----------------------------------------------------------------

    /// Copy the focused pane's selection. `clear` drops the selection after
    /// copying, as an explicit copy does in WT.
    fn copy_selection(&mut self, clear: bool) -> bool {
        let Some(pane) = self.focused_pane() else { return false };
        let text = {
            let mut term = pane.session.term().lock();
            let text = term.selection_to_string().filter(|t| !t.is_empty());
            if text.is_some() && clear {
                term.selection = None;
            }
            text
        };
        let Some(text) = text else { return false };
        if let Some(clipboard) = &mut self.clipboard
            && let Err(e) = clipboard.set_text(text)
        {
            log::warn!("copy failed: {e}");
        }
        self.render();
        true
    }

    fn paste(&mut self) {
        let Some(text) = self.clipboard.as_mut().and_then(|c| c.get_text().ok()) else { return };
        let Some(pane) = self.focused_pane() else { return };
        let bracketed = pane.session.term().lock().mode().contains(TermMode::BRACKETED_PASTE);
        // With bracketed paste the shell won't run pasted lines by itself,
        // so (like WT) the multi-line warning only applies without it.
        let multi_line = !bracketed && text.trim_end_matches(['\r', '\n']).contains(['\r', '\n']);
        let large = text.len() > 5 * 1024;
        let warning = if multi_line && self.settings.multi_line_paste_warning {
            Some("You are about to paste text that contains multiple lines. Each line may run as a command.")
        } else if large && self.settings.large_paste_warning {
            Some("You are about to paste more than 5 KiB of text.")
        } else {
            None
        };
        match warning {
            Some(body) => self.show_dialog(Dialog::Paste(text), "Paste anyway?", body, "Paste"),
            None => pane.session.write(paste_bytes(&text, bracketed)),
        }
    }

    /// Files dropped on the window: paste their paths into the pane under
    /// `at` (window logical coordinates; the focused pane when unknown) and
    /// focus it. Like WT, this skips the paste warnings.
    pub fn on_files_dropped(&mut self, paths: Vec<PathBuf>, at: Option<(f32, f32)>) {
        if paths.is_empty() || self.dialog.is_some() {
            return;
        }
        let ui = self.ui();
        let under = at.and_then(|(x, y)| {
            let (x, y) = (x - ui.get_content_x(), y - ui.get_content_y());
            self.active_layout()
                .into_iter()
                .find(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
                .map(|(id, _)| id)
        });
        let Some(id) = under.or_else(|| self.active_terminal().map(|(_, id)| id)) else { return };
        self.focus_pane(id);
        let Some(pane) = self.panes.get(&id) else { return };
        let (program, _) = split_commandline(&expand_env(&pane.profile.commandline));
        let kind = crate::file_drop::shell_kind(&program, pane.profile.source.as_deref());
        let text = crate::file_drop::dropped_text(&paths, kind);
        let bracketed = pane.session.term().lock().mode().contains(TermMode::BRACKETED_PASTE);
        pane.session.write(paste_bytes(&text, bracketed));
    }

    fn show_dialog(&mut self, dialog: Dialog, title: &str, body: &str, confirm: &str) {
        let ui = self.ui();
        ui.set_dialog_title(title.into());
        ui.set_dialog_body(body.into());
        ui.set_dialog_confirm(confirm.into());
        ui.set_dialog_visible(true);
        self.dialog = Some(dialog);
    }

    pub fn on_dialog_result(&mut self, ok: bool) {
        let ui = self.ui();
        ui.set_dialog_visible(false);
        ui.invoke_focus_keys();
        match (self.dialog.take(), ok) {
            (Some(Dialog::CloseWindow), true) => {
                self.close_confirmed = true;
                crate::close_window(self.id);
            }
            (Some(Dialog::Paste(text)), true) => {
                if let Some(pane) = self.focused_pane() {
                    let bracketed = pane.session.term().lock().mode().contains(TermMode::BRACKETED_PASTE);
                    pane.session.write(paste_bytes(&text, bracketed));
                }
            }
            _ => {}
        }
    }

    // ---- window close ---------------------------------------------------------------

    /// The window's close button: ask first when several tabs are open.
    pub fn on_close_requested(&mut self) -> slint::CloseRequestResponse {
        let terminals = self.tabs.iter().filter(|t| matches!(t.kind, TabKind::Terminal { .. })).count();
        if self.close_confirmed || !self.settings.confirm_close_all_tabs || terminals <= 1 {
            return slint::CloseRequestResponse::HideWindow;
        }
        let body = format!("You're about to close {terminals} tabs.");
        self.show_dialog(Dialog::CloseWindow, "Close all tabs?", &body, "Close all");
        slint::CloseRequestResponse::KeepWindowShown
    }

    // ---- mouse --------------------------------------------------------------------

    fn cell_metrics(&self, pane: &Pane) -> Option<wtpp_render::CellMetrics> {
        self.renderers.get(&self.font_key(&pane.profile)).map(|r| r.cell_metrics())
    }

    /// Map a pane-local logical position (relative to the text area) to a
    /// grid point and cell side.
    fn cell_at(&self, pane: &Pane, x: f32, y: f32) -> Option<(Point, Side)> {
        let cell = self.cell_metrics(pane)?;
        let scale = self.scale();
        let size = pane.session.size();
        let (px, py) = ((x - pane.profile.padding[0]).max(0.0) * scale, (y - pane.profile.padding[1]).max(0.0) * scale);
        let col = ((px / cell.width) as usize).min(size.cols as usize - 1);
        let row = ((py / cell.height) as usize).min(size.rows as usize - 1);
        let side = if px % cell.width < cell.width / 2.0 { Side::Left } else { Side::Right };
        let offset = pane.session.term().lock().grid().display_offset();
        Some((viewport_to_point(offset, Point::new(row, Column(col))), side))
    }

    /// Viewport cell (1-based, as mouse reports use) for a position.
    fn report_cell(&self, pane: &Pane, x: f32, y: f32) -> Option<(usize, usize)> {
        let (point, _) = self.cell_at(pane, x, y)?;
        let offset = pane.session.term().lock().grid().display_offset() as i32;
        Some((point.column.0 + 1, (point.line.0 + offset) as usize + 1))
    }

    pub fn on_pointer(&mut self, index: usize, down: bool, button: u8, x: f32, y: f32, mods: Mods) {
        let Some(&id) = self.visible.get(index) else { return };
        if down {
            self.focus_pane(id);
        }
        let Some(pane) = self.panes.get(&id) else { return };
        let mode = *pane.session.term().lock().mode();

        // Apps that asked for mouse events get them, unless Shift is held
        // (which forces selection, like in WT).
        if mode.intersects(TermMode::MOUSE_MODE) && !mods.shift {
            if let Some((col, row)) = self.report_cell(pane, x, y)
                && let Some(bytes) = mouse_report(mode, button, down, false, col, row, mods)
            {
                pane.session.write(bytes);
            }
            return;
        }

        let Some((point, side)) = self.cell_at(pane, x, y) else { return };
        // Ctrl+click opens links, like WT.
        if (button, down) == (0, true) && mods.ctrl
            && let Some((url, ..)) = link_at(&pane.session, point)
        {
            if let Err(e) = crate::platform::open(&url) {
                log::warn!("couldn't open {url}: {e}");
            }
            return;
        }
        match (button, down) {
            (0, true) => {
                let pane = self.panes.get_mut(&id).unwrap();
                let clicks = match pane.last_click {
                    Some((at, p, n)) if at.elapsed() < DOUBLE_CLICK && p == point => n % 3 + 1,
                    _ => 1,
                };
                pane.last_click = Some((Instant::now(), point, clicks));
                pane.selecting = true;
                let mut term = pane.session.term().lock();
                match (&mut term.selection, mods.shift) {
                    (Some(sel), true) => sel.update(point, side),
                    _ => {
                        let ty = match (clicks, mods.alt) {
                            (_, true) => SelectionType::Block,
                            (2, _) => SelectionType::Semantic,
                            (3, _) => SelectionType::Lines,
                            _ => SelectionType::Simple,
                        };
                        term.selection = Some(Selection::new(ty, point, side));
                    }
                }
            }
            (0, false) => {
                let pane = self.panes.get_mut(&id).unwrap();
                pane.selecting = false;
                let empty = {
                    let mut term = pane.session.term().lock();
                    let empty = term.selection.as_ref().is_none_or(|s| s.is_empty());
                    if empty {
                        term.selection = None;
                    }
                    empty
                };
                if !empty && self.settings.copy_on_select {
                    self.copy_selection(false);
                }
            }
            // Right-click copies the selection, or pastes when there is none
            // (unless the right-click menu is on; it handles the click).
            // (Not a match guard: copying has side effects.)
            #[allow(clippy::collapsible_match)]
            (2, true) if !self.settings.right_click_context_menu => {
                if !self.copy_selection(true) {
                    self.paste();
                }
            }
            _ => {}
        }
        self.render();
    }

    /// An entry of a pane's right-click menu.
    pub fn pane_menu(&mut self, index: usize, action: &str) {
        if let Some(&id) = self.visible.get(index) {
            self.focus_pane(id);
        }
        let action = match action {
            "copy" => Action::Copy,
            "paste" => Action::Paste,
            "select-all" => Action::SelectAll,
            "find" => Action::Find,
            "split" => Action::SplitPane { side: None, duplicate: true },
            "duplicate" => Action::DuplicateTab,
            "close" => Action::ClosePane,
            other => {
                log::warn!("unknown pane menu action {other}");
                return;
            }
        };
        self.run_action(action);
    }

    /// The pointer moved over a pane without a button held.
    pub fn on_pointer_hover(&mut self, index: usize, x: f32, y: f32, ctrl: bool) {
        let Some(&id) = self.visible.get(index) else { return };
        if self.settings.focus_follow_mouse {
            self.focus_pane(id);
        }
        let link = match (ctrl, self.panes.get(&id)) {
            (true, Some(pane)) => self
                .cell_at(pane, x, y)
                .and_then(|(point, _)| link_at(&pane.session, point))
                .map(|(_, start, end)| (id, start, end)),
            _ => None,
        };
        if link != self.hover_link {
            self.hover_link = link;
            self.render();
        }
    }

    pub fn on_pointer_moved(&mut self, index: usize, x: f32, y: f32) {
        let Some(&id) = self.visible.get(index) else { return };
        let Some(pane) = self.panes.get(&id) else { return };
        let mode = *pane.session.term().lock().mode();
        // A Shift+drag selection grows even while the app has mouse mode.
        if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) && !pane.selecting {
            if let Some((col, row)) = self.report_cell(pane, x, y)
                && let Some(bytes) = mouse_report(mode, 0, true, true, col, row, Mods::NONE)
            {
                pane.session.write(bytes);
            }
            return;
        }
        if !pane.selecting {
            return;
        }
        let Some((point, side)) = self.cell_at(pane, x, y) else { return };
        if let Some(sel) = &mut pane.session.term().lock().selection {
            sel.update(point, side);
        }
        self.render();
    }

    /// The scrollbar thumb was dragged: `top` is its position as a fraction
    /// of the track (0 = oldest history, 1 = live output).
    pub fn on_scrollbar(&mut self, index: usize, top: f32) {
        let Some(pane) = self.visible.get(index).and_then(|id| self.panes.get(id)) else { return };
        {
            let mut term = pane.session.term().lock();
            let history = term.grid().history_size() as i32;
            let target = history - (top.clamp(0.0, 1.0) * history as f32).round() as i32;
            let current = term.grid().display_offset() as i32;
            term.scroll_display(Scroll::Delta(target - current));
        }
        self.render();
    }

    /// Cursor blink tick (WT blinks at the system caret rate, ~530 ms).
    pub fn on_blink(&mut self) {
        if self.focused_pane().is_some() {
            self.cursor_on = !self.cursor_on;
            self.render();
        }
    }

    /// Typing shows the cursor immediately, like every editor.
    fn wake_cursor(&mut self) {
        if !self.cursor_on {
            self.cursor_on = true;
            self.render();
        }
    }

    pub fn on_scroll(&mut self, index: usize, x: f32, y: f32, delta_y: f32, ctrl: bool) -> bool {
        if ctrl {
            // Ctrl+wheel zooms, like WT.
            self.set_font_zoom(self.font_zoom + delta_y.signum());
            return true;
        }
        let Some(&id) = self.visible.get(index) else { return false };
        // Where mouse-mode apps are told the wheel turned.
        let cell = self.panes.get(&id).and_then(|p| self.report_cell(p, x, y)).unwrap_or((1, 1));
        let Some(pane) = self.panes.get_mut(&id) else { return false };

        // A wheel notch is 60 logical pixels: 3 lines, like WT, whatever
        // the font size (trackpads scroll proportionally).
        const PIXELS_PER_LINE: f32 = 20.0;
        let lines_f = delta_y / PIXELS_PER_LINE + pane.scroll_remainder;
        let lines = lines_f.trunc() as i32;
        pane.scroll_remainder = lines_f.fract();
        if lines == 0 {
            return true;
        }

        let mode = *pane.session.term().lock().mode();
        if mode.intersects(TermMode::MOUSE_MODE) {
            // One wheel event per notch (3 lines), like xterm and WT; apps
            // scroll several lines per event themselves.
            const LINES_PER_EVENT: i32 = 3;
            pane.wheel_lines += lines;
            let events = pane.wheel_lines / LINES_PER_EVENT;
            pane.wheel_lines %= LINES_PER_EVENT;
            // Wheel up is button 4 (64), down is button 5 (65).
            let button = if events > 0 { 64 } else { 65 };
            for _ in 0..events.unsigned_abs() {
                if let Some(bytes) = mouse_report(mode, button, true, false, cell.0, cell.1, Mods::NONE) {
                    pane.session.write(bytes);
                }
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // Full-screen apps without mouse support (less, man) get arrows.
            let key = if lines > 0 { Key::Up } else { Key::Down };
            let opts = pane.session.encode_options();
            let bytes = wtpp_input::encode(key, Mods::NONE, &opts).unwrap_or_default();
            pane.session.write(bytes.repeat(lines.unsigned_abs() as usize));
        } else {
            pane.session.term().lock().scroll_display(Scroll::Delta(lines));
            self.render();
        }
        true
    }
}

/// How often output is drawn at most (120 Hz).
const FRAME: std::time::Duration = std::time::Duration::from_millis(8);

/// The profile of a tab's focused pane.
fn tab_profile<'a>(app: &'a App, tab: &Tab) -> Option<&'a ResolvedProfile> {
    match &tab.kind {
        TabKind::Terminal { focused, .. } => app.panes.get(focused).map(|p| &p.profile),
        TabKind::Settings => None,
    }
}

/// A tab's color: set from its menu, else the focused pane's profile's.
fn tab_color(app: &App, tab: &Tab) -> Option<wtpp_config::Color> {
    tab.color.or_else(|| match &tab.kind {
        TabKind::Terminal { focused, .. } => app.panes.get(focused)?.profile.tab_color,
        TabKind::Settings => None,
    })
}

/// Renderer colors for a profile's scheme.
pub(crate) fn theme_for(settings: &Settings, profile: &ResolvedProfile) -> Theme {
    let scheme = settings.scheme(&profile.color_scheme);
    let rgb = |c: wtpp_config::Color| ansi::Rgb { r: c.0, g: c.1, b: c.2 };
    Theme {
        foreground: rgb(scheme.foreground),
        background: rgb(scheme.background),
        cursor: rgb(scheme.cursor_color),
        selection_background: rgb(scheme.selection_background),
        ansi: scheme.ansi().map(rgb),
        bold_is_bright: profile.intense_text_style.bright(),
        bold_font: profile.intense_text_style.bold_font(),
    }
}

/// The link under `point` and its extent on the line: an OSC 8 hyperlink,
/// or a URL in the text.
fn link_at(session: &Session, point: Point) -> Option<(String, Point, Point)> {
    let term = session.term().lock();
    let grid = term.grid();
    let row = &grid[point.line];
    let at = |col: usize| Point::new(point.line, Column(col));
    if let Some(link) = grid[point].hyperlink() {
        let same = |c: usize| row[Column(c)].hyperlink().as_ref() == Some(&link);
        let mut start = point.column.0;
        while start > 0 && same(start - 1) {
            start -= 1;
        }
        let mut end = point.column.0;
        while end + 1 < grid.columns() && same(end + 1) {
            end += 1;
        }
        return Some((link.uri().to_string(), at(start), at(end)));
    }
    let line: Vec<char> = (0..grid.columns()).map(|c| row[Column(c)].c).collect();
    let (url, start, end) = url_at(&line, point.column.0)?;
    Some((url, at(start), at(end - 1)))
}

/// Find a URL in `line` that covers column `col`: the URL and its column
/// range (end exclusive).
fn url_at(line: &[char], col: usize) -> Option<(String, usize, usize)> {
    let text: String = line.iter().collect();
    const SCHEMES: [&str; 4] = ["https://", "http://", "file://", "ftp://"];
    let ends_url = |c: char| c.is_whitespace() || "\"'<>`{}|^".contains(c);
    for scheme in SCHEMES {
        let mut search = 0;
        while let Some(found) = text[search..].find(scheme) {
            let start = search + found;
            let end = text[start..].find(ends_url).map_or(text.len(), |e| start + e);
            // Trailing punctuation is almost never part of the link.
            let url = text[start..end].trim_end_matches(['.', ',', ')', ';', ':', '!', '?', ']']);
            let (start_col, end_col) = (text[..start].chars().count(), text[..start].chars().count() + url.chars().count());
            if (start_col..end_col).contains(&col) && url.len() > scheme.len() {
                return Some((url.to_string(), start_col, end_col));
            }
            search = end.max(start + 1);
        }
    }
    None
}

fn is_cmd(program: &str) -> bool {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    file == "cmd" || file == "cmd.exe"
}

/// Text from the cursor to the end of its line, following soft wraps.
fn cursor_line(session: &Session) -> Vec<char> {
    use wtpp_session::alacritty_terminal::index::Line;
    use wtpp_session::alacritty_terminal::term::cell::Flags;

    let term = session.term().lock();
    let grid = term.grid();
    let cursor = grid.cursor.point;
    let mut out = Vec::new();
    let mut line = cursor.line;
    let mut col = cursor.column.0;
    while line.0 < grid.screen_lines() as i32 {
        let row = &grid[line];
        for c in col..grid.columns() {
            let cell = &row[Column(c)];
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                out.push(cell.c);
            }
        }
        if !row[Column(grid.columns() - 1)].flags.contains(Flags::WRAPLINE) {
            break;
        }
        line = Line(line.0 + 1);
        col = 0;
    }
    out
}

/// How many characters Ctrl+Delete removes, Windows-style: the rest of the
/// current word plus the whitespace after it (or, from whitespace, up to the
/// next word). Trailing blanks at the end of the input stop the count.
fn chars_to_next_word(line: &[char]) -> usize {
    let end = line.iter().rposition(|c| !c.is_whitespace()).map_or(0, |i| i + 1);
    let line = &line[..end];
    let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
    let mut i = 0;
    if line.first().is_some_and(is_word) {
        while line.get(i).is_some_and(is_word) {
            i += 1;
        }
    } else if line.first().is_some_and(|c| !c.is_whitespace()) {
        // Punctuation run.
        while line.get(i).is_some_and(|c| !is_word(c) && !c.is_whitespace()) {
            i += 1;
        }
    }
    while line.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    i
}

fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn is_exe_path(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    lower.ends_with(".exe") && (lower.contains('\\') || lower.contains('/'))
}

/// Bytes to write for pasted text: line endings become CR (what Enter
/// sends), and bracketed paste wraps the text so shells don't run it.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // Strip ESC so pasted text can't terminate the bracket early.
        let inner = normalized.replace('\x1b', "");
        format!("\x1b[200~{inner}\x1b[201~").into_bytes()
    } else {
        normalized.into_bytes()
    }
}

/// Encode a mouse event for an app that enabled mouse reporting.
/// `button`: 0 left, 1 middle, 2 right, 64/65 wheel. `col`/`row` are 1-based.
pub fn mouse_report(mode: TermMode, button: u8, down: bool, motion: bool, col: usize, row: usize, mods: Mods) -> Option<Vec<u8>> {
    if button > 2 && button < 64 {
        return None;
    }
    let mut code = button as u32;
    if motion {
        code += 32;
    }
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.ctrl {
        code += 16;
    }
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if down { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{col};{row}{end}").into_bytes());
    }
    // Legacy X10 encoding: release is button 3, coordinates offset by 32.
    if col > 223 || row > 223 {
        return None;
    }
    let code = if down || button >= 64 { code } else { 3 + (code & !3) };
    Some(vec![0x1b, b'[', b'M', 32 + code as u8, 32 + col as u8, 32 + row as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_path_titles() {
        assert!(is_exe_path(r"C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe"));
        assert!(is_exe_path(r"C:\WINDOWS\system32\cmd.exe"));
        assert!(!is_exe_path("mark@host: ~/wtpp"));
        assert!(!is_exe_path("setup.exe"));
    }

    #[test]
    fn kill_word_counts() {
        let count = |s: &str| chars_to_next_word(&s.chars().collect::<Vec<_>>());
        assert_eq!(count("two three   "), 4); // "two "
        assert_eq!(count("wo three"), 3); // mid-word: rest of word + space
        assert_eq!(count("  three"), 2); // whitespace up to the next word
        assert_eq!(count("three      "), 5); // last word: not the blanks after it
        assert_eq!(count("--force x"), 2); // punctuation is its own word, as in Windows text boxes
        assert_eq!(count("     "), 0);
        assert!(is_cmd(r"C:\WINDOWS\system32\cmd.exe"));
        assert!(!is_cmd("powershell.exe"));
    }

    #[test]
    fn urls_in_text() {
        let line: Vec<char> = "see https://example.com/a?b=1, or (http://x.org).".chars().collect();
        assert_eq!(url_at(&line, 10), Some(("https://example.com/a?b=1".into(), 4, 29)));
        assert_eq!(url_at(&line, 40).map(|u| u.0).as_deref(), Some("http://x.org"));
        assert_eq!(url_at(&line, 2), None);
    }

    #[test]
    fn paste_normalizes_newlines() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc");
        assert_eq!(paste_bytes("ls\n", true), b"\x1b[200~ls\r\x1b[201~");
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~x[201~y\x1b[201~");
    }

    #[test]
    fn mouse_reports() {
        let sgr = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(mouse_report(sgr, 0, true, false, 5, 3, Mods::NONE).unwrap(), b"\x1b[<0;5;3M");
        assert_eq!(mouse_report(sgr, 0, false, false, 5, 3, Mods::NONE).unwrap(), b"\x1b[<0;5;3m");
        assert_eq!(mouse_report(sgr, 64, true, false, 1, 1, Mods::NONE).unwrap(), b"\x1b[<64;1;1M");
        assert_eq!(mouse_report(sgr, 0, true, true, 2, 2, Mods::NONE).unwrap(), b"\x1b[<32;2;2M");

        let x10 = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(mouse_report(x10, 0, true, false, 1, 1, Mods::NONE).unwrap(), b"\x1b[M !!");
        assert_eq!(mouse_report(x10, 0, false, false, 1, 1, Mods::NONE).unwrap(), b"\x1b[M#!!");
        assert_eq!(mouse_report(x10, 0, true, false, 300, 1, Mods::NONE), None);
    }
}
