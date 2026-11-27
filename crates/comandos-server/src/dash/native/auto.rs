//! Modo auto: a hands-free voice chat with ONE Claude Code session, as if
//! Jesús sat at it (Opus 5.5, high effort, on the Claude account with the
//! most usage left). It works like any Claude Code session and also sees and
//! drives the other sessions with `comandos auto` (`comandos_cli::auto`).
//!
//! POST `/auto/session` finds that session or starts it, in its own folder
//! («Modo auto», which also names its tab on the desktop and the web), whose
//! `CLAUDE.md` says how to talk by voice and `.claude/settings.json` lets it
//! run `comandos auto` without asking. A session found running another model
//! (or on an account far emptier than the best one) is relaunched in its pane
//! before the answer, continuing its conversation when the account stays.
use super::{Answer, Entry, Key, Native, NativeRoute, Verb, reply};
use crate::{ReplyBody, Request};
use http::StatusCode;
use serde_json::{Map, Value, json};
use std::{path::Path, sync::Arc};


pub const ROUTES: &[Entry] = &[Entry {
    verb: Verb::Post,
    key: Key::Path("/auto/session"),
    route: NativeRoute::AutoSession,
}];

/// Under HOME. Its last part names the tab.
pub const DIR: &str = ".local/share/comandos/Modo auto";
const DIR_NAME: &str = "comandos/Modo auto";

/// What the orchestrator runs with.
pub const MODEL: &str = "claude-opus-5-5";
pub const EFFORT: &str = "high";

/// An account is left for a better one only when that one has at least this
/// many more points of usage left (a switch starts a new conversation).
const SWITCH_GAIN: f64 = 15.0;

/// Where a new session's launcher lives: its line waits typed at the prompt.
const LAUNCHER: &str = "/.local/share/comandos/launch/";

const SHELLS: [&str; 5] = ["bash", "zsh", "sh", "fish", "dash"];

pub const INSTRUCTIONS: &str = "\
# Modo auto de ComandOS

Eres la sesión de Claude Code con la que Jesús habla por voz desde su celular o desde Android Auto,
casi siempre manejando. Dice «OK ComandOS», habla (a veces mucho rato, con pausas) y lo que escribes
se le lee en voz alta. No puede mirar la pantalla.

## Cómo hablas

- Como en una llamada: frases naturales en español, al grano. Una a tres frases para lo simple; más
  solo si él pide detalle o una explicación.
- Nada de Markdown, listas, tablas, código, rutas, enlaces ni identificadores en lo que dices. Si hay
  varias cosas, dilas en una frase: «Hay dos: Relotto terminó y Threads te pregunta algo».
- Nunca digas en voz alta secretos: llaves, tokens, contraseñas, correos completos ni números de cuenta.
- El dictado se equivoca. Si un nombre suena raro, usa el más parecido; si de verdad no entendiste,
  pregunta en una frase corta.
- Si una tarea va a tardar, dilo en una frase al empezar y avisa al terminar.

## Qué haces

Eres una sesión normal de Claude Code: investigas, contestas, trabajas en sus proyectos (están en
~/codebase) y usas tus herramientas como siempre. Además ves y manejas las otras sesiones de ComandOS
con estos comandos, siempre con la herramienta Bash:

- `comandos auto sesiones`: quién te espera, quién trabaja y qué dijo cada una al final.
- `comandos auto leer SESIÓN [N]`: los últimos N turnos de una sesión (3 si no dices) y su pregunta
  abierta con sus opciones numeradas.
- `comandos auto enviar SESIÓN \"TEXTO\"`: escribe TEXTO en esa sesión y le da Enter.
- `comandos auto responder SESIÓN NÚMERO [\"TEXTO\"]`: contesta la pregunta abierta con esa opción
  (TEXTO solo para la opción de escribir otra respuesta; «Enviar» manda las casillas marcadas).

