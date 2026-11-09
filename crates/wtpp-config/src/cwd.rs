//! Opening a new pane in another pane's current directory: which directory
//! that is for the new pane's shell, and having shells report it.

use crate::settings::{is_wsl_program, join_commandline, split_commandline};

/// The distro a WSL command line runs (`-d`), or `None` for the default one
/// or a command line that isn't WSL's.
fn distro(commandline: &str) -> Option<String> {
    let (program, args) = split_commandline(commandline);
    if !is_wsl_program(&program) {
        return None;
    }
    let i = args.iter().position(|a| a == "-d" || a == "--distribution")?;
    args.get(i + 1).cloned()
}

fn is_wsl(commandline: &str) -> bool {
    is_wsl_program(&split_commandline(commandline).0)
}

fn is_cmd(commandline: &str) -> bool {
    let (program, _) = split_commandline(commandline);
    let file = program.rsplit(['/', '\\']).next().unwrap_or(&program).to_ascii_lowercase();
    file == "cmd" || file == "cmd.exe"
}

/// `\\wsl$\Debian\home` (or `\\wsl.localhost\…`, which Windows 10 lacks): the
/// distro and the Linux path.
fn wsl_share(path: &str) -> Option<(&str, String)> {
    let lower = path.to_ascii_lowercase();
    let rest = ["\\\\wsl$\\", "\\\\wsl.localhost\\"].iter().find(|p| lower.starts_with(*p)).map(|p| &path[p.len()..])?;
    let (distro, dir) = rest.split_once('\\').unwrap_or((rest, ""));
    (!distro.is_empty()).then(|| (distro, format!("/{}", dir.replace('\\', "/"))))
}

/// `\\wsl$\<distro>\<path>`: a Linux directory as a Windows working
/// directory, which wsl.exe starts in, old and new alike (`--cd` with a
/// Linux path fails on Windows 10's).
fn wsl_unc(distro: &str, linux_path: &str) -> String {
    format!(r"\\wsl$\{distro}{}", linux_path.trim_end_matches('/').replace('/', "\\"))
}

/// `/mnt/c/Users` → `C:\Users`: a Windows drive as WSL mounts it.
fn mounted_drive(linux_path: &str) -> Option<String> {
    let rest = linux_path.strip_prefix("/mnt/")?;
    let (drive, dir) = rest.split_once('/').unwrap_or((rest, ""));
    let letter = drive.chars().next().filter(|c| drive.len() == 1 && c.is_ascii_alphabetic())?;
    Some(format!("{}:\\{}", letter.to_ascii_uppercase(), dir.trim_end_matches('/').replace('/', "\\")))
}

