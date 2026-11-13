//! Opening a new pane in another pane's current directory: which directory
//! that is for the new pane's shell, and having shells report it.

use crate::settings::{is_wsl_program, join_commandline, split_commandline_windows};

/// The distro a WSL command line runs (`-d`), or `None` for the default one
/// or a command line that isn't WSL's.
fn distro(commandline: &str) -> Option<String> {
    let (program, args) = split_commandline_windows(commandline);
    if !is_wsl_program(&program) {
        return None;
    }
    let i = args.iter().position(|a| a == "-d" || a == "--distribution")?;
    args.get(i + 1).cloned()
}

fn is_wsl(commandline: &str) -> bool {
    is_wsl_program(&split_commandline_windows(commandline).0)
}

fn is_cmd(commandline: &str) -> bool {
    let (program, _) = split_commandline_windows(commandline);
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
    powershell_args_with_command(program, args, POWERSHELL_PROMPT_HOOK)
}

/// [`powershell_args_with_hook`] with any `command` (that has no double
/// quotes) in place of the hook.
pub fn powershell_args_with_command(program: &str, args: &[String], command: &str) -> Option<Vec<String>> {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    if !matches!(file.strip_suffix(".exe").unwrap_or(&file), "powershell" | "pwsh") {
        return None;
    }
    const PLAIN: [&str; 4] = ["-nologo", "-noprofile", "-sta", "-mta"];
    if !args.iter().all(|a| PLAIN.contains(&a.to_ascii_lowercase().as_str())) {
        return None;
    }
    let mut args = args.to_vec();
    args.extend(["-NoExit".to_string(), "-Command".to_string(), command.to_string()]);
    Some(args)
}

/// bash's PROMPT_COMMAND: report the directory at every prompt. Only from
/// the environment, so a `.bashrc` that sets its own wins (and one that adds
/// to it keeps this); other shells ignore it.
///
/// Two reports: OSC 9;9 without symbolic links, which Windows can start a
/// program in (`\\wsl$\Debian\usr\bin`, not `…\bin`, a link), then OSC 7 with
/// the name the prompt shows (`%` encoded). A pane opened from it starts in
/// the first and gets the second as WTPP_CD: at its first prompt it changes
/// to that name, if it's the same directory, quietly, and forgets it.
pub const BASH_PROMPT_COMMAND: &str = concat!(
    r#"[ -n "$WTPP_CD" ] && { [ "$WTPP_CD" -ef . ] && cd -- "$WTPP_CD"; unset WTPP_CD; } >/dev/null 2>&1; "#,
    r#"printf '\033]9;9;%s\033\\\033]7;file://%s\033\\' "$(pwd -P 2>/dev/null || printf %s "$PWD")" "${PWD//\%/%25}""#,
);

/// bash on Linux and macOS: OSC 7 with the name the prompt shows (wt++ can
/// read the directory without symbolic links from the process itself).
pub const UNIX_BASH_PROMPT_COMMAND: &str = r#"printf '\033]7;file://%s\033\\' "${PWD//\%/%25}""#;

