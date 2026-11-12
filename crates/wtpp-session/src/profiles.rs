//! Shell profiles and auto-discovery of the shells installed on this machine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use alacritty_terminal::vte::ansi::CursorShape;
use wtpp_config::settings::is_plausible_distro_name;
use wtpp_input::KeyStyle;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    /// The environment to start from instead of wt++'s own (see
    /// [`crate::env::pane_environment`]); `env` still goes over it.
    pub base_env: Option<crate::env::Vars>,
    pub key_style: KeyStyle,
    /// Send key events (win32-input-mode) without waiting for the console
    /// host to ask for them: the user said so, or it's known to take them.
    pub force_win32_input: bool,
    /// Scrollback lines.
    pub history: usize,
    /// Cursor shape until the running app asks for a different one.
    pub cursor_shape: CursorShape,
    /// Characters that end a word for double-click selection.
    pub word_delimiters: String,
    /// Icon for discovered shells whose program has none embedded.
    pub icon: Option<String>,
    /// The token the shell's ssh reports carry (see `wtpp_config::ssh`); the
    /// shell gets it in `env`. None: reports aren't taken.
    pub ssh_token: Option<String>,
}

impl Profile {
    pub fn new(name: impl Into<String>, program: impl Into<String>, args: &[&str]) -> Profile {
        let program = program.into();
        Profile {
            name: name.into(),
            key_style: key_style_for(&program),
            force_win32_input: false,
            program,
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: None,
            env: HashMap::new(),
            base_env: None,
            history: 9001,
            cursor_shape: CursorShape::Beam,
            word_delimiters: ",│`|:\"' ()[]{}<>\t".into(),
            icon: None,
            ssh_token: None,
        }
    }
}

/// cmd and PowerShell want Windows Terminal's bytes for word editing;
/// everything else (including `wsl.exe`, which runs a Linux shell) is Unix.
pub fn key_style_for(program: &str) -> KeyStyle {
    // Split on both separators so Windows paths parse on any host (settings
    // files are shared between machines).
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    match file.strip_suffix(".exe").unwrap_or(&file) {
        "cmd" | "powershell" | "pwsh" => KeyStyle::Windows,
        _ => KeyStyle::Unix,
    }
}

/// What [`discover`] found.
#[derive(Clone, Debug)]
pub struct Discovery {
    /// All shells we can find, the user's default first.
    pub profiles: Vec<Profile>,
    /// False when WSL couldn't be asked (it printed something other than a
    /// list, or hung): there may be more distros than `profiles` has.
    pub wsl_known: bool,
}

pub fn discover() -> Discovery {
    #[cfg(windows)]
    {
        windows::discover()
    }
    #[cfg(not(windows))]
    {
        let etc_shells = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        let profiles = unix_profiles(std::env::var("SHELL").ok().as_deref(), &etc_shells, |p| p.exists());
        Discovery { profiles, wsl_known: true }
    }
}

/// Text from `wsl.exe`: UTF-16LE, or UTF-8 under `WSL_UTF8=1`. NULs before
/// the very end, or a BOM, mean UTF-16 (a UTF-8 list has neither). `None`
/// if it's neither.
fn decode_wsl_output(stdout: &[u8]) -> Option<String> {
    if let Some(rest) = stdout.strip_prefix(&[0xFF, 0xFE]) {
        return Some(decode_utf16le(rest));
    }
    let end = stdout.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    if stdout[..end].contains(&0) {
        return Some(decode_utf16le(stdout));
    }
    let text = &stdout[..end];
    let text = text.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(text);
    std::str::from_utf8(text).ok().map(str::to_string)
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let utf16: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    String::from_utf16_lossy(&utf16)
}

fn wsl_output_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split(['\r', '\n']).map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\0')).filter(|l| !l.is_empty())
}

/// The distros in what `wsl.exe -l -q` printed, or `None` if it isn't a list
/// of distros. `-q` prints nothing but names, so one line that can't be a
/// name means the output is something else (its usage, on versions without
/// `-l`; an error message), in whatever language: all of it is rejected.
pub fn parse_wsl_list(stdout: &[u8], success: bool) -> Option<Vec<String>> {
    if !success {
        return None;
    }
    let text = decode_wsl_output(stdout)?;
    let lines: Vec<&str> = wsl_output_lines(&text).collect();
    if !lines.iter().all(|l| is_plausible_distro_name(l)) {
        return None;
    }
    Some(lines.into_iter().filter(|l| !is_hidden_distro(l)).map(str::to_string).collect())
}

/// Docker's internal distros aren't shells anyone wants a tab of.
fn is_hidden_distro(name: &str) -> bool {
    name.starts_with("docker-desktop")
}

/// One distro's values under `HKCU\...\Lxss`.
#[cfg_attr(not(windows), allow(dead_code))]
struct RegisteredDistro {
    key: String,
    name: String,
    state: Option<u32>,
}

/// The usable distros of the registry's list, the default first (WT's order).
/// State 1 is "installed"; others are being installed or removed.
#[cfg_attr(not(windows), allow(dead_code))]
fn registered_distro_names(distros: &[RegisteredDistro], default: Option<&str>) -> Vec<String> {
    let usable = |d: &&RegisteredDistro| d.state.is_none_or(|s| s == 1) && !is_hidden_distro(&d.name);
    let is_default = |d: &&RegisteredDistro| default.is_some_and(|k| d.key.eq_ignore_ascii_case(k));
    let first = distros.iter().filter(usable).filter(is_default);
    let rest = distros.iter().filter(usable).filter(|d| !is_default(d));
    first.chain(rest).map(|d| d.name.clone()).collect()
}

