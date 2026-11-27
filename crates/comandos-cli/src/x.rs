//! Project-session CLI with an injected tmux program and native config/search.
use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, Write},
    os::unix::{
        fs::MetadataExt,
        process::{CommandExt, ExitStatusExt},
    },
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

/// Every tmux operation, including kill and final attachment, uses this boundary.
pub trait Tmux {
    fn call(&self, args: &[&str], quiet_stderr: bool) -> i32;
    fn finish(&self, args: &[&str]) -> i32;
}

pub struct TmuxProgram {
    pub program: PathBuf,
    pub prefix: Vec<OsString>,
}
impl TmuxProgram {
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.prefix).args(args);
        command
    }
}
impl Tmux for TmuxProgram {
    fn call(&self, args: &[&str], quiet_stderr: bool) -> i32 {
        let mut command = self.command(args);
        if quiet_stderr {
            command.stderr(Stdio::null());
        }
        match command.status() {
            Ok(status) => status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)),
            Err(error) => {
                if !quiet_stderr {
                    eprintln!("tmux: {error}");
                }
                if error.kind() == io::ErrorKind::NotFound {
                    127
                } else {
                    126
                }
            }
        }
    }
    fn finish(&self, args: &[&str]) -> i32 {
        let error = self.command(args).exec();
        eprintln!("tmux: {error}");
        if error.kind() == io::ErrorKind::NotFound {
            127
        } else {
            126
        }
    }
}

pub struct Context {
    pub home: PathBuf,
    pub cwd: PathBuf,
    pub agent: Option<String>,
    pub in_tmux: bool,
}
impl Context {
    fn from_env() -> io::Result<Self> {
        let physical = env::current_dir()?;
        // Bash's `cd; pwd` preserves a valid logical PWD, including symlinks.
        let logical = env::var_os("PWD")
            .map(PathBuf::from)
            .filter(|path| {
                path.is_absolute()
                    && fs::metadata(path)
                        .ok()
                        .zip(fs::metadata(&physical).ok())
                        .is_some_and(|(a, b)| a.dev() == b.dev() && a.ino() == b.ino())
            })
            .unwrap_or(physical);
        Ok(Self {
            home: env::var_os("HOME").map(PathBuf::from).unwrap_or_default(),
            cwd: logical,
            agent: env::var("CCX_AGENT").ok(),
            in_tmux: env::var_os("TMUX").is_some_and(|value| !value.is_empty()),
        })
    }
}

fn conf_get(config: &str, wanted: &str) -> String {
    let raw = config
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            (key.trim_end() == wanted).then(|| value.trim().to_owned())
        })
        .next_back()
        .unwrap_or_default();
    if raw.len() >= 2
        && ((raw.starts_with('"') && raw.ends_with('"'))
            || (raw.starts_with('\'') && raw.ends_with('\'')))
    {
        raw[1..raw.len() - 1].to_owned()
    } else {
        raw
    }
}

fn launch(agent: &str, config: &str) -> String {
    let key = agent
        .chars()
        .map(|c| match c {
            'a'..='z' => c.to_ascii_uppercase(),
            '-' | '.' => '_',
            _ => c,
        })
        .collect::<String>();
    let custom = conf_get(config, &format!("AGENT_LAUNCH_{key}"));
    if !custom.is_empty() {
        return custom;
    }
    match agent {
        "claude" => "claude --continue 2>/dev/null || claude",
        "codex" => "codex resume --last 2>/dev/null || codex",
        "grok" => "grok --continue 2>/dev/null || grok",
        "opencode" => "opencode --continue 2>/dev/null || opencode",
        "agy" => "agy --continue 2>/dev/null || agy",
        "omp" => "omp --continue 2>/dev/null || omp",
        "gemini" => "gemini",
        custom => custom,
    }
    .to_owned()
}

fn logical_absolute(cwd: &Path, input: &Path) -> PathBuf {
    let path = if input.is_absolute() {
        input.to_owned()
    } else {
        cwd.join(input)
    };
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other.as_os_str()),
        }
    }
    clean
}

fn directories(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    // GNU find -P does not traverse a symlink even at its starting point.
    // Direct requested paths are resolved separately and retain Bash's behavior.
    if !fs::symlink_metadata(root).is_ok_and(|metadata| metadata.is_dir()) {
        return paths;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return paths;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let parent = entry.path();
        if !parent.to_string_lossy().contains("/node_modules/") {
            paths.push(parent.clone());
        }
        if let Ok(children) = fs::read_dir(&parent) {
            for child in children.flatten() {
                let path = child.path();
                if child.file_type().is_ok_and(|kind| kind.is_dir())
                    && !path.to_string_lossy().contains("/node_modules/")
                {
                    paths.push(path);
                }
            }
        }
    }
    paths
}

