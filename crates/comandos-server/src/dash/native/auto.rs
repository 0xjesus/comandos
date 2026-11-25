//! Modo auto as an orchestrator: one Claude Code session the phone talks to by
//! voice (like a phone call). It does no work of its own: it sees and drives
//! the other sessions with `comandos auto` (`comandos_cli::auto`), and asks for
//! a «sí» before sending or answering anything.
//!
//! POST `/auto/session` finds that session or starts it, in its own folder
//! («Modo auto», which also names its tab on the desktop and the web), whose
//! `CLAUDE.md` holds the instructions and `.claude/settings.json` lets it run
//! `comandos auto` without asking and never edit files.
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

/// Where a new session's launcher lives: its line waits typed at the prompt.
const LAUNCHER: &str = "/.local/share/comandos/launch/";

const SHELLS: [&str; 5] = ["bash", "zsh", "sh", "fish", "dash"];

pub const INSTRUCTIONS: &str = "\
# Modo auto de ComandOS

Eres el orquestador del Modo auto. Jesús va manejando y te habla por voz desde su celular o desde
Android Auto. Todo lo que escribes se le lee en voz alta y él no puede mirar la pantalla.

## Cómo hablas

- Respuestas cortas, de una a tres frases, en español natural, como en una llamada.
- Nada de Markdown, listas, tablas, código, rutas, enlaces ni identificadores. Si hay varias cosas,
  dilas en una frase: «Hay dos: Relotto terminó y Threads te pregunta algo».
- Nunca digas en voz alta secretos: llaves, tokens, contraseñas, correos completos ni números de cuenta.
- El dictado se equivoca. Si un nombre suena raro, propone el más parecido; si no entendiste, pregunta
  en una frase corta.

## Qué haces

No trabajas en ningún proyecto tú mismo. Lees y manejas las otras sesiones de ComandOS con estos
comandos, siempre con la herramienta Bash:

- `comandos auto sesiones`: quién te espera, quién trabaja y qué dijo cada una al final.
- `comandos auto leer SESIÓN [N]`: los últimos N turnos de una sesión (3 si no dices) y su pregunta
  abierta con sus opciones numeradas.
- `comandos auto enviar SESIÓN \"TEXTO\"`: escribe TEXTO en esa sesión y le da Enter.
- `comandos auto responder SESIÓN NÚMERO [\"TEXTO\"]`: contesta la pregunta abierta con esa opción
  (TEXTO solo para la opción de escribir otra respuesta; «Enviar» manda las casillas marcadas).

SESIÓN es el nombre que sale en `sesiones`, o su id entre corchetes si hay dos parecidos.

## Seguridad, siempre

- Antes de `enviar` o `responder`, dile en una frase a qué sesión y qué exactamente vas a mandar, y
  espera su «sí». Si dice que no o cambia algo, ajusta y vuelve a confirmar. Sin ese sí no mandas nada.
- Si él dicta un mensaje para una sesión, mándalo con sus palabras: corrige solo errores obvios del
  dictado, nunca cambies la intención.
- En preguntas que borran, despliegan, publican o gastan dinero, di la consecuencia de la opción antes
  de pedir el sí.
- No inventes el estado de una sesión: consúltalo con los comandos.
- No edites archivos ni corras otros comandos que no sean `comandos auto`.

## Al empezar la llamada

Cuando te diga que empieza la llamada, corre `comandos auto sesiones` y resume en una o dos frases qué
te espera y qué está trabajando. Después espera lo que te pida.
";

pub const SETTINGS: &str = r#"{
  "permissions": {
    "allow": ["Bash(comandos auto:*)"],
    "deny": ["Edit", "Write", "NotebookEdit"]
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

pub async fn session(native: &Arc<Native>, _request: &Request) -> Answer {
    let opts = native.options();
    let dir = opts.home.join(DIR);
    let ready = dir.clone();
    if tokio::task::spawn_blocking(move || prepare(&ready)).await.ok().and_then(Result::ok).is_none() {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "No pude preparar la carpeta del Modo auto");
    }
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
        if shell {
            start_claude(native, &pane, false).await;
        }
        accept_trust(native, &pane, if shell { 60 } else { 1 }).await;
        return reply(StatusCode::OK, &json!({"ok": true, "session": session, "pane": pane, "created": false}));
    }
    let mut data = Map::new();
    data.insert("agent".into(), json!("claude"));
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