/// Arguments of a distro's profile, opening in the Linux home. `~` as the
/// first argument is new in Windows 10 1903 (build 18362). Not `--cd ~`: on
/// inbox WSL (10.0.19041) that fails with "cannot find the file specified".
#[cfg_attr(not(windows), allow(dead_code))]
fn wsl_args(distro: &str, windows_build: Option<u32>) -> Vec<String> {
    const HOME_SINCE_BUILD: u32 = 18362;
    let args = ["-d".to_string(), distro.to_string()];
    if windows_build.is_none_or(|b| b >= HOME_SINCE_BUILD) {
        std::iter::once("~".to_string()).chain(args).collect()
    } else {
        args.to_vec()
    }
}

/// Run `command` for its stdout and whether it succeeded. `None` if it took
/// longer than `timeout` (it's killed then). The pipe is read on its own
/// thread: a full one would block the child, and anything the child left
/// running can hold it open long after the child is gone.
#[cfg_attr(not(windows), allow(dead_code))]
fn output_with_timeout(mut command: Command, timeout: Duration) -> std::io::Result<Option<(Vec<u8>, bool)>> {
    use std::io::Read;
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    // The timeout below is a thread's: it dies with wt++, this doesn't.
    #[cfg(windows)]
    let _job = windows::KillWithUs::new(&child);
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).ok();
        sender.send(bytes).ok();
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                child.kill().ok();
                child.wait().ok();
                return Ok(None);
            }
        }
    };
    let drained = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(250));
    Ok(receiver.recv_timeout(drained).ok().map(|bytes| (bytes, status.success())))
}

/// Where CreateProcess would find `program` if this were its PATH: System32,
/// Windows, then `path`'s directories, adding `.exe` if it has no extension.
/// Quoted if it has a space, for the command line. `None` if it has a
/// directory already, or isn't found.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn find_program(program: &str, path: &str) -> Option<String> {
    if program.contains(['\\', '/', ':']) || program.starts_with('"') {
        return None;
    }
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let dirs = [format!(r"{root}\System32"), root].into_iter().chain(path.split(';').map(|d| d.trim().trim_matches('"').to_string()));
    let file = if Path::new(program).extension().is_some() { program.to_string() } else { format!("{program}.exe") };
    let found = dirs.filter(|d| !d.is_empty()).map(|d| Path::new(&d).join(&file)).find(|c| c.is_file())?;
    let found = found.to_string_lossy().into_owned();
    Some(if found.contains(' ') { format!("\"{found}\"") } else { found })
}

/// Profiles from `$SHELL` and `/etc/shells`, deduplicated by shell name
/// (`/bin/bash` and `/usr/bin/bash` are the same shell to a user).
#[cfg_attr(windows, allow(dead_code))]
fn unix_profiles(
    login_shell: Option<&str>,
    etc_shells: &str,
    exists: impl Fn(&Path) -> bool,
) -> Vec<Profile> {
    // Not interactive shells, or (rbash: restricted bash) not one anyone
    // wants a tab of.
    const NOT_SHELLS: &[&str] = &["nologin", "false", "git-shell", "tmux", "screen", "rbash"];

    let candidates = login_shell.into_iter().chain(
        etc_shells.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')),
    );

    let mut profiles: Vec<Profile> = Vec::new();
    for path in candidates {
        let Some(name) = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned())
        else {
            continue;
        };
        if NOT_SHELLS.contains(&name.as_str())
            || profiles.iter().any(|p| p.name == name)
            || !exists(Path::new(path))
        {
            continue;
        }
        profiles.push(Profile::new(name, path, &["-l"]));
    }

    if profiles.is_empty() {
        profiles.push(Profile::new("sh", "/bin/sh", &["-l"]));
    }
    profiles
}

/// The Windows build number (17763 is 1809), from the registry.
#[cfg(windows)]
pub use windows::windows_build;