/// bash's PROMPT_COMMAND for what's turned on: reporting the directory
/// (`cwd`; [`BASH_PROMPT_COMMAND`] in WSL, else [`UNIX_BASH_PROMPT_COMMAND`])
/// and the ssh function ([`crate::ssh::BASH_SSH_INIT`], before the report)
/// with the run of a given ssh ([`crate::ssh::BASH_SSH_RUN`], after it, so
/// the directory is known while it runs). `None` for neither.
pub fn bash_prompt_command(wsl: bool, cwd: bool, ssh: bool) -> Option<String> {
    let report = if wsl { BASH_PROMPT_COMMAND } else { UNIX_BASH_PROMPT_COMMAND };
    let parts = [ssh.then_some(crate::ssh::BASH_SSH_INIT), cwd.then_some(report), ssh.then_some(crate::ssh::BASH_SSH_RUN)];
    let parts: Vec<&str> = parts.into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// zsh's startup files in wt++'s own ZDOTDIR, as VS Code does it: each runs
/// the user's own from where zsh would have read it (their ZDOTDIR, or HOME;
/// their .zshenv may move it), with ZDOTDIR set to that while it runs. The
/// last file read puts ZDOTDIR back as it was, so nothing started later sees
/// wt++'s. The .zshrc adds a precmd function reporting the directory (OSC 7)
/// if WTPP_REPORT_CWD says to, and with a WTPP_TOKEN, the ssh function (the
/// last file, see [`crate::ssh::ZSH_SSH`]).
pub const ZSH_FILES: [(&str, &str); 5] = [
    (
        ".zshenv",
        r#"# wt++ shell integration (see wt++'s README). Runs your own startup files.
__wtpp_user() {
  builtin typeset -g WTPP_ZDOTDIR=$ZDOTDIR
  ZDOTDIR=${WTPP_USER_ZDOTDIR:-$HOME}
  if [[ -f $ZDOTDIR/$1 && $ZDOTDIR != $WTPP_ZDOTDIR ]]; then
    builtin source -- "$ZDOTDIR/$1"
  fi
  builtin typeset -g WTPP_USER_ZDOTDIR=$ZDOTDIR
  ZDOTDIR=$WTPP_ZDOTDIR
}
__wtpp_restore() {
  if [[ -n $WTPP_ZDOTDIR_WAS_SET || $WTPP_USER_ZDOTDIR != $HOME ]]; then
    ZDOTDIR=$WTPP_USER_ZDOTDIR
  else
    builtin unset ZDOTDIR
  fi
  builtin unset WTPP_USER_ZDOTDIR WTPP_ZDOTDIR WTPP_ZDOTDIR_WAS_SET
  builtin unfunction __wtpp_user __wtpp_restore
}
__wtpp_user .zshenv
[[ -o interactive || -o login ]] || __wtpp_restore
"#,
    ),
    (
        ".zprofile",
        r#"# wt++ shell integration (see wt++'s README). Runs your own startup files.
__wtpp_user .zprofile
"#,
    ),
    (
        ".zshrc",
        r#"# wt++ shell integration (see wt++'s README). Runs your own startup files.
__wtpp_user .zshrc
__wtpp_report_cwd() {
  builtin emulate -L zsh
  builtin printf '\e]7;file://%s\e\\' "${PWD//\%/%25}"
}
builtin typeset -ga precmd_functions
[[ -n $WTPP_REPORT_CWD ]] && precmd_functions+=(__wtpp_report_cwd)
builtin unset WTPP_REPORT_CWD
[[ -n $WTPP_TOKEN ]] && builtin source -- "$ZDOTDIR/wtpp-ssh.zsh"
[[ -o login ]] || __wtpp_restore
"#,
    ),
    (
        ".zlogin",
        r#"# wt++ shell integration (see wt++'s README). Runs your own startup files.
__wtpp_user .zlogin
__wtpp_restore
"#,
    ),
    ("wtpp-ssh.zsh", crate::ssh::ZSH_SSH),
];

/// Write [`ZSH_FILES`] to `dir`, where they aren't already as they should be.
pub fn write_zsh_integration(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, text) in ZSH_FILES {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
            std::fs::write(&path, text)?;
        }
    }
    Ok(())
}

