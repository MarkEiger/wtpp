//! The Settings tab: an editable copy of the settings ("draft") bound to the
//! `SettingsData` global, saved to `settings.json` on demand.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use wtpp_config::actions::{escape_input, unescape_input};
use wtpp_config::schemes::builtin_schemes;
use wtpp_config::settings::{
    DiscoveredProfile, FALLBACK_SOURCE, FontWeight, Padding, ProfileSettings, RefreshReport,
    find_windows_terminal_settings, guid_for,
};
use wtpp_config::{Action, AppTheme, TabWidthMode, BellStyle, GraphicsApi, Binding, COLOR_NAMES, Chord, CloseOnExit, CursorShape, IntenseTextStyle, KeyEncoding, Keymap, LaunchMode, Settings};
use wtpp_input::{Key, Mods};

use crate::app::{App, Tab, TabKind};
use crate::{BindingRow, ColorRow, MainWindow, SettingsData};

/// Order of the bell and intense-text dropdowns in the UI.
const BELL_STYLES: [BellStyle; 4] = [BellStyle::Audible, BellStyle::Visual, BellStyle::All, BellStyle::None];
const INTENSE_STYLES: [IntenseTextStyle; 4] =
    [IntenseTextStyle::Bright, IntenseTextStyle::Bold, IntenseTextStyle::All, IntenseTextStyle::None];

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Pages before the per-profile ones.
const PAGE_SHORTCUTS: usize = 5;
const PAGE_DEFAULTS: usize = 6;
const NAV_FIXED: [&str; 7] = ["Startup", "Interaction", "Appearance", "Color schemes", "Rendering", "Keyboard shortcuts", "Defaults"];
const THEMES: [AppTheme; 3] = [AppTheme::Dark, AppTheme::Light, AppTheme::System];
const TAB_WIDTHS: [TabWidthMode; 3] = [TabWidthMode::Equal, TabWidthMode::TitleLength, TabWidthMode::Compact];
const GRAPHICS_APIS: [GraphicsApi; 6] = [
    GraphicsApi::Automatic,
    GraphicsApi::Direct3d12,
    GraphicsApi::Vulkan,
    GraphicsApi::Metal,
    GraphicsApi::OpenGl,
    GraphicsApi::Software,
];

pub(crate) struct Draft {
    settings: Settings,
    keymap: Keymap,
    page: usize,
    /// Row (in `rows`) whose keys are being recorded.
    pub(crate) recording: Option<usize>,
    rows: Vec<Row>,
    /// A newly added action waiting for its keys.
    pending: Option<Action>,
    filter: String,
    scheme: usize,
    catalog: Vec<Action>,
    dirty: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Binding(usize),
    Pending,
}

impl Draft {
    fn new(settings: &Settings) -> Draft {
        let (keymap, _) = settings.keymap();
        Draft {
            catalog: Action::catalog(settings.visible_profiles().len()),
            settings: settings.clone(),
            keymap,
            page: 0,
            recording: None,
            rows: Vec::new(),
            pending: None,
            filter: String::new(),
            scheme: 0,
            dirty: false,
        }
    }

    fn profile_names(&self) -> Vec<String> {
        self.settings.visible_profiles().into_iter().map(|p| p.name).collect()
    }

    /// The profile entry the current page edits (`None` for "Defaults").
    fn profile_index(&self) -> Option<usize> {
        self.page.checked_sub(PAGE_DEFAULTS + 1)
    }

    fn profile_mut(&mut self) -> &mut ProfileSettings {
        match self.profile_index() {
            Some(i) if i < self.settings.profiles.list.len() => &mut self.settings.profiles.list[i],
            _ => &mut self.settings.profiles.defaults,
        }
    }

    /// Update the list of shells, leaving every other edit alone. Profile
    /// pages go by position: stay on the same profile if it's still there.
    fn refresh_shells(&mut self, found: &[DiscoveredProfile], wsl_known: bool) -> RefreshReport {
        let shown = self.profile_index().and_then(|i| self.settings.profiles.list.get(i)).map(|p| (p.guid.clone(), p.name.clone()));
        let report = self.settings.refresh_discovered(found, wsl_known);
        self.settings.profiles_discovered = true;
        if let Some((guid, name)) = shown {
            let index = self.settings.profiles.list.iter().position(|p| p.guid == guid && p.name == name);
            self.page = index.map_or(PAGE_DEFAULTS, |i| PAGE_DEFAULTS + 1 + i);
        }
        self.catalog = Action::catalog(self.settings.visible_profiles().len());
        report
    }