#[path = "x/glob.rs"]
mod glob;

fn glob_pattern(pattern: &str) -> Option<glob::Pattern> {
    glob::Pattern::compile(pattern)
}

fn name_matches(path: &Path, pattern: Option<&glob::Pattern>) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .zip(pattern)
        .is_some_and(|(name, pattern)| pattern.is_match(name))
}

pub fn run(
    args: &[String],
    ctx: &Context,
    tmux: &impl Tmux,
    out: &mut impl Write,
) -> io::Result<i32> {
    let config =
        fs::read_to_string(ctx.home.join(".claude/hooks/cc-notify.conf")).unwrap_or_default();
    let (flag_agent, args) = if args.first().is_some_and(|arg| arg == "-a") && args.len() >= 2 {
        (Some(args[1].as_str()), &args[2..])
    } else {
        (None, args)
    };
    let configured = conf_get(&config, "AGENT_DEFAULT");
    let agent = flag_agent
        .or(ctx.agent.as_deref())
        .filter(|value| !value.is_empty())
        .unwrap_or(if configured.is_empty() {
            "claude"
        } else {
            &configured
        });
    let codebase = ctx.home.join("codebase");
    if args.is_empty() {
        writeln!(out, "Sesiones activas:")?;
        out.flush()?;
        if tmux.call(
            &[
                "list-sessions",
                "-F",
                "  #{session_name}#{?session_attached,  [conectada],}",
            ],
            true,
        ) != 0
        {
            writeln!(out, "  (ninguna)")?;
        }
        writeln!(
            out,
            "\nUso: ccx <nombre-de-proyecto>   (busca en ~/codebase)"
        )?;
        return Ok(0);
    }
    if args[0] == "kill" && args.len() >= 2 {
        let code = tmux.call(&["kill-session", "-t", &format!("={}", args[1])], false);
        if code == 0 {
            writeln!(out, "Sesion '{}' eliminada", args[1])?;
        }
        return Ok(code);
    }
    let target = &args[0];
    let input = Path::new(target);
    let input = if input.is_absolute() {
        input.to_owned()
    } else {
        ctx.cwd.join(input)
    };
    let directory = if input.is_dir() {
        logical_absolute(&ctx.cwd, &input)
    } else {
        let candidates = directories(&codebase);
        let exact_pattern = glob_pattern(target);
        if let Some(exact) = candidates
            .iter()
            .find(|path| name_matches(path, exact_pattern.as_ref()))
        {
            exact.clone()
        } else {
            let pattern = glob_pattern(&format!("*{target}*"));
            let matches = candidates
                .into_iter()
                .filter(|path| name_matches(path, pattern.as_ref()))
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [] => {
                    writeln!(out, "No encontre '{target}' en {}", codebase.display())?;
                    return Ok(1);
                }
                [one] => one.clone(),
                many => {
                    writeln!(out, "Varios candidatos, se mas especifico:")?;
                    for path in many {
                        writeln!(out, "  {}", path.display())?;
                    }
                    return Ok(1);
                }
            }
        }
    };
    let Some(directory_text) = directory.to_str() else {
        return Err(io::Error::other("ruta no UTF-8"));
    };
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("/")
        .replace(['.', ':'], "-");
    let session = format!("={name}");
    if tmux.call(&["has-session", "-t", &session], true) != 0 {
        let command = format!("{}; exec $SHELL", launch(agent, &config));
        tmux.call(
            &[
                "new-session",
                "-d",
                "-s",
                &name,
                "-n",
                "claude",
                "-c",
                directory_text,
                &command,
            ],
            false,
        );
        tmux.call(
            &[
                "new-window",
                "-d",
                "-t",
                &session,
                "-n",
                "shell",
                "-c",
                directory_text,
            ],
            false,
        );
        writeln!(
            out,
            "Sesion '{name}' creada en {directory_text} con {agent} (ventanas: agente + shell)"
        )?;
    }
    out.flush()?;
    Ok(tmux.finish(&[
        if ctx.in_tmux {
            "switch-client"
        } else {
            "attach"
        },
        "-t",
        &session,
    ]))
}

pub fn main(args: &[String]) -> i32 {
    let result = Context::from_env().and_then(|ctx| {
        run(
            args,
            &ctx,
            &TmuxProgram {
                program: "tmux".into(),
                prefix: vec![],
            },
            &mut io::stdout().lock(),
        )
    });
    result.unwrap_or_else(|error| {
        eprintln!("ccx: {error}");
        1
    })
}