/// Environment for a bash or zsh on Linux and macOS so it reports the
/// directory by the name it shows (with `cwd`) and has the ssh function
/// (with `ssh_env`, see [`crate::ssh::shell_env`], which it's given too).
/// `zsh_dir` holds [`ZSH_FILES`]; `var` reads the environment the shell
/// would get. Nothing for other programs, or where the user has set things
/// up themselves.
pub fn unix_prompt_env(
    program: &str,
    zsh_dir: Option<&str>,
    var: impl Fn(&str) -> Option<String>,
    cwd: bool,
    ssh_env: &[(String, String)],
) -> Vec<(String, String)> {
    let file = program.rsplit('/').next().unwrap_or(program);
    let mut env = match file {
        "bash" => {
            let Some(ours) = bash_prompt_command(false, cwd, !ssh_env.is_empty()) else { return Vec::new() };
            match var("PROMPT_COMMAND").filter(|p| !p.trim().is_empty()) {
                Some(theirs) if theirs.contains(&ours) => Vec::new(),
                Some(theirs) => vec![("PROMPT_COMMAND".into(), format!("{theirs}; {ours}"))],
                None => vec![("PROMPT_COMMAND".into(), ours)],
            }
        }
        "zsh" => {
            let Some(dir) = zsh_dir else { return Vec::new() };
            if !cwd && ssh_env.is_empty() {
                return Vec::new();
            }
            let theirs = var("ZDOTDIR").filter(|z| !z.is_empty());
            let mut env = Vec::new();
            // Started from a pane that has it already (wt++ in wt++).
            if theirs.as_deref() != Some(dir) {
                let home = var("HOME").unwrap_or_default();
                env.push(("ZDOTDIR".to_string(), dir.to_string()));
                env.push(("WTPP_USER_ZDOTDIR".to_string(), theirs.clone().unwrap_or(home)));
                if theirs.is_some() {
                    env.push(("WTPP_ZDOTDIR_WAS_SET".into(), "1".into()));
                }
            }
            if cwd {
                env.push(("WTPP_REPORT_CWD".into(), "1".into()));
            }
            env
        }
        _ => return Vec::new(),
    };
    env.extend(ssh_env.iter().cloned());
    env
}

