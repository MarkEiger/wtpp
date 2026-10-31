//! Files dropped onto a pane: their paths, quoted for the pane's shell.

use std::path::Path;

/// How a shell wants paths written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    Cmd,
    PowerShell,
    /// A Linux shell behind `wsl.exe`: Windows paths become `/mnt/c/...`.
    Wsl,
    /// Git Bash / MSYS2 (a Windows `bash.exe`): Windows paths become `/c/...`.
    Msys,
    Posix,
}

/// The shell kind for a profile's program and `source` (WT marks the WSL
/// profiles it generates with `Windows.Terminal.Wsl`).
pub fn shell_kind(program: &str, source: Option<&str>) -> ShellKind {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    match file.strip_suffix(".exe").unwrap_or(&file) {
        "cmd" => ShellKind::Cmd,
        "powershell" | "pwsh" => ShellKind::PowerShell,
        "wsl" => ShellKind::Wsl,
        _ if source == Some("Windows.Terminal.Wsl") => ShellKind::Wsl,
        "bash" | "sh" | "zsh" | "fish" if file.ends_with(".exe") => ShellKind::Msys,
        _ => ShellKind::Posix,
    }
}

/// The text to paste for dropped paths: each quoted for `kind`, separated
/// by spaces, and followed by one (so a second drop doesn't run into the
/// first, as in Terminal and iTerm); no newline, so nothing runs by itself.
pub fn dropped_text(paths: &[impl AsRef<Path>], kind: ShellKind) -> String {
    let quoted: Vec<String> = paths
        .iter()
        .map(|p| {
            let path = p.as_ref().to_string_lossy();
            match kind {
                ShellKind::Cmd | ShellKind::PowerShell => quote_windows(&path, kind == ShellKind::PowerShell),
                ShellKind::Wsl => quote_posix(&wsl_path(&path)),
                ShellKind::Msys => quote_posix(&msys_path(&path)),
                ShellKind::Posix => quote_posix(&path),
            }
        })
        .collect();
    if quoted.is_empty() { String::new() } else { quoted.join(" ") + " " }
}

