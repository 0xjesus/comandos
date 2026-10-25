//! Lista cerrada: el puente puede consultar y engancharse, nunca crear o matar sesiones.
use crate::dash::native::tmux::Tmux;
use std::path::PathBuf;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxTarget {
    User,
    Private(PathBuf),
    /// Private interaction fixture with an explicit scope launcher.
    PrivateScoped(PathBuf, PathBuf),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachCommand {
    Version,
    HasSession(String),
    ListSessionNames,
    /// `ignore_size` only applies with `active_pane` (both are `attach -f` flags of
    /// tmux 3.2): a phone cropping one pane of a desktop split never sizes the window.
    Attach {
        session: String,
        active_pane: bool,
        ignore_size: bool,
    },
}
impl AttachCommand {
    pub fn args(&self) -> Vec<String> {
        match self {
            Self::Version => vec!["-V".into()],
            Self::HasSession(s) => vec!["has-session".into(), "-t".into(), format!("={s}")],
            Self::ListSessionNames => vec![
                "list-sessions".into(),
                "-F".into(),
                "#{session_name}".into(),
            ],
            Self::Attach {
                session,
                active_pane,
                ignore_size,
            } => {
                let mut args = vec!["attach".into()];
                if *active_pane {
                    let flags = if *ignore_size { "active-pane,ignore-size" } else { "active-pane" };
                    args.extend(["-f".into(), flags.into()]);
                }
                args.extend(["-t".into(), format!("={session}")]);
                args
            }
        }
    }
}
impl TmuxTarget {
    pub fn prefix(&self) -> Vec<std::ffi::OsString> {
        match self {
            Self::User => Vec::new(),
            Self::Private(dir) | Self::PrivateScoped(dir, _) => Tmux::private(dir).program.prefix,
        }
    }
    pub(crate) fn probe(&self, command: &AttachCommand) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("tmux");
        cmd.args(self.prefix())
            .args(command.args())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .kill_on_drop(true);
        if let Self::Private(dir) | Self::PrivateScoped(dir, _) = self {
            cmd.env_clear()
                .env("HOME", dir.join("home"))
                .env("PATH", "/usr/bin:/bin")
                .env("SHELL", "/bin/sh");
        }
        cmd
    }
    pub(crate) fn pty_command(
        &self,
        command: Option<&AttachCommand>,
    ) -> std::io::Result<pty_process::Command> {
        let (program, args) = if let Some(command) = command {
            let mut args = self.prefix();
            args.extend(command.args().into_iter().map(std::ffi::OsString::from));
            (std::ffi::OsString::from("tmux"), args)
        } else {
            let shell = if matches!(self, Self::User) {
                std::env::var_os("SHELL")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "/bin/sh".into())
            } else {
                "/bin/sh".into()
            };
            (shell, Vec::new())
        };
        let scope = match self {
            Self::User => Some(
                crate::dash::native::quick::find_scope(std::env::var_os("PATH").as_deref())
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "terminal requires user scope launcher",
                        )
                    })?,
            ),
            Self::PrivateScoped(_, path) => Some(crate::dash::native::quick::scope_program(path)),
            Self::Private(_) => None,
        };
        let mut cmd = if let Some(scope) = scope {
            pty_process::Command::new(scope.path)
                .args(scope.prefix)
                .arg("--")
                .arg(program)
                .args(args)
        } else {
            pty_process::Command::new(program).args(args)
        };
        if let Self::Private(dir) | Self::PrivateScoped(dir, _) = self {
            cmd = cmd
                .env_clear()
                .env("HOME", dir.join("home"))
                .env("PATH", "/usr/bin:/bin")
                .env("SHELL", "/bin/sh");
        }
        Ok(cmd
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .kill_on_drop(true))
    }
}
pub fn valid_session(name: &str) -> bool {
    (1..=80).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
pub fn supports_active_pane(version: &str) -> bool {
    let parts: Vec<u32> = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(2)
        .filter_map(|s| s.parse().ok())
        .collect();
    matches!(parts.as_slice(), [major,minor] if *major>3 || (*major==3 && *minor>=2))
}