/// Environment for a WSL profile so its bash reports the directory (with
/// `cwd`) and has the ssh function (with `ssh_env`): our PROMPT_COMMAND and
/// those, passed into WSL by WSLENV (added to what's there), and with
/// `logical_dir`, WTPP_CD (see [`BASH_PROMPT_COMMAND`]). Nothing if the
/// profile sets PROMPT_COMMAND itself. `wslenv` is the WSLENV the shell would
/// otherwise get.
pub fn wsl_prompt_env(
    profile_sets_prompt_command: bool,
    wslenv: Option<&str>,
    logical_dir: Option<&str>,
    cwd: bool,
    ssh_env: &[(String, String)],
) -> Vec<(String, String)> {
    let Some(prompt_command) = bash_prompt_command(true, cwd, !ssh_env.is_empty()).filter(|_| !profile_sets_prompt_command) else {
        return Vec::new();
    };
    let mut wslenv = wslenv.unwrap_or("").trim_matches(':').to_string();
    let mut env = vec![("PROMPT_COMMAND".to_string(), prompt_command)];
    if let Some(dir) = logical_dir.filter(|_| cwd) {
        env.push(("WTPP_CD".into(), dir.into()));
    }
    env.extend(ssh_env.iter().cloned());
    for (name, _) in &env {
        let listed = wslenv.split(':').any(|v| v.split('/').next().is_some_and(|n| n.eq_ignore_ascii_case(name)));
        if !listed {
            if !wslenv.is_empty() {
                wslenv.push(':');
            }
            wslenv.push_str(&format!("{name}/u"));
        }
    }
    env.push(("WSLENV".into(), wslenv));
    env
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
        assert_eq!(crate::settings::split_commandline(&line).1, with);
        assert!(!POWERSHELL_PROMPT_HOOK.contains('"'));
    }

    #[test]
    fn wsl_env() {
        let env = |sets, wslenv| wsl_prompt_env(sets, wslenv, None, true, &[]);
        assert_eq!(env(false, None)[1], ("WSLENV".into(), "PROMPT_COMMAND/u".into()));
        assert_eq!(env(false, None)[0], ("PROMPT_COMMAND".into(), BASH_PROMPT_COMMAND.into()));
        assert_eq!(env(false, Some("USERPROFILE/p:FOO"))[1].1, "USERPROFILE/p:FOO:PROMPT_COMMAND/u");
        assert_eq!(env(false, Some("FOO:"))[1].1, "FOO:PROMPT_COMMAND/u");
        assert_eq!(env(false, Some("PROMPT_COMMAND/u:FOO"))[1].1, "PROMPT_COMMAND/u:FOO");
        assert_eq!(env(false, Some("prompt_command"))[1].1, "prompt_command");
        assert!(env(true, Some("FOO")).is_empty());
        // The directory without symbolic links (`/bin` is one to `/usr/bin`):
        // Windows can't start a program in a Linux symbolic link.
        assert!(BASH_PROMPT_COMMAND.contains("pwd -P"));
        // With the name the prompt shows.
        let with = wsl_prompt_env(false, Some("FOO"), Some("/bin"), true, &[]);
        assert_eq!(with[1], ("WTPP_CD".into(), "/bin".into()));
        assert_eq!(with[2], ("WSLENV".into(), "FOO:PROMPT_COMMAND/u:WTPP_CD/u".into()));
        assert!(wsl_prompt_env(true, None, Some("/bin"), true, &[]).is_empty());
        // The ssh function, with or without the directory.
        let ssh = crate::ssh::shell_env("tok", Some((&["-p".to_string(), "22".to_string(), "h".to_string()], crate::ssh::Run::Function)));
        let with = wsl_prompt_env(false, None, Some("/bin"), true, &ssh);
        assert_eq!(with[0].1, bash_prompt_command(true, true, true).unwrap());
        assert_eq!(with[2], ("WTPP_TOKEN".into(), "tok".into()));
        assert_eq!(with[3], ("WTPP_SSH".into(), "-p\n22\nh".into()));
        assert_eq!(with[4], ("WSLENV".into(), "PROMPT_COMMAND/u:WTPP_CD/u:WTPP_TOKEN/u:WTPP_SSH/u".into()));
        let only_ssh = wsl_prompt_env(false, None, Some("/bin"), false, &ssh[..1]);
        assert_eq!(only_ssh[0].1, bash_prompt_command(true, false, true).unwrap());
        assert!(!only_ssh[0].1.contains("WTPP_CD") && !only_ssh[0].1.contains("]9;9;"));
        assert_eq!(only_ssh[2], ("WSLENV".into(), "PROMPT_COMMAND/u:WTPP_TOKEN/u".into()));
        assert!(wsl_prompt_env(false, None, None, false, &[]).is_empty());
        assert!(wsl_prompt_env(true, None, None, true, &ssh).is_empty());
    }

    #[test]
    fn prompt_commands() {
        assert_eq!(bash_prompt_command(false, true, false).as_deref(), Some(UNIX_BASH_PROMPT_COMMAND));
        assert_eq!(bash_prompt_command(true, true, false).as_deref(), Some(BASH_PROMPT_COMMAND));
        assert_eq!(bash_prompt_command(true, false, false), None);
        let both = bash_prompt_command(false, true, true).unwrap();
        assert_eq!(both, format!("{}; {UNIX_BASH_PROMPT_COMMAND}; {}", crate::ssh::BASH_SSH_INIT, crate::ssh::BASH_SSH_RUN));
        assert!(!bash_prompt_command(false, false, true).unwrap().contains("]7;"));
    }

    #[test]
    fn unix_shells() {
        let none = |_: &str| None;
        let cwd_only = |program: &str, zsh: Option<&str>, var: &dyn Fn(&str) -> Option<String>| unix_prompt_env(program, zsh, var, true, &[]);
        assert_eq!(cwd_only("/bin/bash", None, &none), [("PROMPT_COMMAND".to_string(), UNIX_BASH_PROMPT_COMMAND.to_string())]);
        let theirs = |n: &str| (n == "PROMPT_COMMAND").then(|| "history -a".to_string());
        assert_eq!(cwd_only("bash", None, &theirs)[0].1, format!("history -a; {UNIX_BASH_PROMPT_COMMAND}"));
        let ours = |n: &str| (n == "PROMPT_COMMAND").then(|| UNIX_BASH_PROMPT_COMMAND.to_string());
        assert!(cwd_only("bash", None, &ours).is_empty());
        let home = |n: &str| (n == "HOME").then(|| "/home/me".to_string());
        assert_eq!(
            cwd_only("/usr/bin/zsh", Some("/wt/zsh"), &home),
            [
                ("ZDOTDIR".to_string(), "/wt/zsh".to_string()),
                ("WTPP_USER_ZDOTDIR".to_string(), "/home/me".to_string()),
                ("WTPP_REPORT_CWD".to_string(), "1".to_string())
            ]
        );
        let set = |n: &str| match n {
            "ZDOTDIR" => Some("/home/me/.config/zsh".to_string()),
            "HOME" => Some("/home/me".to_string()),
            _ => None,
        };
        assert_eq!(cwd_only("zsh", Some("/wt/zsh"), &set)[1].1, "/home/me/.config/zsh");
        assert_eq!(cwd_only("zsh", Some("/wt/zsh"), &set)[2], ("WTPP_ZDOTDIR_WAS_SET".to_string(), "1".to_string()));
        assert!(cwd_only("zsh", None, &set).is_empty());
        assert_eq!(cwd_only("zsh", Some("/wt/zsh"), &|n| (n == "ZDOTDIR").then(|| "/wt/zsh".to_string())), [("WTPP_REPORT_CWD".to_string(), "1".to_string())]);
        assert!(cwd_only("fish", Some("/wt/zsh"), &home).is_empty());
        for (name, text) in ZSH_FILES {
            assert!(text.contains("__wtpp_user") || name == "wtpp-ssh.zsh");
        }
        // The ssh function alone, or with the directory reports; nothing for neither.
        let ssh = crate::ssh::shell_env("tok", None);
        let bash = unix_prompt_env("bash", None, none, false, &ssh);
        assert_eq!(bash, [("PROMPT_COMMAND".to_string(), bash_prompt_command(false, false, true).unwrap()), ssh[0].clone()]);
        let zsh = unix_prompt_env("zsh", Some("/wt/zsh"), home, false, &ssh);
        assert_eq!(zsh.last(), Some(&ssh[0]));
        assert!(!zsh.iter().any(|(k, _)| k == "WTPP_REPORT_CWD"));
        assert!(unix_prompt_env("bash", None, none, false, &[]).is_empty());
        assert!(unix_prompt_env("zsh", Some("/wt/zsh"), home, false, &[]).is_empty());
        assert!(unix_prompt_env("fish", None, none, true, &ssh).is_empty());
    }

    /// The hook in a real bash: the first prompt goes to WTPP_CD if it's the
    /// same directory, says nothing either way, and reports both names.
    #[cfg(unix)]
    #[test]
    fn bash_hook() {
        let Ok(bash) = std::process::Command::new("bash").arg("--version").output() else { return };
        assert!(bash.status.success());
        let base = std::env::temp_dir().join(format!("wtpp-hook-{}", std::process::id()));
        let real = base.join("real dir %41 ü");
        std::fs::create_dir_all(&real).unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let run = |cwd: &std::path::Path, wtpp_cd: &str| {
            let out = std::process::Command::new("bash")
                .args(["--norc", "--noprofile", "-c", "eval \"$PROMPT_COMMAND\"; echo; echo \"pwd=$(pwd) cd=${WTPP_CD-unset}\""])
                .current_dir(cwd)
                .env("PROMPT_COMMAND", BASH_PROMPT_COMMAND)
                .env("WTPP_CD", wtpp_cd)
                .env_remove("PWD")
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap()
        };
        let real_text = real.canonicalize().unwrap().to_string_lossy().into_owned();
        let link_text = link.to_string_lossy().into_owned();
        let out = run(&real, &link_text);
        let encoded = link_text.replace('%', "%25");
        assert_eq!(out, format!("\x1b]9;9;{real_text}\x1b\\\x1b]7;file://{encoded}\x1b\\\npwd={link_text} cd=unset\n"));
        // Not the same directory (or gone): stays, and quiet.
        let out = run(&real, "/");
        assert!(out.ends_with(&format!("\npwd={real_text} cd=unset\n")), "{out}");
        let out = run(&real, &base.join("gone").to_string_lossy());
        assert!(out.ends_with(&format!("\npwd={real_text} cd=unset\n")), "{out}");
        std::fs::remove_dir_all(&base).ok();
    }
}