    fn rebuild_rows(&mut self, names: &[String]) {
        let filter = self.filter.to_lowercase();
        let mut rows: Vec<(String, Row)> = self
            .keymap
            .bindings
            .iter()
            .enumerate()
            .map(|(i, b)| (b.action.label(names), Row::Binding(i)))
            .filter(|(label, row)| {
                let Row::Binding(i) = row else { return true };
                filter.is_empty()
                    || label.to_lowercase().contains(&filter)
                    || self.keymap.bindings[*i].chord.label().to_lowercase().contains(&filter)
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        self.rows = self.pending.iter().map(|_| Row::Pending).chain(rows.into_iter().map(|(_, r)| r)).collect();
    }

    fn row_action(&self, row: usize) -> Option<Action> {
        match self.rows.get(row)? {
            Row::Binding(i) => Some(self.keymap.bindings[*i].action.clone()),
            Row::Pending => self.pending.clone(),
        }
    }
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn strings(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    model(items.into_iter().map(SharedString::from).collect())
}

impl App {
    /// Open Settings on the keyboard shortcuts page.
    pub fn open_shortcuts(&mut self) {
        self.open_settings();
        if let Some(d) = self.draft.as_mut() {
            d.page = PAGE_SHORTCUTS;
        }
        self.refresh_settings();
    }

    pub fn open_settings(&mut self) {
        if let Some(i) = self.tabs.iter().position(|t| matches!(t.kind, TabKind::Settings)) {
            self.select_tab(i);
            return;
        }
        self.draft = Some(Draft::new(&self.settings));
        self.tabs.push(Tab::new(TabKind::Settings));
        self.active = self.tabs.len() - 1;
        self.set_status("", false);
        if let (_, Some(error)) = Settings::load(&self.settings_path) {
            self.set_status(
                &format!(
                    "settings.json has an error and isn't being used: {error}. Fix it with Open JSON file; \
                     if you save here instead, the old file is kept as settings.json.bak."
                ),
                true,
            );
        }
        self.refresh_settings();
        self.relayout();
    }

    fn set_status(&self, text: &str, error: bool) {
        let ui = self.ui();
        let data = ui.global::<SettingsData>();
        data.set_status(text.into());
        data.set_status_is_error(error);
    }

    /// Push the whole draft into the UI.
    pub(crate) fn refresh_settings(&mut self) {
        let is_windows = cfg!(windows);
        let can_import = find_windows_terminal_settings().is_some();
        let fonts = self.font_families.clone();
        let ui = self.ui();
        let data = ui.global::<SettingsData>();
        let Some(d) = self.draft.as_mut() else { return };
        // A snapshot: `d` is updated below (page, rows) while reading this.
        let s = &d.settings.clone();
        let names = d.profile_names();

        let nav: Vec<String> = NAV_FIXED
            .iter()
            .map(|s| s.to_string())
            .chain(s.profiles.list.iter().map(|p| {
                let name = p.name.clone().unwrap_or_else(|| "Profile".into());
                if p.hidden == Some(true) { format!("{name} (hidden)") } else { name }
            }))
            .collect();
        if d.page >= nav.len() {
            d.page = PAGE_DEFAULTS;
        }
        data.set_nav_items(strings(nav));
        data.set_page(d.page as i32);
        data.set_dirty(d.dirty);
        data.set_is_windows(is_windows);
        data.set_titlebar_tabs(true);
        data.set_new_tab_keys(self.keymap.chord_label(&Action::NewTab(None)).unwrap_or_default().into());

        // Startup / interaction / appearance
        data.set_profile_names(strings(names.clone()));
        data.set_default_profile(s.default_profile_index() as i32);
        data.set_launch_mode(match s.launch_mode {
            LaunchMode::Default => 0,
            LaunchMode::Maximized => 1,
            LaunchMode::Fullscreen => 2,
        });
        data.set_initial_cols(s.initial_cols as i32);
        data.set_initial_rows(s.initial_rows as i32);
        data.set_can_import_wt(can_import);
        data.set_shells_refreshing(crate::shell_refresh_running());
        refresh_default_terminal(&ui);
        data.set_copy_on_select(s.copy_on_select);
        data.set_word_delimiters(s.word_delimiters.clone().into());
        data.set_confirm_close(s.confirm_close_all_tabs);
        data.set_multi_line_paste_warning(s.multi_line_paste_warning);
        data.set_focus_follow_mouse(s.focus_follow_mouse);
        data.set_inherit_cwd(s.inherit_working_directory);
        data.set_split_ssh(s.split_duplicates_ssh);
        data.set_allow_osc52(s.allow_osc52);
        data.set_reload_env(s.reload_environment_variables);
        data.set_remember_bounds(s.remember_window_bounds);
        data.set_large_paste_warning(s.large_paste_warning);
        data.set_tabs_in_titlebar(s.show_tabs_in_titlebar);
        data.set_theme(THEMES.iter().position(|t| *t == s.theme).unwrap_or(0) as i32);
        data.set_tab_width_mode(TAB_WIDTHS.iter().position(|t| *t == s.tab_width_mode).unwrap_or(0) as i32);
        data.set_always_show_tabs(s.always_show_tabs);
        data.set_right_click_menu(s.right_click_context_menu);
        data.set_graphics_api(GRAPHICS_APIS.iter().position(|g| *g == s.graphics_api).unwrap_or(0) as i32);

        // Color schemes
        let schemes = s.all_schemes();
        d.scheme = d.scheme.min(schemes.len().saturating_sub(1));
        let scheme = &schemes[d.scheme];
        let builtin = builtin_schemes().iter().any(|b| b.name == scheme.name);
        data.set_scheme_names(strings(schemes.iter().map(|s| s.name.clone())));
        data.set_scheme_index(d.scheme as i32);
        data.set_scheme_builtin(builtin);
        data.set_scheme_name(scheme.name.clone().into());
        data.set_scheme_colors(model(
            scheme
                .colors()
                .iter()
                .zip(COLOR_NAMES)
                .map(|(c, name)| ColorRow {
                    name: name.into(),
                    hex: c.to_string().into(),
                    color: slint::Color::from_rgb_u8(c.0, c.1, c.2),
                })
                .collect(),
        ));

        // Actions
        d.rebuild_rows(&names);
        let rows: Vec<BindingRow> = d
            .rows
            .iter()
            .map(|row| {
                let (action, keys) = match row {
                    Row::Binding(i) => {
                        let b = &d.keymap.bindings[*i];
                        (b.action.clone(), b.chord.label())
                    }
                    Row::Pending => (d.pending.clone().unwrap(), String::new()),
                };
                let input = match &action {
                    Action::SendInput(text) => Some(escape_input(text)),
                    _ => None,
                };
                BindingRow {
                    action: action.label(&names).into(),
                    keys: keys.into(),
                    has_input: input.is_some(),
                    input: input.unwrap_or_default().into(),
                }
            })
            .collect();
        data.set_bindings(model(rows));
        data.set_action_catalog(strings(d.catalog.iter().map(|a| a.label(&names))));
        data.set_recording(d.recording.map_or(-1, |r| r as i32));

        // Profile page
        let is_defaults = d.profile_index().is_none();
        let entry = match d.profile_index() {
            Some(i) => s.profiles.list.get(i).cloned().unwrap_or_default(),
            None => s.profiles.defaults.clone(),
        };
        let resolved = s.resolve(&entry);
        data.set_p_is_defaults(is_defaults);
        data.set_p_name(resolved.name.clone().into());
        data.set_p_commandline(if is_defaults { entry.commandline.clone().unwrap_or_default() } else { resolved.commandline.clone() }.into());
        data.set_p_startdir(resolved.starting_directory.clone().unwrap_or_default().into());
        data.set_p_hidden(resolved.hidden);
        data.set_p_scheme(schemes.iter().position(|x| x.name == resolved.color_scheme).unwrap_or(0) as i32);
        let mut families = fonts;
        if !families.iter().any(|f| f.eq_ignore_ascii_case(&resolved.font_face)) {
            families.insert(0, resolved.font_face.clone());
        }
        data.set_p_font(families.iter().position(|f| f.eq_ignore_ascii_case(&resolved.font_face)).unwrap_or(0) as i32);
        data.set_font_families(strings(families));
        data.set_p_font_size(resolved.font_size.round() as i32);
        data.set_cursor_shapes(strings(CursorShape::ALL.iter().map(|c| c.label().to_string())));
        data.set_p_cursor(CursorShape::ALL.iter().position(|c| *c == resolved.cursor_shape).unwrap_or(0) as i32);
        data.set_p_padding(Padding(resolved.padding).to_string().into());
        data.set_p_history(resolved.history_size as i32);
        data.set_p_tab_title(resolved.tab_title.clone().unwrap_or_default().into());
        data.set_p_icon(entry.icon.clone().unwrap_or_default().into());
        data.set_p_suppress_title(resolved.suppress_application_title);
        data.set_p_environment(
            entry.environment.iter().flatten().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ").into(),
        );
        data.set_p_bell(BELL_STYLES.iter().position(|b| *b == resolved.bell_style).unwrap_or(0) as i32);
        data.set_p_intense(INTENSE_STYLES.iter().position(|b| *b == resolved.intense_text_style).unwrap_or(0) as i32);
        data.set_font_weights(strings(FontWeight::NAMES.iter().map(|(n, w)| format!("{} ({w})", capitalize(n)))));
        data.set_p_weight(FontWeight::NAMES.iter().position(|(_, w)| *w == resolved.font_weight).unwrap_or(4) as i32);
        data.set_p_cell_height(format!("{}", resolved.cell_height).into());
        data.set_p_ligatures(resolved.ligatures);
        data.set_p_close_on_exit(match resolved.close_on_exit {
            CloseOnExit::Graceful => 0,
            CloseOnExit::Always => 1,
            CloseOnExit::Never => 2,
        });
        data.set_p_encoding(match resolved.key_encoding {
            KeyEncoding::Auto => 0,
            KeyEncoding::Unix => 1,
            KeyEncoding::Windows => 2,
        });
    }

    fn edit(&mut self, f: impl FnOnce(&mut Draft)) {
        if let Some(d) = self.draft.as_mut() {
            f(d);
            d.dirty = true;
        }
        self.refresh_settings();
    }

    pub fn settings_set_value(&mut self, key: &str, value: &str) {
        let Some(d) = self.draft.as_mut() else { return };
        let int = value.parse::<i64>().unwrap_or(0);
        let flag = value == "true";
        match key {
            // Navigation and filtering aren't edits.
            "page" => {
                d.page = int.max(0) as usize;
                d.recording = None;
                self.refresh_settings();
                return;
            }
            "bindingFilter" => {
                d.filter = value.to_string();
                d.recording = None;
                self.refresh_settings();
                return;
            }
            _ => {}
        }
        let font_families = self.font_families.clone();
        self.edit(|d| {
            let s = &mut d.settings;
            match key {
                "defaultProfile" => {
                    // The stand-in shell isn't in the file, so nothing can refer to it.
                    let chosen = s.visible_profiles().into_iter().nth(int as usize);
                    s.default_profile = chosen.filter(|p| p.source.as_deref() != Some(FALLBACK_SOURCE)).map(|p| p.guid);
                }
                "launchMode" => {
                    s.launch_mode = [LaunchMode::Default, LaunchMode::Maximized, LaunchMode::Fullscreen][int.clamp(0, 2) as usize]
                }
                "initialCols" => s.initial_cols = int.clamp(10, 999) as u16,
                "initialRows" => s.initial_rows = int.clamp(5, 999) as u16,
                "copyOnSelect" => s.copy_on_select = flag,
                "wordDelimiters" => s.word_delimiters = value.to_string(),
                "confirmCloseAllTabs" => s.confirm_close_all_tabs = flag,
                "multiLinePasteWarning" => s.multi_line_paste_warning = flag,
                "focusFollowMouse" => s.focus_follow_mouse = flag,
                "inheritWorkingDirectory" => s.inherit_working_directory = flag,
                "splitDuplicatesSsh" => s.split_duplicates_ssh = flag,
                "allowOSC52" => s.allow_osc52 = flag,
                "reloadEnvironmentVariables" => s.reload_environment_variables = flag,
                "rememberWindowBounds" => s.remember_window_bounds = flag,
                "largePasteWarning" => s.large_paste_warning = flag,
                "showTabsInTitlebar" => s.show_tabs_in_titlebar = flag,
                "theme" => s.theme = THEMES[int.clamp(0, 2) as usize],
                "tabWidthMode" => s.tab_width_mode = TAB_WIDTHS[int.clamp(0, 2) as usize],
                "alwaysShowTabs" => s.always_show_tabs = flag,
                "rightClickContextMenu" => s.right_click_context_menu = flag,
                "graphicsAPI" => s.graphics_api = GRAPHICS_APIS[int.clamp(0, 5) as usize],
                "schemeName" => {
                    let all = s.all_schemes();
                    let Some(old) = all.get(d.scheme).map(|x| x.name.clone()) else { return };
                    let new = value.trim();
                    if new.is_empty() || all.iter().any(|x| x.name == new) {
                        return;
                    }
                    if let Some(scheme) = s.schemes.iter_mut().find(|x| x.name == old) {
                        scheme.name = new.to_string();
                    }
                    for p in std::iter::once(&mut s.profiles.defaults).chain(s.profiles.list.iter_mut()) {
                        if p.color_scheme.as_deref() == Some(&old) {
                            p.color_scheme = Some(new.to_string());
                        }
                    }
                }
                _ => {
                    let schemes: Vec<String> = s.all_schemes().into_iter().map(|x| x.name).collect();
                    let p = d.profile_mut();
                    let text = || (!value.trim().is_empty()).then(|| value.to_string());
                    match key {
                        "p.name" => p.name = text(),
                        "p.commandline" => p.commandline = text(),
                        "p.startingDirectory" => p.starting_directory = text(),
                        "p.hidden" => p.hidden = Some(flag),
                        "p.colorScheme" => p.color_scheme = schemes.get(int as usize).cloned(),
                        "p.fontFace" => {
                            // The list may have the profile's missing font prepended.
                            let mut list = font_families;
                            let current = p.font_face().map(str::to_string);
                            if let Some(c) = current.filter(|c| !list.iter().any(|f| f.eq_ignore_ascii_case(c))) {
                                list.insert(0, c);
                            }
                            p.set_font_face(list.get(int as usize).cloned());
                        }
                        "p.fontSize" => p.set_font_size(Some(int.clamp(6, 72) as f32)),
                        "p.cursorShape" => p.cursor_shape = CursorShape::ALL.get(int as usize).copied(),
                        "p.padding" => {
                            if let Some(padding) = Padding::parse(value) {
                                p.padding = Some(padding);
                            }
                        }
                        "p.historySize" => p.history_size = Some(int.max(0) as u32),
                        "p.tabTitle" => p.tab_title = text(),
                        "p.icon" => p.icon = text(),
                        "p.suppressApplicationTitle" => p.suppress_application_title = Some(flag),
                        "p.environment" => {
                            let vars: std::collections::BTreeMap<String, String> = value
                                .split(';')
                                .filter_map(|pair| {
                                    let (k, v) = pair.split_once('=')?;
                                    let k = k.trim();
                                    (!k.is_empty()).then(|| (k.to_string(), v.trim().to_string()))
                                })
                                .collect();
                            p.environment = (!vars.is_empty()).then_some(vars);
                        }
                        "p.bellStyle" => p.bell_style = BELL_STYLES.get(int as usize).copied(),
                        "p.intenseTextStyle" => p.intense_text_style = INTENSE_STYLES.get(int as usize).copied(),
                        "p.fontWeight" => {
                            p.font.get_or_insert_with(Default::default).weight =
                                FontWeight::NAMES.get(int as usize).map(|(_, w)| FontWeight(*w));
                        }
                        "p.ligatures" => p.font.get_or_insert_with(Default::default).set_ligatures(flag),
                        "p.cellHeight" => {
                            if let Ok(h) = value.trim().parse::<f32>()
                                && (0.8..=3.0).contains(&h)
                            {
                                p.font.get_or_insert_with(Default::default).cell_height = Some(h);
                            }
                        }
                        "p.closeOnExit" => {
                            p.close_on_exit = Some([CloseOnExit::Graceful, CloseOnExit::Always, CloseOnExit::Never][int.clamp(0, 2) as usize])
                        }
                        "p.keyEncoding" => {
                            p.key_encoding = Some([KeyEncoding::Auto, KeyEncoding::Unix, KeyEncoding::Windows][int.clamp(0, 2) as usize])
                        }
                        other => log::warn!("unknown setting {other}"),
                    }
                }
            }
        });
    }

    pub fn settings_save(&mut self) {
        let Some(d) = self.draft.as_mut() else { return };
        d.settings.set_keymap(&d.keymap);
        let settings = d.settings.clone();
        // This replaces the file; if it doesn't parse, it's the only copy of
        // whatever the user had in it, so it's kept, or nothing is saved.
        let backup = match Settings::backup_if_broken(&self.settings_path) {
            Ok(backup) => backup,
            Err(e) => {
                let path = self.settings_path.display();
                self.set_status(&format!("{path} has an error and couldn't be copied, so nothing was saved: {e}"), true);
                return;
            }
        };
        match settings.save(&self.settings_path) {
            Ok(()) => {
                d.dirty = false;
                self.apply_settings(settings);
                let path = self.settings_path.display();
                let status = match backup {
                    Some(backup) => format!("Saved to {path}. The old file had an error; it's kept as {}.", backup.display()),
                    None => format!("Saved to {path}"),
                };
                self.set_status(&status, false);
            }
            Err(e) => self.set_status(&format!("Couldn't save {}: {e}", self.settings_path.display()), true),
        }
        self.refresh_settings();
    }

    pub fn settings_discard(&mut self) {
        let page = self.draft.as_ref().map_or(0, |d| d.page);
        let mut draft = Draft::new(&self.settings);
        draft.page = page;
        self.draft = Some(draft);
        self.set_status("Changes discarded.", false);
        self.refresh_settings();
    }

    /// The settings file changed on disk (edited by hand, or another window).
    pub fn on_settings_file_changed(&mut self) {
        let (settings, error) = Settings::load(&self.settings_path);
        // Logged once, by the watcher, not here for each window.
        if let Some(error) = error {
            if self.draft.is_some() {
                self.set_status(&format!("settings.json has an error: {error}"), true);
            }
            return;
        }
        if settings == self.settings {
            return;
        }
        log::info!("settings.json changed, reloading");
        self.apply_settings(settings);
        // Follow the file unless the user has unsaved edits in the UI.
        if let Some(d) = &self.draft
            && !d.dirty
        {
            let page = d.page;
            let mut draft = Draft::new(&self.settings);
            draft.page = page;
            self.draft = Some(draft);
            self.set_status("Reloaded settings.json.", false);
            self.refresh_settings();
        }
    }

    pub fn settings_open_json(&mut self) {
        if !self.settings_path.exists()
            && let Err(e) = self.settings.save(&self.settings_path)
        {
            self.set_status(&format!("Couldn't create {}: {e}", self.settings_path.display()), true);
            return;
        }
        if let Err(e) = crate::platform::open(&self.settings_path.to_string_lossy()) {
            self.set_status(&format!("Couldn't open {}: {e}", self.settings_path.display()), true);
        }
    }

    /// The "Default terminal application" button: takes effect at once (it's
    /// a system setting, not part of settings.json).
    pub fn settings_set_default_term(&mut self, make_default: bool) {
        let result = if make_default { crate::default_terminal::make_default() } else { crate::default_terminal::restore() };
        match result {
            Ok(note) => self.set_status(&note, false),
            Err(e) => self.set_status(&format!("Couldn't change the default terminal: {e}"), true),
        }
        refresh_default_terminal(&self.ui());
    }

    pub fn settings_import_wt(&mut self) {
        let Some(path) = find_windows_terminal_settings() else { return };
        let (wt, error) = Settings::load(&path);
        if let Some(error) = error {
            self.set_status(&format!("Couldn't read Windows Terminal settings: {error}"), true);
            return;
        }
        let profiles = wt.profiles.list.len();
        let schemes = wt.schemes.len();
        self.edit(|d| {
            d.settings.import_windows_terminal(&wt);
            d.keymap = d.settings.keymap().0;
            d.catalog = Action::catalog(d.settings.visible_profiles().len());
        });
        self.set_status(
            &format!("Imported {profiles} profiles and {schemes} color schemes from Windows Terminal. Review, then Save."),
            false,
        );
    }

    /// The list of shells changed (`settings` has it): show it everywhere,
    /// and put it in the draft too. Returns what that changed in the draft,
    /// where unsaved edits may make it differ from the file.
    pub(crate) fn on_shells_changed(&mut self, settings: Settings, found: &[DiscoveredProfile], wsl_known: bool) -> Option<RefreshReport> {
        self.apply_settings(settings);
        let report = self.draft.as_mut().map(|d| d.refresh_shells(found, wsl_known));
        self.refresh_settings();
        report
    }

    /// The "Refresh shell list" button's result.
    pub(crate) fn settings_shells_refreshed(&mut self, report: &RefreshReport, not_saved: Option<&str>) {
        let summary = report.summary();
        match not_saved {
            Some(why) => self.set_status(&format!("{summary} {why}"), true),
            None => self.set_status(&summary, false),
        }
    }

    // ---- color schemes ---------------------------------------------------------

    pub fn settings_select_scheme(&mut self, index: usize) {
        if let Some(d) = self.draft.as_mut() {
            d.scheme = index;
        }
        self.refresh_settings();
    }

    pub fn settings_set_scheme_color(&mut self, index: usize, hex: &str) {
        let Some(color) = wtpp_config::Color::parse(hex) else { return };
        self.edit(|d| {
            let Some(name) = d.settings.all_schemes().get(d.scheme).map(|s| s.name.clone()) else { return };
            if let Some(scheme) = d.settings.schemes.iter_mut().find(|s| s.name == name) {
                scheme.set_color(index, color);
            }
        });
    }

    pub fn settings_new_scheme(&mut self) {
        self.edit(|d| {
            let all = d.settings.all_schemes();
            let Some(base) = all.get(d.scheme) else { return };
            let mut copy = base.clone();
            let mut n = 1;
            copy.name = format!("{} (copy)", base.name);
            while all.iter().any(|s| s.name == copy.name) {
                n += 1;
                copy.name = format!("{} (copy {n})", base.name);
            }
            d.settings.schemes.push(copy);
            d.scheme = d.settings.all_schemes().len() - 1;
        });
    }

    pub fn settings_delete_scheme(&mut self) {
        self.edit(|d| {
            let Some(name) = d.settings.all_schemes().get(d.scheme).map(|s| s.name.clone()) else { return };
            d.settings.schemes.retain(|s| s.name != name);
            for p in std::iter::once(&mut d.settings.profiles.defaults).chain(d.settings.profiles.list.iter_mut()) {
                if p.color_scheme.as_deref() == Some(&name) {
                    p.color_scheme = None;
                }
            }
            d.scheme = 0;
        });
    }

    // ---- profiles ------------------------------------------------------------------

    pub fn settings_add_profile(&mut self) {
        self.edit(|d| {
            let list = &mut d.settings.profiles.list;
            let mut name = "New profile".to_string();
            let mut n = 1;
            while list.iter().any(|p| p.name.as_deref() == Some(&name)) {
                n += 1;
                name = format!("New profile {n}");
            }
            let commandline = if cfg!(windows) { "cmd.exe".to_string() } else { "/bin/bash -l".to_string() };
            list.push(ProfileSettings {
                guid: Some(guid_for(&name)),
                name: Some(name),
                commandline: Some(commandline),
                ..Default::default()
            });
            d.page = PAGE_DEFAULTS + list.len();
        });
    }

    pub fn settings_delete_profile(&mut self) {
        self.edit(|d| {
            if let Some(i) = d.profile_index()
                && i < d.settings.profiles.list.len()
            {
                d.settings.profiles.list.remove(i);
                d.page = PAGE_DEFAULTS;
            }
        });
    }

    // ---- key bindings ------------------------------------------------------------------

    pub fn settings_record_binding(&mut self, row: usize) {
        let Some(d) = self.draft.as_mut() else { return };
        d.recording = Some(row);
        let ui = self.ui();
        ui.global::<SettingsData>().set_recording(row as i32);
        ui.global::<SettingsData>().set_binding_message("".into());
        // Keys must reach our handler, not a text field.
        ui.invoke_focus_keys();
    }

    pub fn settings_add_binding(&mut self) {
        let ui = self.ui();
        let index = ui.global::<SettingsData>().get_catalog_index().max(0) as usize;
        let Some(d) = self.draft.as_mut() else { return };
        let Some(action) = d.catalog.get(index).cloned() else { return };
        d.pending = Some(action);
        d.filter.clear();
        ui.global::<SettingsData>().set_binding_filter("".into());
        self.refresh_settings();
        self.settings_record_binding(0);
    }

    pub fn settings_delete_binding(&mut self, row: usize) {
        self.edit(|d| {
            match d.rows.get(row) {
                Some(Row::Binding(i)) => {
                    d.keymap.bindings.remove(*i);
                }
                Some(Row::Pending) => d.pending = None,
                None => {}
            }
            d.recording = None;
        });
    }

    pub fn settings_reset_bindings(&mut self) {
        self.edit(|d| {
            d.keymap = Keymap::defaults();
            d.pending = None;
            d.recording = None;
        });
        self.ui().global::<SettingsData>().set_binding_message("All key bindings reset to the defaults.".into());
    }

    pub fn settings_set_binding_input(&mut self, row: usize, text: &str) {
        let Some(d) = self.draft.as_mut() else { return };
        let action = Action::SendInput(unescape_input(text));
        match d.rows.get(row) {
            Some(Row::Binding(i)) => d.keymap.bindings[*i].action = action,
            Some(Row::Pending) => d.pending = Some(action),
            None => return,
        }
        d.dirty = true;
        // Don't rebuild the list while the user is typing in it.
        self.ui().global::<SettingsData>().set_dirty(true);
    }

    /// A key press while a binding is being recorded.
    pub(crate) fn record_key(&mut self, key: Key, mods: Mods) {
        let Some(d) = self.draft.as_mut() else { return };
        let Some(row) = d.recording else { return };
        let names = d.profile_names();

        let message = if key == Key::Escape && mods == Mods::NONE {
            if d.rows.get(row) == Some(&Row::Pending) {
                d.pending = None;
            }
            d.recording = None;
            String::new()
        } else if key == Key::Backspace && mods == Mods::NONE {
            if let Some(Row::Binding(i)) = d.rows.get(row).copied() {
                let removed = d.keymap.bindings.remove(i);
                d.dirty = true;
                d.recording = None;
                format!("Removed {} from {}.", removed.chord.label(), removed.action.label(&names))
            } else {
                d.recording = None;
                d.pending = None;
                String::new()
            }
        } else if matches!(key, Key::Char(_)) && !mods.ctrl && !mods.alt && !mods.meta {
            // Plain typing keys would stop working in the terminal.
            "Use Ctrl, Alt or Win with that key (or a function key), so normal typing still works.".into()
        } else {
            let chord = Chord::new(key, mods);
            let Some(action) = d.row_action(row) else { return };
            let old_chord = match d.rows.get(row) {
                Some(Row::Binding(i)) => Some(d.keymap.bindings[*i].chord),
                _ => None,
            };
            let displaced: Vec<Binding> =
                d.keymap.bindings.iter().filter(|b| b.chord == chord && b.action != action).cloned().collect();
            d.keymap.bindings.retain(|b| b.chord != chord);
            match old_chord {
                Some(old) => {
                    if let Some(b) = d.keymap.bindings.iter_mut().find(|b| b.chord == old && b.action == action) {
                        b.chord = chord;
                    } else {
                        d.keymap.bindings.push(Binding { chord, action: action.clone() });
                    }
                }
                None => {
                    d.keymap.bindings.push(Binding { chord, action: action.clone() });
                    d.pending = None;
                }
            }
            d.dirty = true;
            d.recording = None;
            match displaced.first() {
                Some(old) => format!(
                    "{} now runs {} (it used to run {}).",
                    chord.label(),
                    action.label(&names),
                    old.action.label(&names)
                ),
                None => format!("{} now runs {}.", chord.label(), action.label(&names)),
            }
        };
        self.refresh_settings();
        self.ui().global::<SettingsData>().set_binding_message(message.into());
    }
}

/// Show whether wt++ is the system's default terminal.
fn refresh_default_terminal(ui: &MainWindow) {
    use crate::default_terminal::Status;
    let data = ui.global::<SettingsData>();
    data.set_default_term(match crate::default_terminal::status() {
        Status::Unsupported => 0,
        Status::NotDefault => 1,
        Status::Default => 2,
    });
    data.set_default_term_description(
        if cfg!(windows) {
            "Console programs (Command Prompt, PowerShell, …) started from the Start menu, Run or Explorer open in it."
        } else {
            "Your desktop opens it when it needs a terminal."
        }
        .into(),
    );
}