/// Double-quote a Windows path when cmd or PowerShell would split or
/// interpret it. PowerShell expands `$` and backticks inside double quotes,
/// so those get a backtick escape. (Windows file names can't contain `"`.)
pub fn quote_windows(path: &str, powershell: bool) -> String {
    const SPECIAL: &str = "&()[]{}^=;!'+,`~$@#%<>|";
    if !path.is_empty() && !path.chars().any(|c| c.is_whitespace() || SPECIAL.contains(c)) {
        return path.to_string();
    }
    let mut out = String::with_capacity(path.len() + 2);
    out.push('"');
    for c in path.chars() {
        if powershell && matches!(c, '$' | '`') {
            out.push('`');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// POSIX shell quoting: plain paths stay as they are, anything else goes in
/// single quotes (with `'` written as `'\''`).
pub fn quote_posix(path: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c);
    if !path.is_empty() && path.chars().all(plain) {
        return path.to_string();
    }
    format!("'{}'", path.replace('\'', r"'\''"))
}

/// A Windows path as WSL sees it: `C:\Users\me` → `/mnt/c/Users/me`, and
/// `\\wsl$\Ubuntu\home\me` (or `\\wsl.localhost\...`) → `/home/me`.
pub fn wsl_path(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let lower = path.to_ascii_lowercase();
    for prefix in [r"\\wsl$\", r"\\wsl.localhost\"] {
        if lower.starts_with(prefix) {
            // Drop the distro name too.
            let rest = path[prefix.len()..].split_once('\\').map_or("", |(_, rest)| rest);
            return format!("/{}", rest.replace('\\', "/"));
        }
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = path[2..].replace('\\', "/");
        let rest = rest.trim_start_matches('/');
        return if rest.is_empty() { format!("/mnt/{drive}") } else { format!("/mnt/{drive}/{rest}") };
    }
    path.replace('\\', "/")
}

/// A Windows path as Git Bash / MSYS2 sees it: `C:\Users\me` → `/c/Users/me`.
pub fn msys_path(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = path[2..].replace('\\', "/");
        return format!("/{drive}/{}", rest.trim_start_matches('/'));
    }
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_kinds() {
        assert_eq!(shell_kind(r"C:\WINDOWS\system32\cmd.exe", None), ShellKind::Cmd);
        assert_eq!(shell_kind("powershell.exe", None), ShellKind::PowerShell);
        assert_eq!(shell_kind(r"C:\Program Files\PowerShell\7\pwsh.exe", None), ShellKind::PowerShell);
        assert_eq!(shell_kind("wsl.exe", None), ShellKind::Wsl);
        assert_eq!(shell_kind("ubuntu.exe", Some("Windows.Terminal.Wsl")), ShellKind::Wsl);
        assert_eq!(shell_kind("/bin/bash", None), ShellKind::Posix);
        assert_eq!(shell_kind(r"C:\Program Files\Git\bin\bash.exe", None), ShellKind::Msys);
        assert_eq!(shell_kind("bash", None), ShellKind::Posix);
    }

    #[test]
    fn windows_quoting() {
        assert_eq!(quote_windows(r"C:\Users\me\a.txt", false), r"C:\Users\me\a.txt");
        assert_eq!(quote_windows(r"C:\My Dir", false), r#""C:\My Dir""#);
        assert_eq!(quote_windows(r"C:\a&b", false), r#""C:\a&b""#);
        assert_eq!(quote_windows(r"C:\x (1)", true), r#""C:\x (1)""#);
        assert_eq!(quote_windows(r"C:\$x`y", true), r#""C:\`$x``y""#);
        assert_eq!(quote_windows(r"C:\$x", false), r#""C:\$x""#);
    }

    #[test]
    fn posix_quoting() {
        assert_eq!(quote_posix("/home/me/a.txt"), "/home/me/a.txt");
        assert_eq!(quote_posix("/home/me/My Dir"), "'/home/me/My Dir'");
        assert_eq!(quote_posix("/tmp/it's"), r"'/tmp/it'\''s'");
        assert_eq!(quote_posix("/tmp/$HOME"), "'/tmp/$HOME'");
        assert_eq!(quote_posix(r"C:\Users"), r"'C:\Users'");
        assert_eq!(quote_posix(""), "''");
    }

    #[test]
    fn wsl_paths() {
        assert_eq!(wsl_path(r"C:\Users\me\a b.txt"), "/mnt/c/Users/me/a b.txt");
        assert_eq!(wsl_path(r"D:\"), "/mnt/d");
        assert_eq!(wsl_path(r"\\?\E:\x"), "/mnt/e/x");
        assert_eq!(wsl_path(r"\\wsl$\Ubuntu\home\me"), "/home/me");
        assert_eq!(wsl_path(r"\\wsl.localhost\Ubuntu-22.04\etc\hosts"), "/etc/hosts");
        assert_eq!(wsl_path(r"\\WSL.LOCALHOST\Ubuntu"), "/");
        assert_eq!(wsl_path(r"\\server\share\f"), "//server/share/f");
    }

    #[test]
    fn msys_paths() {
        assert_eq!(msys_path(r"C:\Users\me\a b.txt"), "/c/Users/me/a b.txt");
        assert_eq!(msys_path(r"D:\"), "/d/");
        assert_eq!(msys_path(r"\\server\share"), "//server/share");
        assert_eq!(dropped_text(&[r"C:\My Dir"], ShellKind::Msys), "'/c/My Dir' ");
    }

    #[test]
    fn joins_paths() {
        let paths = [r"C:\My Dir", r"C:\a.txt"];
        assert_eq!(dropped_text(&paths, ShellKind::Cmd), r#""C:\My Dir" C:\a.txt "#);
        assert_eq!(dropped_text(&paths, ShellKind::Wsl), "'/mnt/c/My Dir' /mnt/c/a.txt ");
        assert_eq!(dropped_text(&["/a b", "/c"], ShellKind::Posix), "'/a b' /c ");
        let none: [&str; 0] = [];
        assert_eq!(dropped_text(&none, ShellKind::Posix), "");
    }
}
