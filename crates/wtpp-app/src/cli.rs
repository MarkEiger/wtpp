//! Command-line arguments, following Windows Terminal's `wt.exe`:
//! `wtpp [--pos X,Y] [new-tab] [-p PROFILE] [-d DIR] [--] [COMMANDLINE...]`.

use std::path::PathBuf;

#[derive(Debug, Default, PartialEq)]
pub struct Args {
    /// Profile name (or GUID) for the first tab.
    pub profile: Option<String>,
    pub starting_directory: Option<PathBuf>,
    /// Run this instead of the profile's command line.
    pub commandline: Option<String>,
    /// Where the window opens (`--pos X,Y`, screen pixels).
    pub position: Option<(i32, i32)>,
}

impl Args {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
        let mut out = Args::default();
        let mut args = args.into_iter().peekable();
        // Window options come before the subcommand, as in wt.exe.
        if args.peek().map(String::as_str) == Some("--pos") {
            args.next();
            let pos = args.next().ok_or("--pos needs X,Y")?;
            let parsed = pos.split_once(',').and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)));
            out.position = Some(parsed.ok_or(format!("--pos needs X,Y, not {pos:?}"))?);
        }
        if matches!(args.peek().map(String::as_str), Some("new-tab" | "nt")) {
            args.next();
        }
        let mut rest = Vec::new();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-p" | "--profile" => out.profile = Some(args.next().ok_or("-p needs a profile name")?),
                "-d" | "--startingDirectory" => {
                    let dir = args.next().ok_or("-d needs a directory")?;
                    // Explorer passes "C:\dir\" quoted, which arrives with a
                    // trailing quote on some paths.
                    out.starting_directory = Some(PathBuf::from(dir.trim_end_matches('"')));
                }
                "--" => {
                    rest.extend(args.by_ref());
                }
                a if a.starts_with('-') && rest.is_empty() => return Err(format!("unknown option {a}")),
                _ => {
                    rest.push(arg);
                    rest.extend(args.by_ref());
                }
            }
        }
        if !rest.is_empty() {
            out.commandline = Some(wtpp_config::settings::join_commandline(&rest[0], &rest[1..]));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn wt_style_arguments() {
        assert_eq!(parse(&[]).unwrap(), Args::default());
        let a = parse(&["-d", r"C:\code", "-p", "Debian"]).unwrap();
        assert_eq!(a.profile.as_deref(), Some("Debian"));
        assert_eq!(a.starting_directory, Some(PathBuf::from(r"C:\code")));
        let a = parse(&["new-tab", "-p", "cmd", "--", "ping", "-t", "localhost"]).unwrap();
        assert_eq!(a.commandline.as_deref(), Some("ping -t localhost"));
        assert_eq!(parse(&["htop"]).unwrap().commandline.as_deref(), Some("htop"));
        assert!(parse(&["--bogus"]).is_err());
        let a = parse(&["--pos", "2600,-100", "-p", "cmd"]).unwrap();
        assert_eq!(a.position, Some((2600, -100)));
        assert_eq!(a.profile.as_deref(), Some("cmd"));
        assert!(parse(&["--pos", "12"]).is_err());
        assert!(parse(&["-p"]).is_err());
    }
}