fn is_drive_path(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// The starting directory for a new pane running `target`, from `cwd`, the
/// current directory of a pane running `source`; `None` where it doesn't
/// translate cleanly (the new pane then starts where its profile says).
/// Windows drives carry over both ways (as `/mnt/c/…` in WSL); a directory
/// that's only in Linux, only to the same distro.
pub fn starting_directory_from(cwd: &str, source: &str, target: &str) -> Option<String> {
    let cwd = cwd.trim();
    let same_distro = |d: &str| distro(target).is_some_and(|t| t.eq_ignore_ascii_case(d));
    if cwd.starts_with('/') {
        if !is_wsl(source) {
            // A Unix machine's own directory.
            return (!is_wsl(target)).then(|| cwd.to_string());
        }
        if let Some(windows) = mounted_drive(cwd) {
            return Some(windows);
        }
        let source_distro = distro(source)?;
        return (is_wsl(target) && same_distro(&source_distro)).then(|| wsl_unc(&source_distro, cwd));
    }
    if let Some((share_distro, linux)) = wsl_share(cwd) {
        return (is_wsl(target) && same_distro(share_distro)).then(|| wsl_unc(share_distro, &linux));
    }
    if is_drive_path(cwd) {
        return Some(cwd.to_string());
    }
    // A network share: cmd can't start in one, wsl.exe wouldn't know it.
    (cwd.starts_with("\\\\") && !is_cmd(target) && !is_wsl(target)).then(|| cwd.to_string())
}

/// PowerShell's location isn't its process's working directory, so it has
/// to say where it is: after the profile has run, the prompt function (the
/// user's own, or the default) is wrapped to also send OSC 9;9, like Windows
/// Terminal suggests. It runs first and keeps `$?` and `$LASTEXITCODE`.
/// No double quotes, so it passes through any command-line quoting.
pub const POWERSHELL_PROMPT_HOOK: &str = "$global:__wtpp_prompt = $function:prompt; \
     function global:prompt { \
     $p = if ($global:__wtpp_prompt) { (& $global:__wtpp_prompt) -join '' } else { 'PS> ' }; \
     $l = $executionContext.SessionState.Path.CurrentLocation; \
     if ($l.Provider.Name -eq 'FileSystem') { $e = [string][char]27; $e + ']9;9;' + $l.ProviderPath + $e + '\\' + $p } else { $p } }";

/// `args` for `powershell`/`pwsh` with the prompt hook added, when they're
/// arguments it can be added to: none that run a command or a script, or
/// that would change what `-NoExit -Command` means. `None` otherwise.
pub fn powershell_args_with_hook(program: &str, args: &[String]) -> Option<Vec<String>> {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    if !matches!(file.strip_suffix(".exe").unwrap_or(&file), "powershell" | "pwsh") {
        return None;
    }
    const PLAIN: [&str; 4] = ["-nologo", "-noprofile", "-sta", "-mta"];
    if !args.iter().all(|a| PLAIN.contains(&a.to_ascii_lowercase().as_str())) {
        return None;
    }
    let mut args = args.to_vec();
    args.extend(["-NoExit".to_string(), "-Command".to_string(), POWERSHELL_PROMPT_HOOK.to_string()]);
    Some(args)
}

/// bash's PROMPT_COMMAND: report the directory at every prompt. Only from
/// the environment, so a `.bashrc` that sets its own wins (and one that adds
/// to it keeps this); other shells ignore it.
pub const BASH_PROMPT_COMMAND: &str = r#"printf '\033]9;9;%s\033\\' "$PWD""#;

/// Environment for a WSL profile so its bash reports the directory: our
/// PROMPT_COMMAND, passed into WSL by WSLENV (added to what's there). Nothing
/// if the profile sets PROMPT_COMMAND itself. `wslenv` is the WSLENV the
/// shell would otherwise get.
pub fn wsl_prompt_env(profile_sets_prompt_command: bool, wslenv: Option<&str>) -> Vec<(String, String)> {
    if profile_sets_prompt_command {
        return Vec::new();
    }
    let wslenv = wslenv.unwrap_or("").trim_matches(':');
    let listed = wslenv.split(':').any(|v| v.split('/').next().is_some_and(|name| name.eq_ignore_ascii_case("PROMPT_COMMAND")));
    let wslenv = match (listed, wslenv.is_empty()) {
        (true, _) => wslenv.to_string(),
        (false, true) => "PROMPT_COMMAND/u".to_string(),
        (false, false) => format!("{wslenv}:PROMPT_COMMAND/u"),
    };
    vec![("PROMPT_COMMAND".into(), BASH_PROMPT_COMMAND.into()), ("WSLENV".into(), wslenv)]
}

/// For tests and logs: a command line with the hook, as it would be run.
pub fn powershell_commandline_with_hook(program: &str, args: &[String]) -> Option<String> {
    powershell_args_with_hook(program, args).map(|args| join_commandline(program, &args))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEBIAN: &str = "wsl.exe ~ -d Debian";
    const KALI: &str = "wsl.exe ~ -d kali-linux";
    const CMD: &str = r"C:\Windows\system32\cmd.exe";
    const PS: &str = "powershell.exe -NoLogo";

    fn from(cwd: &str, source: &str, target: &str) -> Option<String> {
        starting_directory_from(cwd, source, target)
    }

    #[test]
    fn windows_directories() {
        for target in [CMD, PS, DEBIAN, r#""C:\Program Files\Git\bin\bash.exe" -i -l"#] {
            assert_eq!(from(r"C:\Users\me", CMD, target), Some(r"C:\Users\me".into()), "{target}");
            assert_eq!(from(r"D:\", PS, target), Some(r"D:\".into()), "{target}");
        }
        // A network share: not for cmd, nor WSL.
        assert_eq!(from(r"\\server\share\dir", PS, PS), Some(r"\\server\share\dir".into()));
        assert_eq!(from(r"\\server\share\dir", PS, CMD), None);
        assert_eq!(from(r"\\server\share\dir", PS, DEBIAN), None);
    }

    #[test]
    fn from_wsl() {
        // A Windows drive, as WSL mounts it: for anything.
        assert_eq!(from("/mnt/c/Users/me", DEBIAN, CMD), Some(r"C:\Users\me".into()));
        assert_eq!(from("/mnt/d", DEBIAN, PS), Some(r"D:\".into()));
        assert_eq!(from("/mnt/d/", DEBIAN, KALI), Some(r"D:\".into()));
        assert_eq!(from("/mnt/c/my dir/", DEBIAN, DEBIAN), Some(r"C:\my dir".into()));
        // Not a drive.
        assert_eq!(from("/mnt/wsl/x", DEBIAN, CMD), None);
        assert_eq!(from("/mnt", DEBIAN, CMD), None);
        // Only in Linux: the same distro, as a share wsl.exe starts in.
        assert_eq!(from("/home/me/src", DEBIAN, DEBIAN), Some(r"\\wsl$\Debian\home\me\src".into()));
        assert_eq!(from("/home/me/my dir", DEBIAN, "wsl.exe -d debian"), Some(r"\\wsl$\Debian\home\me\my dir".into()));
        assert_eq!(from("/", DEBIAN, DEBIAN), Some(r"\\wsl$\Debian".into()));
        assert_eq!(from("/home/me", DEBIAN, KALI), None);
        assert_eq!(from("/home/me", DEBIAN, CMD), None);
        assert_eq!(from("/home/me", DEBIAN, PS), None);
        // The default distro has no name to go by.
        assert_eq!(from("/home/me", "wsl.exe", "wsl.exe"), None);
    }

    #[test]
    fn wsl_shares() {
        // As `wslpath -w` reports it (Windows Terminal's snippet).
        assert_eq!(from(r"\\wsl.localhost\Debian\home\me", DEBIAN, DEBIAN), Some(r"\\wsl$\Debian\home\me".into()));
        assert_eq!(from(r"\\wsl$\Debian\home\me", PS, DEBIAN), Some(r"\\wsl$\Debian\home\me".into()));
        assert_eq!(from(r"\\WSL$\Debian", PS, DEBIAN), Some(r"\\wsl$\Debian".into()));
        assert_eq!(from(r"\\wsl$\Debian\home", PS, KALI), None);
        assert_eq!(from(r"\\wsl$\Debian\home", PS, PS), None);
        assert_eq!(from(r"\\wsl$\Debian\home", PS, CMD), None);
    }

    #[test]
    fn unix_machines() {
        assert_eq!(from("/home/me/src", "/bin/bash -l", "/usr/bin/zsh -l"), Some("/home/me/src".into()));
        assert_eq!(from("/", "/bin/bash -l", "/bin/bash -l"), Some("/".into()));
        assert_eq!(from("", "/bin/bash -l", "/bin/bash -l"), None);
        assert_eq!(from("relative", "/bin/bash -l", "/bin/bash -l"), None);
    }

    #[test]
    fn powershell_hook_only_where_it_fits() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let with = powershell_args_with_hook("powershell.exe", &args(&["-NoLogo"])).unwrap();
        assert_eq!(with[..3], args(&["-NoLogo", "-NoExit", "-Command"]));
        assert_eq!(with[3], POWERSHELL_PROMPT_HOOK);
        assert!(powershell_args_with_hook(r"C:\Program Files\PowerShell\7\pwsh.exe", &[]).is_some());
        assert!(powershell_args_with_hook("PWSH", &args(&["-nologo", "-NoProfile"])).is_some());
        for a in [&["-Command", "x"][..], &["-c", "x"], &["-File", "x.ps1"], &["-NoExit"], &["-EncodedCommand", "x"], &["-ExecutionPolicy", "Bypass"], &["x.ps1"], &["-com", "x"]] {
            assert_eq!(powershell_args_with_hook("powershell.exe", &args(a)), None, "{a:?}");
        }
        assert_eq!(powershell_args_with_hook("cmd.exe", &[]), None);
        assert_eq!(powershell_args_with_hook("/bin/bash", &[]), None);
        // It survives a round trip through a command line as one argument.
        let line = powershell_commandline_with_hook("powershell.exe", &args(&["-NoLogo"])).unwrap();
        assert_eq!(split_commandline(&line).1, with);
        assert!(!POWERSHELL_PROMPT_HOOK.contains('"'));
    }

    #[test]
    fn wsl_env() {
        let env = |sets, wslenv| wsl_prompt_env(sets, wslenv);
        assert_eq!(env(false, None)[1], ("WSLENV".into(), "PROMPT_COMMAND/u".into()));
        assert_eq!(env(false, None)[0], ("PROMPT_COMMAND".into(), BASH_PROMPT_COMMAND.into()));
        assert_eq!(env(false, Some("USERPROFILE/p:FOO"))[1].1, "USERPROFILE/p:FOO:PROMPT_COMMAND/u");
        assert_eq!(env(false, Some("FOO:"))[1].1, "FOO:PROMPT_COMMAND/u");
        assert_eq!(env(false, Some("PROMPT_COMMAND/u:FOO"))[1].1, "PROMPT_COMMAND/u:FOO");
        assert_eq!(env(false, Some("prompt_command"))[1].1, "prompt_command");
        assert!(env(true, Some("FOO")).is_empty());
        assert_eq!(BASH_PROMPT_COMMAND, r#"printf '\033]9;9;%s\033\\' "$PWD""#);
    }
}
