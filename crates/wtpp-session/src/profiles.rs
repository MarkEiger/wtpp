//! Shell profiles and auto-discovery of the shells installed on this machine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use alacritty_terminal::vte::ansi::CursorShape;
use wtpp_input::KeyStyle;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub key_style: KeyStyle,
    /// Scrollback lines.
    pub history: usize,
    /// Cursor shape until the running app asks for a different one.
    pub cursor_shape: CursorShape,
    /// Characters that end a word for double-click selection.
    pub word_delimiters: String,
    /// Icon for discovered shells whose program has none embedded.
    pub icon: Option<String>,
}

impl Profile {
    pub fn new(name: impl Into<String>, program: impl Into<String>, args: &[&str]) -> Profile {
        let program = program.into();
        Profile {
            name: name.into(),
            key_style: key_style_for(&program),
            program,
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: None,
            env: HashMap::new(),
            history: 9001,
            cursor_shape: CursorShape::Beam,
            word_delimiters: ",│`|:\"' ()[]{}<>\t".into(),
            icon: None,
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

/// All shells we can find, the user's default first.
pub fn discover() -> Vec<Profile> {
    #[cfg(windows)]
    {
        windows::discover()
    }
    #[cfg(not(windows))]
    {
        let etc_shells = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        unix_profiles(std::env::var("SHELL").ok().as_deref(), &etc_shells, |p| p.exists())
    }
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

#[cfg(windows)]
mod windows {
    use super::Profile;
    use std::path::PathBuf;
    use std::process::Command;

    pub fn discover() -> Vec<Profile> {
        let mut profiles = Vec::new();

        if let Some(pwsh) = find_pwsh() {
            profiles.push(Profile::new("PowerShell", pwsh.to_string_lossy(), &["-NoLogo"]));
        }
        profiles.push(Profile::new("Windows PowerShell", "powershell.exe", &["-NoLogo"]));
        let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        profiles.push(Profile::new("Command Prompt", comspec, &[]));

        for distro in wsl_distros() {
            profiles.push(Profile::new(distro.clone(), "wsl.exe", &["-d", &distro, "--cd", "~"]));
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

        profiles
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

    /// `wsl -l -q` prints one distro per line, in UTF-16LE.
    fn wsl_distros() -> Vec<String> {
        use std::os::windows::process::CommandExt;
        // We're a GUI app: without this, a console window flashes up.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let Ok(out) = Command::new("wsl.exe").args(["-l", "-q"]).creation_flags(CREATE_NO_WINDOW).output() else {
            return Vec::new();
        };
        let utf16: Vec<u16> =
            out.stdout.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
        String::from_utf16_lossy(&utf16)
            .lines()
            .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\0').to_string())
            .filter(|l| !l.is_empty() && !l.starts_with("docker-desktop"))
            .collect()
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
}