SESIÓN es el nombre que sale en `sesiones`, o su id entre corchetes si hay dos parecidos. No inventes
el estado de una sesión: consúltalo.

## Cuidado

- Cuando te dicte un mensaje para otra sesión, mándalo con sus palabras (corrige solo errores obvios del
  dictado) y dile en una frase a cuál lo mandaste. Si no queda claro a qué sesión o qué decir, pregunta
  antes.
- Antes de algo que borra, despliega, publica, envía mensajes a otras personas o gasta dinero, di qué
  vas a hacer y espera su «sí».

## Al empezar la llamada

Cuando te diga que empieza la llamada, corre `comandos auto sesiones` y resume en una o dos frases qué
te espera y qué está trabajando. Después espera lo que te pida.
";

pub const SETTINGS: &str = r#"{
  "permissions": {
    "allow": ["Bash(comandos auto:*)"]
  }
}
"#;

fn error(status: StatusCode, message: &str) -> Answer {
    reply(status, &json!({"ok": false, "error": message}))
}

/// Writes the folder, its instructions and its permissions (only what changed).
fn prepare(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir.join(".claude"))?;
    for (path, body) in [(dir.join("CLAUDE.md"), INSTRUCTIONS), (dir.join(".claude/settings.json"), SETTINGS)] {
        if std::fs::read_to_string(&path).ok().as_deref() != Some(body) {
            std::fs::write(&path, body)?;
        }
    }
    Ok(())
}

/// The orchestrator already open, from `list-panes -a` lines
/// (`session\tpane\tpath\tcommand\tdead`): a live pane in its folder, Claude
/// running first. The third value tells whether only a shell is there.
pub fn existing(listing: &str, dir: &str) -> Option<(String, String, bool)> {
    let panes: Vec<(String, String, bool)> = listing
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let [session, pane, path, command, dead] = f.as_slice() else {
                return None;
            };
            (*path == dir && *dead == "0").then(|| ((*session).to_owned(), (*pane).to_owned(), SHELLS.contains(command)))
        })
        .collect();
    panes.iter().find(|p| !p.2).or_else(|| panes.first()).cloned()
}