#[cfg(windows)]
mod windows {
    use super::{Discovery, Profile, RegisteredDistro};
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey,
        RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    };

    const LXSS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";
    const WSL_TIMEOUT: Duration = Duration::from_secs(5);

    /// A job object that takes its processes with it when closed, which
    /// wt++ exiting does too. Without one, things only stay as they were.
    pub struct KillWithUs(HANDLE);

    impl KillWithUs {
        pub fn new(child: &std::process::Child) -> Option<KillWithUs> {
            use std::os::windows::io::AsRawHandle;
            // SAFETY: no name or attributes; the handle is closed in drop.
            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if job.is_null() {
                log::warn!("couldn't create a job object: {}", std::io::Error::last_os_error());
                return None;
            }
            let job = KillWithUs(job);
            // SAFETY: all-zero is valid for this plain struct.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `limits` is of the class's type and size; both handles are open.
            let assigned = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const c_void,
                    size_of_val(&limits) as u32,
                ) != 0
                    && AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) != 0
            };
            if !assigned {
                log::warn!("couldn't put a process in a job object: {}", std::io::Error::last_os_error());
                return None;
            }
            Some(job)
        }
    }

    impl Drop for KillWithUs {
        fn drop(&mut self) {
            // SAFETY: opened in new, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn discover() -> Discovery {
        let mut profiles = Vec::new();

        if let Some(pwsh) = find_pwsh() {
            profiles.push(Profile::new("PowerShell", pwsh.to_string_lossy(), &["-NoLogo"]));
        }
        profiles.push(Profile::new("Windows PowerShell", "powershell.exe", &["-NoLogo"]));
        let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        profiles.push(Profile::new("Command Prompt", comspec, &[]));

        let (distros, wsl_known) = wsl_distros();
        let build = windows_build();
        for distro in distros {
            let mut profile = Profile::new(distro.clone(), "wsl.exe", &[]);
            profile.args = super::wsl_args(&distro, build);
            profiles.push(profile);
        }

        let program_files = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let git_bash = PathBuf::from(&program_files).join(r"Git\bin\bash.exe");
        if git_bash.exists() {
            let mut profile = Profile::new("Git Bash", git_bash.to_string_lossy(), &["-i", "-l"]);
            // bash.exe has no icon of its own; WT uses Git's.
            let icon = PathBuf::from(&program_files).join(r"Git\mingw64\share\git\git-for-windows.ico");
            profile.icon = icon.exists().then(|| icon.to_string_lossy().into_owned());
            profiles.push(profile);
        }

        Discovery { profiles, wsl_known }
    }

    fn find_pwsh() -> Option<PathBuf> {
        let program_files = std::env::var("ProgramFiles").ok()?;
        let installed = PathBuf::from(program_files).join(r"PowerShell\7\pwsh.exe");
        if installed.exists() {
            return Some(installed);
        }
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths).map(|p| p.join("pwsh.exe")).find(|p| p.exists())
        })
    }

    /// WSL's distros, and whether that's a real answer. From the registry,
    /// like WT: no process to start, and the same on all versions. `wsl.exe`
    /// only if the registry has none.
    fn wsl_distros() -> (Vec<String>, bool) {
        let registered = registered_distros();
        if !registered.is_empty() {
            let default = reg_string(HKEY_CURRENT_USER, LXSS_KEY, "DefaultDistribution");
            let names = super::registered_distro_names(&registered, default.as_deref());
            log::info!("WSL distros from the registry: {names:?}");
            return (names, true);
        }
        use std::os::windows::process::CommandExt;
        // We're a GUI app: without this, a console window flashes up.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = Command::new("wsl.exe");
        command.args(["-l", "-q"]).creation_flags(CREATE_NO_WINDOW);
        match super::output_with_timeout(command, WSL_TIMEOUT) {
            Ok(Some((stdout, success))) => match super::parse_wsl_list(&stdout, success) {
                Some(names) => {
                    log::info!("WSL distros from wsl.exe: {names:?}");
                    (names, true)
                }
                None => {
                    let text = super::decode_wsl_output(&stdout).unwrap_or_default();
                    let first = super::wsl_output_lines(&text).next().unwrap_or_default().to_string();
                    log::warn!("wsl.exe -l -q (success: {success}) didn't print a list of distros: {first:?}");
                    (Vec::new(), false)
                }
            },
            Ok(None) => {
                log::warn!("wsl.exe -l -q didn't finish in {WSL_TIMEOUT:?}; killed it");
                (Vec::new(), false)
            }
            // No WSL here, and that's an answer.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::info!("no WSL: not in the registry, and no wsl.exe");
                (Vec::new(), true)
            }
            Err(e) => {
                log::warn!("couldn't run wsl.exe: {e}");
                (Vec::new(), false)
            }
        }
    }

    /// Entries under Lxss that are distros (it has other subkeys too). A
    /// name that can't be quoted on a command line couldn't be started.
    fn registered_distros() -> Vec<RegisteredDistro> {
        reg_subkeys(HKEY_CURRENT_USER, LXSS_KEY)
            .into_iter()
            .filter_map(|key| {
                let path = format!(r"{LXSS_KEY}\{key}");
                let name = reg_string(HKEY_CURRENT_USER, &path, "DistributionName")?;
                let name = name.trim().to_string();
                if name.is_empty() || name.contains(|c: char| c == '"' || c.is_control()) {
                    log::warn!("ignoring WSL distro {name:?} in the registry");
                    return None;
                }
                Some(RegisteredDistro { state: reg_dword(HKEY_CURRENT_USER, &path, "State"), name, key })
            })
            .collect()
    }

    /// Windows' build number. From the registry: unlike GetVersionEx, it
    /// doesn't depend on the application manifest.
    pub fn windows_build() -> Option<u32> {
        let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let build = reg_string(HKEY_LOCAL_MACHINE, key, "CurrentBuildNumber")?.trim().parse().ok();
        log::info!("Windows build {build:?}");
        build
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut buf = vec![0u16; 512];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: `buf` holds `size` bytes.
        let status = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_void,
                &mut size,
            )
        };
        (status == 0).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    fn reg_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
        let (key, value) = (wide(key), wide(value));
        let mut data = 0u32;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: `data` holds `size` bytes.
        let status = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut data as *mut u32 as *mut c_void,
                &mut size,
            )
        };
        (status == 0).then_some(data)
    }

    /// Names of a key's subkeys; none if there's no such key.
    fn reg_subkeys(root: HKEY, key: &str) -> Vec<String> {
        let key = wide(key);
        let mut handle: HKEY = std::ptr::null_mut();
        // SAFETY: `key` is NUL-terminated; `handle` receives the opened key.
        if unsafe { RegOpenKeyExW(root, key.as_ptr(), 0, KEY_READ, &mut handle) } != 0 {
            return Vec::new();
        }
        let mut names = Vec::new();
        for index in 0.. {
            // Key names are at most 255 characters.
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            // SAFETY: `name` holds `len` characters; the rest is optional.
            let status = unsafe {
                RegEnumKeyExW(
                    handle,
                    index,
                    name.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status != 0 {
                break;
            }
            names.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        // SAFETY: opened above, closed once.
        unsafe { RegCloseKey(handle) };
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ETC_SHELLS: &str = "# /etc/shells: valid login shells\n\
        /bin/sh\n/usr/bin/sh\n/bin/bash\n/usr/bin/bash\n/usr/bin/zsh\n\
        /usr/bin/tmux\n/usr/sbin/nologin\n/usr/bin/fish\n";

    #[test]
    fn login_shell_first_and_deduplicated() {
        let profiles = unix_profiles(Some("/usr/bin/zsh"), ETC_SHELLS, |_| true);
        let names: Vec<_> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["zsh", "sh", "bash", "fish"]);
        assert_eq!(profiles[0].program, "/usr/bin/zsh");
    }

    #[test]
    fn skips_missing_binaries() {
        let profiles = unix_profiles(None, ETC_SHELLS, |p| !p.ends_with("fish"));
        assert!(profiles.iter().all(|p| p.name != "fish"));
    }

    #[test]
    fn falls_back_to_sh() {
        let profiles = unix_profiles(None, "", |_| true);
        assert_eq!(profiles[0].program, "/bin/sh");
    }

    #[test]
    fn key_styles() {
        assert_eq!(key_style_for(r"C:\Windows\System32\cmd.exe"), KeyStyle::Windows);
        assert_eq!(key_style_for("pwsh"), KeyStyle::Windows);
        assert_eq!(key_style_for("wsl.exe"), KeyStyle::Unix);
        assert_eq!(key_style_for("/bin/bash"), KeyStyle::Unix);
    }

    const USAGE: &str = "Usage: wsl.exe [Argument] [Options...] [CommandLine]\r\n\
        \r\n\
        Arguments for running Linux binaries:\r\n\
        \r\n\
        \x20   If no command line is provided, wsl.exe launches the default shell.\r\n\
        \r\n\
        \x20   --exec, -e <CommandLine>\r\n\
        \x20       Execute the specified command without using the default Linux shell.\r\n\
        \r\n\
        \x20   --shell-type <standard|login|none>\r\n\
        \x20       Execute the specified command with the provided shell type.\r\n\
        \r\n\
        \x20   --\r\n\
        \x20       Pass the remaining command line as-is.\r\n";

    const USAGE_GERMAN: &str = "Syntax: wsl.exe [Argument] [Optionen...] [Befehlszeile]\r\n\
        \r\n\
        Argumente zum Ausführen von Linux-Binärdateien:\r\n\
        \r\n\
        \x20   Wenn keine Befehlszeile angegeben wird, startet wsl.exe die Standardshell.\r\n\
        \r\n\
        \x20   --exec, -e <Befehlszeile>\r\n\
        \x20       Führt den angegebenen Befehl aus, ohne die Linux-Standardshell zu verwenden.\r\n";

    const NO_DISTROS: &str = "Windows Subsystem for Linux has no installed distributions.\r\n\
        Distributions can be installed by visiting the Microsoft Store:\r\n\
        https://aka.ms/wslstore\r\n";

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn list(names: &[&str]) -> Option<Vec<String>> {
        Some(names.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn usage_is_never_a_list() {
        for text in [USAGE, USAGE_GERMAN, NO_DISTROS] {
            for success in [true, false] {
                assert_eq!(parse_wsl_list(&utf16(text), success), None, "UTF-16 {text}");
                assert_eq!(parse_wsl_list(text.as_bytes(), success), None, "UTF-8 {text}");
                let with_bom: Vec<u8> = [0xFF, 0xFE].into_iter().chain(utf16(text)).collect();
                assert_eq!(parse_wsl_list(&with_bom, success), None, "BOM {text}");
            }
        }
    }

    #[test]
    fn legacy_codepage_output_is_rejected() {
        // Not UTF-8 ("für" in a Windows code page), no NULs.
        assert_eq!(parse_wsl_list(b"Hilfe f\xfcr wsl\r\n", true), None);
        assert_eq!(parse_wsl_list(b"f\xfcr\r\n", true), None);
    }

    #[test]
    fn failure_is_rejected_even_with_a_good_list() {
        assert_eq!(parse_wsl_list(&utf16("Ubuntu\r\n"), false), None);
        assert_eq!(parse_wsl_list(b"", false), None);
    }

    #[test]
    fn one_junk_line_rejects_everything() {
        assert_eq!(parse_wsl_list(&utf16("Ubuntu\r\nNot a distro\r\nDebian\r\n"), true), None);
        assert_eq!(parse_wsl_list(b"Ubuntu\n--help\n", true), None);
    }

    #[test]
    fn utf16_lists() {
        let want = list(&["Ubuntu-22.04", "Debian"]);
        // As wsl.exe prints it: CRLF, no BOM.
        assert_eq!(parse_wsl_list(&utf16("Ubuntu-22.04\r\nDebian\r\n"), true), want);
        // BOM.
        let with_bom: Vec<u8> = [0xFF, 0xFE].into_iter().chain(utf16("Ubuntu-22.04\r\nDebian\r\n")).collect();
        assert_eq!(parse_wsl_list(&with_bom, true), want);
        // Trailing NUL character, a stray NUL byte, blank lines, spaces.
        assert_eq!(parse_wsl_list(&utf16("Ubuntu-22.04\r\nDebian\r\n\0"), true), want);
        let mut odd = utf16("Ubuntu-22.04\r\nDebian");
        odd.push(0);
        assert_eq!(parse_wsl_list(&odd, true), want);
        assert_eq!(parse_wsl_list(&utf16("\r\n Ubuntu-22.04 \r\n\r\n\0Debian\0\r\n"), true), want);
        // LF only, and one name with no newline.
        assert_eq!(parse_wsl_list(&utf16("Ubuntu-22.04\nDebian"), true), want);
        assert_eq!(parse_wsl_list(&utf16("Debian"), true), list(&["Debian"]));
    }

    #[test]
    fn utf8_lists() {
        let want = list(&["Ubuntu", "kali-linux"]);
        assert_eq!(parse_wsl_list(b"Ubuntu\nkali-linux\n", true), want);
        assert_eq!(parse_wsl_list(b"Ubuntu\r\nkali-linux\r\n", true), want);
        assert_eq!(parse_wsl_list(b"\xEF\xBB\xBFUbuntu\nkali-linux", true), want);
        // Trailing NULs don't make it UTF-16.
        assert_eq!(parse_wsl_list(b"Ubuntu\nkali-linux\n\0", true), want);
        assert_eq!(parse_wsl_list(b"Ubuntu\nkali-linux\0\0", true), want);
    }

    #[test]
    fn empty_lists() {
        for bytes in [&b""[..], b"\r\n", b"\0\0", b" \n ", &[0xFF, 0xFE], &[0xEF, 0xBB, 0xBF]] {
            assert_eq!(parse_wsl_list(bytes, true), list(&[]), "{bytes:?}");
        }
        assert_eq!(parse_wsl_list(&utf16("\r\n\r\n"), true), list(&[]));
    }

    #[test]
    fn real_world_names() {
        let names = ["Ubuntu-22.04", "kali-linux", "OracleLinux_8_5", "SUSE-Linux-Enterprise-Server-15-SP4"];
        let text = names.join("\r\n");
        assert_eq!(parse_wsl_list(&utf16(&text), true), list(&names));
        assert_eq!(parse_wsl_list(text.as_bytes(), true), list(&names));
        // Docker's are real distros, but not for us.
        let docker = "docker-desktop\r\nUbuntu\r\ndocker-desktop-data\r\n";
        assert_eq!(parse_wsl_list(&utf16(docker), true), list(&["Ubuntu"]));
        assert_eq!(parse_wsl_list(&utf16("docker-desktop\r\n"), true), list(&[]));
    }

    fn registered(key: &str, name: &str, state: Option<u32>) -> RegisteredDistro {
        RegisteredDistro { key: key.into(), name: name.into(), state }
    }

    #[test]
    fn registry_distros_default_first() {
        let distros = [
            registered("{a}", "Alpine", Some(1)),
            registered("{b}", "docker-desktop", Some(1)),
            registered("{c}", "My Distro", None),
            registered("{d}", "Installing", Some(3)),
            registered("{E}", "Debian", Some(1)),
            registered("{f}", "docker-desktop-data", None),
        ];
        assert_eq!(registered_distro_names(&distros, Some("{e}")), ["Debian", "Alpine", "My Distro"]);
        assert_eq!(registered_distro_names(&distros, None), ["Alpine", "My Distro", "Debian"]);
        assert_eq!(registered_distro_names(&distros, Some("{gone}")), ["Alpine", "My Distro", "Debian"]);
        // The default isn't special if it can't be used.
        assert_eq!(registered_distro_names(&distros, Some("{d}")), ["Alpine", "My Distro", "Debian"]);
        assert!(registered_distro_names(&[], Some("{e}")).is_empty());
    }

    #[test]
    fn home_only_where_windows_has_it() {
        let commandline = |build| wtpp_config::settings::join_commandline_windows("wsl.exe", &wsl_args("Debian", build));
        assert_eq!(commandline(Some(17763)), "wsl.exe -d Debian");
        assert_eq!(commandline(Some(18361)), "wsl.exe -d Debian");
        assert_eq!(commandline(Some(18362)), "wsl.exe ~ -d Debian");
        assert_eq!(commandline(Some(19044)), "wsl.exe ~ -d Debian");
        assert_eq!(commandline(Some(26100)), "wsl.exe ~ -d Debian");
        assert_eq!(commandline(None), "wsl.exe ~ -d Debian");
    }

    #[test]
    fn distro_names_survive_the_commandline() {
        use wtpp_config::settings::{join_commandline_windows as join_commandline, split_commandline_windows as split_commandline};
        for distro in ["My Distro", "Ubuntu-22.04", "Tab\tName", "ünï cödé", "a&b (c)", "100%"] {
            let args = wsl_args(distro, Some(26100));
            let line = join_commandline("wsl.exe", &args);
            let (program, parsed) = split_commandline(&line);
            assert_eq!(program, "wsl.exe");
            assert_eq!(parsed, args, "{line}");
        }
        assert_eq!(
            join_commandline("wsl.exe", &wsl_args("My Distro", Some(26100))),
            r#"wsl.exe ~ -d "My Distro""#
        );
    }

    /// The real thing, on the machine running the tests.
    #[cfg(windows)]
    #[test]
    fn discovers_this_machine() {
        let discovery = discover();
        println!("{discovery:#?}");
        assert!(discovery.profiles.iter().any(|p| p.name == "Command Prompt"));
        for p in discovery.profiles.iter().filter(|p| p.program == "wsl.exe") {
            assert_eq!(p.args[p.args.len() - 2..], ["-d".to_string(), p.name.clone()]);
        }
    }

    #[cfg(windows)]
    #[test]
    fn programs_found_in_a_given_path() {
        let root = std::env::var("SystemRoot").unwrap();
        let found = |p: &str, path: &str| find_program(p, path).map(|f| f.to_lowercase());
        assert_eq!(found("cmd", ""), Some(format!(r"{root}\System32\cmd.exe").to_lowercase()));
        assert_eq!(found("cmd.exe", r"C:\nowhere"), Some(format!(r"{root}\System32\cmd.exe").to_lowercase()));
        let ps = format!(r"{root}\System32\WindowsPowerShell\v1.0");
        assert_eq!(found("powershell", &format!(r"C:\nowhere;;{ps}")), Some(format!(r"{ps}\powershell.exe").to_lowercase()));
        // Only what that PATH has, and nothing with a directory of its own.
        assert_eq!(found("powershell", ""), None);
        assert_eq!(found("wtpp-no-such-program", &ps), None);
        assert_eq!(found(r"C:\x\y.exe", &ps), None);
        assert_eq!(found("./y", &ps), None);
        // A space: quoted for the command line.
        let dir = std::env::temp_dir().join(format!("wtpp find {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tool.exe"), b"").unwrap();
        let got = find_program("tool", &dir.to_string_lossy()).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert!(got.starts_with('"') && got.ends_with(r#"tool.exe""#), "{got}");
    }

    #[cfg(windows)]
    #[test]
    fn job_takes_the_process_along() {
        let mut child = shell("ping -n 60 127.0.0.1 > nul").stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let job = windows::KillWithUs::new(&child).expect("a job");
        assert!(child.try_wait().unwrap().is_none());
        drop(job);
        let started = Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(started.elapsed() < Duration::from_secs(5), "still running");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn shell(script: &str) -> Command {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let mut command = Command::new("cmd.exe");
            command.creation_flags(0x0800_0000).raw_arg(format!("/c {script}"));
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            command
        }
    }

    #[test]
    fn output_of_a_process() {
        let long = Duration::from_secs(20);
        let (stdout, success) = output_with_timeout(shell("echo Debian"), long).unwrap().unwrap();
        assert!(success);
        assert_eq!(parse_wsl_list(&stdout, success), list(&["Debian"]));
        let (_, success) = output_with_timeout(shell("exit 3"), long).unwrap().unwrap();
        assert!(!success);
        assert_eq!(output_with_timeout(Command::new("wtpp-no-such-program"), long).unwrap_err().kind(), std::io::ErrorKind::NotFound);
        // LxssManager disabled makes wsl.exe exit with 0xC0000409, and rc -1 is
        // common: all that matters of the code is that it isn't 0.
        #[cfg(windows)]
        for code in ["-1", "-1073740791"] {
            let (_, success) = output_with_timeout(shell(&format!("exit {code}")), long).unwrap().unwrap();
            assert!(!success, "{code}");
        }
    }

    #[test]
    fn hung_process_is_killed_on_time() {
        // cmd's `ping` and sh's `sleep` outlive the shell if it's the one
        // killed, holding the pipe open: that mustn't hold us up either.
        let hang = if cfg!(windows) { "ping -n 4 127.0.0.1 > nul" } else { "sleep 3; true" };
        let started = Instant::now();
        assert_eq!(output_with_timeout(shell(hang), Duration::from_millis(200)).unwrap(), None);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn exited_process_holding_its_pipe_open() {
        // The shell is gone at once, but what it left running keeps the pipe.
        let script = if cfg!(windows) { "start /b ping -n 4 127.0.0.1" } else { "sleep 3 & exit 0" };
        let started = Instant::now();
        assert_eq!(output_with_timeout(shell(script), Duration::from_millis(200)).unwrap(), None);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn big_output_does_not_deadlock() {
        let script = if cfg!(windows) { "for /l %i in (1,1,3000) do @echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } else { "head -c 1000000 /dev/zero" };
        let (stdout, success) = output_with_timeout(shell(script), Duration::from_secs(30)).unwrap().unwrap();
        assert!(success);
        assert!(stdout.len() > 100_000, "{}", stdout.len());
    }

    // ---- real captures from Windows 10 VMs: `wsl.exe -l -q`, exact bytes ----

    /// A capture's stdout and exit code (signed, as Windows' codes come out).
    struct Capture {
        stdout: &'static [u8],
        rc: i64,
    }

    macro_rules! capture {
        ($state:literal) => {
            capture!($state, "wsl-l-q")
        };
        ($state:literal, $command:literal) => {
            Capture {
                stdout: include_bytes!(concat!("../testdata/wsl/", $state, "/", $command, ".stdout.bin")),
                rc: include_str!(concat!("../testdata/wsl/", $state, "/", $command, ".rc")).trim().parse().unwrap(),
            }
        };
    }

    impl Capture {
        fn parsed(&self) -> Option<Vec<String>> {
            parse_wsl_list(self.stdout, self.rc == 0)
        }

        /// The lines 1.1.2 turned into profiles.
        fn lines(&self) -> Vec<String> {
            let text = decode_wsl_output(self.stdout).unwrap();
            wsl_output_lines(&text).map(str::to_string).collect()
        }
    }

    fn state_a() -> Capture {
        capture!("A-feature-off")
    }

    fn state_1809() -> Capture {
        capture!("1809-B-feature-on-no-distro")
    }

    #[test]
    fn captures_parse() {
        let a = state_a();
        assert_eq!(a.rc, 1);
        assert_eq!(a.parsed(), None);
        let d = capture!("D-feature-enabled-pending-reboot");
        assert_eq!(d.rc, 1);
        assert_eq!(d.parsed(), None);
        let b = capture!("B-feature-on-no-distro");
        assert_eq!((b.rc, b.stdout.len()), (0, 0));
        assert_eq!(b.parsed(), list(&[]));
        let c = capture!("C-wsl1-two-distros");
        assert_eq!(c.rc, 0);
        assert_eq!(c.parsed(), list(&["Alpine", "test-distro"]));
        assert_eq!(capture!("C1-distros-lxss-stopped").parsed(), list(&["Alpine", "test-distro"]));
        let c2 = capture!("C2-distros-lxss-disabled");
        assert_eq!((c2.rc, c2.stdout.len()), (-1073740791, 0));
        assert_eq!(c2.parsed(), None);
        let old = state_1809();
        assert_eq!(old.rc, -1);
        assert_eq!(old.parsed(), None);
        // Other things it prints when it can't list anything.
        assert_eq!(capture!("B-feature-on-no-distro", "wsl--version").parsed(), None);
        assert_eq!(capture!("B-feature-on-no-distro", "wsl-l").parsed(), None);
    }

    #[test]
    fn captured_text_is_rejected_whatever_the_exit_code() {
        for c in [
            state_a(),
            state_1809(),
            capture!("D-feature-enabled-pending-reboot"),
            capture!("B-feature-on-no-distro", "wsl--version"),
            capture!("B-feature-on-no-distro", "wsl-l"),
        ] {
            assert_eq!(parse_wsl_list(c.stdout, true), None, "{:?}", c.lines().first());
            assert!(!c.lines().is_empty());
        }
    }

    #[test]
    fn no_captured_junk_line_passes_for_a_distro() {
        for c in [state_a(), state_1809(), capture!("B-feature-on-no-distro", "wsl--version")] {
            for line in c.lines() {
                assert!(!is_plausible_distro_name(&line), "{line:?}");
            }
        }
    }

    fn v112_profile(name: &str) -> wtpp_config::settings::ProfileSettings {
        use wtpp_config::settings::{DISCOVERED_SOURCE, guid_for, join_commandline_windows as join_commandline};
        let args: Vec<String> = ["-d", name, "--cd", "~"].iter().map(|s| s.to_string()).collect();
        wtpp_config::settings::ProfileSettings {
            guid: Some(guid_for(name)),
            name: Some(name.into()),
            commandline: Some(join_commandline("wsl.exe", &args)),
            source: Some(DISCOVERED_SOURCE.into()),
            ..Default::default()
        }
    }

    /// The found list and `wsl_known` of a machine in a captured state where
    /// the registry has nothing, so `wsl.exe` gets asked.
    fn discovered_in(capture: &Capture) -> (Vec<wtpp_config::settings::DiscoveredProfile>, bool) {
        use wtpp_config::settings::{DiscoveredProfile, join_commandline_windows as join_commandline};
        let parsed = capture.parsed();
        let wsl_known = parsed.is_some();
        let mut found = vec![
            DiscoveredProfile { name: "Windows PowerShell".into(), commandline: "powershell.exe -NoLogo".into(), icon: None },
            DiscoveredProfile { name: "Command Prompt".into(), commandline: r"C:\Windows\system32\cmd.exe".into(), icon: None },
        ];
        found.extend(parsed.unwrap_or_default().into_iter().map(|d| DiscoveredProfile {
            commandline: join_commandline("wsl.exe", &wsl_args(&d, Some(19044))),
            name: d,
            icon: None,
        }));
        (found, wsl_known)
    }

    #[test]
    fn captured_junk_goes_and_everything_else_stays() {
        use wtpp_config::settings::ProfileSettings;
        const KEPT_DISTROS: [&str; 4] = ["Ubuntu-22.04", "Alpine", "test-distro", "kali-linux"];
        for capture in [state_a(), state_1809(), capture!("D-feature-enabled-pending-reboot")] {
            let junk = capture.lines();
            assert!(junk.len() > 10);
            let (found, wsl_known) = discovered_in(&capture);
            assert!(!wsl_known);

            let mut s = wtpp_config::Settings::default();
            s.profiles.list.push(ProfileSettings { color_scheme: Some("Vintage".into()), ..v112_profile("Command Prompt") });
            s.profiles.list.extend(junk.iter().map(|l| v112_profile(l)));
            s.profiles.list.extend(KEPT_DISTROS.iter().map(|d| v112_profile(d)));
            // The user's own, however junk it looks.
            s.profiles.list.push(ProfileSettings {
                name: Some(junk[0].clone() + " (mine)"),
                commandline: Some("wsl.exe -d \"not a distro\"".into()),
                ..Default::default()
            });

            let report = s.refresh_discovered(&found, wsl_known);
            assert_eq!(report.removed, junk);
            assert_eq!(report.added, ["Windows PowerShell"]);
            assert!(report.wsl_kept, "real WSL profiles were kept, so say so");
            let names: Vec<&str> = s.profiles.list.iter().filter_map(|p| p.name.as_deref()).collect();
            let mine = junk[0].clone() + " (mine)";
            let mut want = vec!["Command Prompt"];
            want.extend(KEPT_DISTROS);
            want.extend([mine.as_str(), "Windows PowerShell"]);
            assert_eq!(names, want);
            assert_eq!(s.profiles.list[0].color_scheme.as_deref(), Some("Vintage"));
        }
    }

    /// settings.json as 1.1.2 really wrote it in those VMs.
    fn real_1_1_2(json: &str) -> wtpp_config::Settings {
        let settings = wtpp_config::Settings::parse(json).unwrap();
        assert!(!settings.profiles_discovered, "1.1.2 has no such flag: this file gets its one run");
        settings
    }

    fn assert_only_real_shells_left(s: &wtpp_config::Settings) {
        let names: Vec<&str> = s.profiles.list.iter().filter_map(|p| p.name.as_deref()).collect();
        assert_eq!(names, ["Windows PowerShell", "Command Prompt"]);
    }

    /// WSL off: no registry key, usage text, rc 1. wsl_known is false, the
    /// junk goes anyway, and there's no note, as no WSL profile exists.
    #[test]
    fn real_settings_upgrade_with_wsl_off() {
        for (json, capture) in [
            (include_str!("../testdata/settings-1.1.2/A-feature-off.json"), state_a()),
            (include_str!("../testdata/settings-1.1.2/1809-B-feature-on-no-distro.json"), state_1809()),
        ] {
            let mut s = real_1_1_2(json);
            // That file's junk is that capture's lines (1.1.2 skipped repeats).
            let junk: Vec<String> = s.profiles.list[2..].iter().filter_map(|p| p.name.clone()).collect();
            let mut lines = capture.lines();
            let mut seen = std::collections::HashSet::new();
            lines.retain(|l| seen.insert(l.clone()));
            assert_eq!(junk, lines);

            let (found, wsl_known) = discovered_in(&capture);
            assert!(!wsl_known);
            let report = s.refresh_discovered(&found, wsl_known);
            assert_eq!(report.removed, junk);
            assert_eq!(report.added, Vec::<String>::new());
            assert_only_real_shells_left(&s);
            assert_eq!(report.summary().matches("WSL").count(), 0, "{}", report.summary());
        }
    }

    /// 1.1.2's junk is still in the file, and by now WSL works, listing nothing.
    #[test]
    fn real_settings_upgrade_after_wsl_got_enabled() {
        let mut s = real_1_1_2(include_str!("../testdata/settings-1.1.2/D2-pending-then-rebooted-settings-kept.json"));
        assert_eq!(s.profiles.list.len(), 23);
        let (found, wsl_known) = discovered_in(&capture!("B-feature-on-no-distro"));
        assert!(wsl_known);
        let report = s.refresh_discovered(&found, wsl_known);
        assert_eq!(report.removed.len(), 21);
        assert!(!report.wsl_kept);
        assert_only_real_shells_left(&s);
    }

    /// Real distros and 1.1.2's broken `--cd ~`: fixed in place, not doubled.
    #[test]
    fn upgrade_with_real_distros_rewrites_commandlines() {
        use wtpp_config::settings::ProfileSettings;
        let mut s = wtpp_config::Settings::default();
        s.profiles.list.push(ProfileSettings { color_scheme: Some("Vintage".into()), ..v112_profile("Alpine") });
        s.profiles.list.push(ProfileSettings { name: Some("Testing".into()), guid: Some("{x}".into()), ..v112_profile("test-distro") });
        let (found, wsl_known) = discovered_in(&capture!("C-wsl1-two-distros"));
        let report = s.refresh_discovered(&found, wsl_known);
        assert_eq!(report.added, ["Windows PowerShell", "Command Prompt"]);
        assert_eq!(report.removed, Vec::<String>::new());
        assert_eq!(s.profiles.list[0].commandline.as_deref(), Some("wsl.exe ~ -d Alpine"));
        assert_eq!(s.profiles.list[0].color_scheme.as_deref(), Some("Vintage"));
        assert_eq!(s.profiles.list[1].commandline.as_deref(), Some("wsl.exe ~ -d test-distro"));
        assert_eq!(s.profiles.list[1].name.as_deref(), Some("Testing"));
        assert_eq!(s.profiles.list.len(), 4);
    }
}