/// The last line with text on a screen.
fn last_line(screen: &str) -> &str {
    screen.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

/// Claude asks whether to trust the folder (it pre-approves `comandos auto`).
/// ComandOS made the folder and its settings: answers «Yes, I trust this
/// folder» when that question shows, waiting up to [tries] × 250 ms for it.
async fn accept_trust(native: &Native, pane: &str, tries: u32) {
    let tmux = &native.options().tmux;
    for _ in 0..tries {
        let screen = tmux.run(&["capture-pane", "-p", "-t", pane]).await.map(|o| o.stdout).unwrap_or_default();
        if screen.contains(DIR_NAME)
            && let Some(keys) = trust_keys(&screen)
        {
            for key in keys {
                let _ = tmux.run(&["send-keys", "-t", pane, key]).await;
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            }
            return;
        }
        if screen.contains("? for shortcuts") || screen.contains("bypass permissions") {
            return; // already in
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

/// The keys that pick «Yes, I trust this folder» on Claude's trust question
/// (its rows have no numbers: `❯ No, exit` / `Yes, I trust this folder`).
pub fn trust_keys(screen: &str) -> Option<Vec<&'static str>> {
    let rows: Vec<&str> = screen.lines().map(str::trim).collect();
    let yes = rows.iter().position(|l| l.trim_start_matches('❯').trim().starts_with("Yes, I trust this folder"))?;
    let cursor = rows.iter().position(|l| l.starts_with('❯'))?;
    let mut keys = vec![if yes > cursor { "Down" } else { "Up" }; yes.abs_diff(cursor)];
    keys.push("Enter");
    Some(keys)
}

/// Starts Claude in a pane that shows only a shell: presses Enter on the
/// launcher a new session leaves typed (waiting for it a moment), or runs
/// `claude` again when it was closed.
async fn start_claude(native: &Native, pane: &str, fresh: bool) {
    let tmux = &native.options().tmux;
    for _ in 0..(if fresh { 40 } else { 1 }) {
        let screen = tmux.run(&["capture-pane", "-p", "-t", pane]).await.map(|o| o.stdout).unwrap_or_default();
        if last_line(&screen).contains(LAUNCHER) {
            let _ = tmux.run(&["send-keys", "-t", pane, "Enter"]).await;
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    let _ = tmux.run(&["send-keys", "-t", pane, "-l", "claude"]).await;
    let _ = tmux.run(&["send-keys", "-t", pane, "Enter"]).await;
}

/// Usage left (0–100) per Claude account in the Consumos limit rows: 100
/// minus its fullest window (the week, the 5-hour session, and a week scoped
/// to Opus). Accounts without a reading are left out.
pub fn usage_left(rows: &[Map<String, Value>]) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    for row in rows {
        if row.get("provider").and_then(Value::as_str) != Some("claude") {
            continue;
        }
        let kind = row.get("kind").and_then(Value::as_str).unwrap_or("");
        let scope = row.get("scope").and_then(Value::as_str).unwrap_or("").to_lowercase();
        let counts = matches!(kind, "weekly_all" | "session") || (kind == "weekly_scoped" && scope.contains("opus"));
        let (Some(account), Some(used)) = (row.get("account").and_then(Value::as_str), row.get("percent").and_then(Value::as_f64))
        else {
            continue;
        };
        if !counts {
            continue;
        }
        let left = (100.0 - used).clamp(0.0, 100.0);
        match out.iter_mut().find(|(a, _)| a == account) {
            Some(slot) => slot.1 = slot.1.min(left),
            None => out.push((account.to_owned(), left)),
        }
    }
    out
}

/// The account with the most usage left (`main` on a tie or without data).
pub fn best_account(left: &[(String, f64)]) -> String {
    let mut best: Option<&(String, f64)> = None;
    for entry in left {
        let better = match best {
            None => true,
            Some(b) => entry.1 > b.1 || (entry.1 == b.1 && entry.0 == "main"),
        };
        if better {
            best = Some(entry);
        }
    }
    best.map_or_else(|| "main".to_owned(), |b| b.0.clone())
}

/// Whether the orchestrator on `current` should move to `best`.
pub fn should_switch(left: &[(String, f64)], current: &str, best: &str) -> bool {
    if current == best {
        return false;
    }
    let of = |a: &str| left.iter().find(|(x, _)| x == a).map(|(_, l)| *l);
    match (of(current), of(best)) {
        (Some(now), Some(top)) => top - now >= SWITCH_GAIN,
        (None, Some(_)) => true,
        _ => false,
    }
}

/// What a running Claude Code was started with: (model, effort, account).
/// The account comes from `CLAUDE_CONFIG_DIR` (`~/.claude-accounts/<alias>`;
/// none is `main`).
pub fn launched_with(argv: &[String], config_dir: Option<&str>) -> (String, String, String) {
    let flag = |name: &str| {
        argv.iter()
            .position(|a| a == name)
            .and_then(|i| argv.get(i + 1).cloned())
            .or_else(|| argv.iter().find_map(|a| a.strip_prefix(&format!("{name}=")).map(str::to_owned)))
            .unwrap_or_default()
    };
    let account = match config_dir.map(|d| d.trim_end_matches('/')) {
        Some(dir) if !dir.is_empty() => match dir.rsplit_once('/') {
            Some((parent, alias)) if parent.ends_with("/.claude-accounts") => alias.to_owned(),
            _ => "main".to_owned(),
        },
        _ => "main".to_owned(),
    };
    (flag("--model"), flag("--effort"), account)
}

/// The `claude` process under the pane's shell, read from `/proc`.
fn claude_under(shell_pid: u32) -> Option<(Vec<String>, Option<String>)> {
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // `pid (comm) state ppid …`; comm may hold spaces.
        let Some(after) = stat.rfind(')').map(|i| &stat[i + 1..]) else {
            continue;
        };
        let comm = stat.split_once('(').and_then(|(_, r)| r.rsplit_once(')')).map(|(c, _)| c).unwrap_or("");
        let ppid = after.split_whitespace().nth(1).and_then(|p| p.parse::<u32>().ok());
        if ppid != Some(shell_pid) || comm != "claude" {
            continue;
        }
        let argv: Vec<String> = std::fs::read(format!("/proc/{pid}/cmdline"))
            .ok()?
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        let dir = std::fs::read(format!("/proc/{pid}/environ")).ok().and_then(|env| {
            env.split(|b| *b == 0)
                .find_map(|kv| kv.strip_prefix(b"CLAUDE_CONFIG_DIR="))
                .map(|v| String::from_utf8_lossy(v).into_owned())
        });
        return Some((argv, dir));
    }
    None
}

async fn screen(native: &Native, pane: &str) -> String {
    native.options().tmux.run(&["capture-pane", "-p", "-t", pane]).await.map(|o| o.stdout).unwrap_or_default()
}

async fn pane_command(native: &Native, pane: &str) -> String {
    native
        .options()
        .tmux
        .run(&["display-message", "-p", "-t", pane, "#{pane_current_command}"])
        .await
        .map(|o| o.stdout.trim().to_owned())
        .unwrap_or_default()
}

/// The `claude` command line for the orchestrator on `account` (its
/// `CLAUDE_CONFIG_DIR` and nothing else from the environment).
async fn command_for(native: &Native, account: &str) -> Option<String> {
    let opts = native.options().clone();
    let cache = native.registry.clone();
    let account = account.to_owned();
    tokio::task::spawn_blocking(move || {
        let registry = super::states::gather::load_registry(&opts, &cache).ok()?;
        let ctx = comandos_runtime::launch_command::Ctx {
            registry,
            home: opts.home.clone(),
            cwd: opts.cwd.clone(),
            search_path: opts.search_path.clone(),
            proxy_port: 0,
            repo_root: opts.repo_root.clone()?,
        };
        comandos_runtime::launch_command::configuration_command(&ctx, "claude", "claude", MODEL, EFFORT, &account, "", &[], false).ok()
    })
    .await
    .ok()
    .flatten()
}

/// Closes the Claude Code in `pane` (only when it is idle: never in the
/// middle of an answer) and starts it again with [MODEL], [EFFORT] and
/// `account`, continuing the conversation when the account stays.
async fn relaunch(native: &Native, pane: &str, account: &str, same_account: bool) -> bool {
    if screen(native, pane).await.contains("esc to interrupt") {
        return false;
    }
    let Some(mut command) = command_for(native, account).await else {
        return false;
    };
    if same_account {
        command.push_str(" --continue");
    }
    let tmux = &native.options().tmux;
    for _ in 0..2 {
        let _ = tmux.run(&["send-keys", "-t", pane, "C-c"]).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    let mut out = false;
    for _ in 0..40 {
        if SHELLS.contains(&pane_command(native, pane).await.as_str()) {
            out = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    if !out {
        return false;
    }
    let _ = tmux.run(&["send-keys", "-t", pane, "-l", &command]).await;
    let _ = tmux.run(&["send-keys", "-t", pane, "Enter"]).await;
    accept_trust(native, pane, 60).await;
    true
}

/// Waits (up to ~20 s) until Claude Code shows its prompt in `pane`.
async fn ready(native: &Native, pane: &str) {
    for _ in 0..80 {
        let text = screen(native, pane).await;
        if text.contains("? for shortcuts") || text.contains("auto mode") || text.contains("bypass permissions") || text.contains("accept edits") {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

/// Puts the running orchestrator on [MODEL]/[EFFORT] and a good account.
/// Returns what it did, for the answer: "", "model" or "account".
async fn tune(native: &Native, pane: &str, left: &[(String, f64)], best: &str) -> &'static str {
    let tmux = &native.options().tmux;
    let Some(shell) = tmux
        .run(&["display-message", "-p", "-t", pane, "#{pane_pid}"])
        .await
        .ok()
        .and_then(|o| o.stdout.trim().parse::<u32>().ok())
    else {
        return "";
    };
    let Some((argv, dir)) = tokio::task::spawn_blocking(move || claude_under(shell)).await.ok().flatten() else {
        return "";
    };
    let (model, effort, account) = launched_with(&argv, dir.as_deref());
    let switch = should_switch(left, &account, best);
    let target = if switch { best } else { account.as_str() };
    if !switch && model == MODEL && effort == EFFORT {
        return "";
    }
    if !relaunch(native, pane, target, !switch).await {
        return "";
    }
    ready(native, pane).await;
    if switch { "account" } else { "model" }
}

pub async fn session(native: &Arc<Native>, _request: &Request) -> Answer {
    let opts = native.options();
    let dir = opts.home.join(DIR);
    let ready_dir = dir.clone();
    if tokio::task::spawn_blocking(move || prepare(&ready_dir)).await.ok().and_then(Result::ok).is_none() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "No pude preparar la carpeta del Modo auto");
    }
    let rows = native.limits.get(&native.refresh_deps()).rows;
    let left = usage_left(&rows);
    let best = best_account(&left);
    let dir_text = dir.to_string_lossy().into_owned();
    let tmux = &opts.tmux;
    let listing = tmux
        .run(&[
            "list-panes",
            "-a",
            "-F",
            "#{session_name}\t#{pane_id}\t#{pane_current_path}\t#{pane_current_command}\t#{pane_dead}",
        ])
        .await
        .map(|o| o.stdout)
        .unwrap_or_default();
    if let Some((session, pane, shell)) = existing(&listing, &dir_text) {
        let mut changed = "";
        if shell {
            match command_for(native, &best).await {
                Some(command) => {
                    let _ = tmux.run(&["send-keys", "-t", &pane, "-l", &command]).await;
                    let _ = tmux.run(&["send-keys", "-t", &pane, "Enter"]).await;
                }
                None => start_claude(native, &pane, false).await,
            }
            accept_trust(native, &pane, 60).await;
            ready(native, &pane).await;
        } else {
            accept_trust(native, &pane, 1).await;
            changed = tune(native, &pane, &left, &best).await;
        }
        return reply(
            StatusCode::OK,
            &json!({"ok": true, "session": session, "pane": pane, "created": false, "relaunched": changed}),
        );
    }
    let mut data = Map::new();
    data.insert("routeId".into(), json!("claude:claude"));
    data.insert("model".into(), json!(MODEL));
    data.insert("effort".into(), json!(EFFORT));
    data.insert("harnessAccount".into(), json!(best));
    data.insert("cwd".into(), json!(dir_text));
    let made = super::sessions::new_session_for(native, &data).await?;
    let body: Value = match &made.body {
        ReplyBody::Bytes(b) => serde_json::from_slice(b).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let session = body.get("session").and_then(Value::as_str).unwrap_or("").to_owned();
    if !made.status.is_success() || session.is_empty() {
        let why = body.get("error").and_then(Value::as_str).unwrap_or("No pude abrir la sesión del Modo auto");
        return error(made.status, why);
    }
    let target = format!("={session}:");
    let pane = tmux
        .run(&["display-message", "-p", "-t", &target, "#{pane_id}"])
        .await
        .map(|o| o.stdout.trim().to_owned())
        .unwrap_or_default();
    if !pane.is_empty() {
        start_claude(native, &pane, true).await;
        accept_trust(native, &pane, 60).await;
        ready(native, &pane).await;
    }
    reply(StatusCode::OK, &json!({"ok": true, "session": session, "pane": pane, "created": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pane_in_the_folder_is_reused_claude_first() {
        let dir = "/h/.local/share/comandos/Modo auto";
        let listing = format!("work\t%1\t/h/proj\tclaude\t0\nold\t%2\t{dir}\tzsh\t0\nauto\t%3\t{dir}\tclaude\t0\n");
        assert_eq!(existing(&listing, dir), Some(("auto".into(), "%3".into(), false)));
        let shell = format!("new\t%4\t{dir}\tzsh\t0\n");
        assert_eq!(existing(&shell, dir), Some(("new".into(), "%4".into(), true)));
        let dead = format!("gone\t%5\t{dir}\tclaude\t1\n");
        assert_eq!(existing(&dead, dir), None);
        let trust = "Quick safety check\n ❯ No, exit\n   Yes, I trust this folder\n Enter to confirm";
        assert_eq!(trust_keys(trust), Some(vec!["Down", "Enter"]));
        assert_eq!(trust_keys("nothing here"), None);
        assert_eq!(last_line("a\n  ➜ x . ~/.local/share/comandos/launch/c  \n\n"), "➜ x . ~/.local/share/comandos/launch/c");
    }

    fn row(account: &str, kind: &str, scope: &str, percent: f64) -> Map<String, Value> {
        json!({"provider": "claude", "account": account, "kind": kind, "scope": scope, "percent": percent})
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn the_account_with_the_most_usage_left_wins() {
        let rows = vec![
            row("main", "weekly_all", "", 3.0),
            row("main", "weekly_scoped", "Fable", 90.0),
            row("jesus-signara", "weekly_all", "", 61.0),
            row("relotto-jesus", "session", "", 14.0),
            row("relotto-jesus", "weekly_all", "", 62.0),
        ];
        let left = usage_left(&rows);
        assert_eq!(left, vec![("main".into(), 97.0), ("jesus-signara".into(), 39.0), ("relotto-jesus".into(), 38.0)]);
        assert_eq!(best_account(&left), "main");
        // The 5-hour window counts too, and an Opus-scoped week.
        let busy = vec![row("main", "session", "", 95.0), row("main", "weekly_all", "", 3.0), row("x", "weekly_scoped", "Opus", 50.0)];
        assert_eq!(best_account(&usage_left(&busy)), "x");
        assert_eq!(best_account(&[]), "main");
        // A small gain keeps the conversation; a big one moves it.
        assert!(!should_switch(&left, "jesus-signara", "jesus-signara"));
        assert!(!should_switch(&[("a".into(), 80.0), ("b".into(), 90.0)], "a", "b"));
        assert!(should_switch(&left, "relotto-jesus", "main"));
    }

    #[test]
    fn the_running_model_effort_and_account_are_read_from_the_process() {
        let argv: Vec<String> = ["claude", "--model", "claude-fable-5-2", "--effort", "high"].iter().map(|s| s.to_string()).collect();
        assert_eq!(launched_with(&argv, None), ("claude-fable-5-2".into(), "high".into(), "main".into()));
        let eq: Vec<String> = ["claude", "--model=claude-opus-5-5"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            launched_with(&eq, Some("/h/.claude-accounts/relotto-jesus/")),
            ("claude-opus-5-5".into(), "".into(), "relotto-jesus".into())
        );
        assert_eq!(launched_with(&eq, Some("/h/.claude")).2, "main");
    }

    #[test]
    fn the_folder_gets_its_instructions_and_permissions() {
        let dir = std::env::temp_dir().join(format!("auto-prepare-{}", std::process::id()));
        prepare(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap(), INSTRUCTIONS);
        let settings: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(".claude/settings.json")).unwrap()).unwrap();
        assert_eq!(settings["permissions"]["allow"][0], "Bash(comandos auto:*)");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
