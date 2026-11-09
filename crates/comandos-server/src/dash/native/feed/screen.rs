//! Lo que la pantalla de un pane dice de hecho: un menú numerado (te espera),
//! el texto de límite de uso (te espera, falla) o «esc to interrupt» /
//! «Working (» (trabajando). Solo lectura de una captura ya hecha.
use crate::dash::native::states::cards::{CONFIRM, INTERRUPT, QUESTION, WORKING, search};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;

/// Líneas del final de la captura donde se buscan los hechos.
pub const TAIL_LINES: usize = 14;
/// Tope de `ask.question`.
const MAX_QUESTION: usize = 300;
/// Tope de cada etiqueta de opción (con sus líneas de continuación).
const MAX_LABEL: usize = 200;

static LIMIT: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)usage limit reached|limit reached.*reset|you[’']ve hit your (usage )?limit")
        .ok()
});

/// Lo que se leyó de una captura.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Screen {
    /// Menú numerado o diálogo de confirmación en pantalla.
    pub ask: Option<Value>,
    pub limit: bool,
    pub work: bool,
}

fn found(re: &LazyLock<Option<Regex>>, text: &str) -> bool {
    search(re, text).unwrap_or(false)
}

/// Una línea sin el marco de caja de los lados (`│ ┃ ║`), conservando la
/// sangría interior para medir continuaciones.
fn inner(line: &str) -> &str {
    let body = line
        .trim_start()
        .strip_prefix(['│', '┃', '║'])
        .unwrap_or(line);
    body.trim_end().trim_end_matches(['│', '┃', '║']).trim_end()
}

/// Una línea de puro marco (`╭──╮`, `────`, `╰──╯`), sin texto.
fn is_rule(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty()
        && t.chars().all(|c| {
            c.is_whitespace()
                || matches!(
                    c,
                    '─' | '━'
                        | '═'
                        | '╭'
                        | '╮'
                        | '╰'
                        | '╯'
                        | '┌'
                        | '┐'
                        | '└'
                        | '┘'
                        | '│'
                        | '┃'
                        | '║'
                        | '▔'
                        | '▁'
                        | '-'
                )
        })
        && t.chars()
            .any(|c| !matches!(c, '│' | '┃' | '║') && !c.is_whitespace())
}

/// Una opción de menú: `❯ 1. Yes` → (cursor, columna del dígito, "1", "Yes").
/// Forma de la especificación: `^\s*(?:❯\s*)?([1-9])\.\s+(.+)$`; también el
/// cursor `›` de Codex.
fn option_line(line: &str) -> Option<(bool, usize, char, String)> {
    let text = inner(line);
    let indent = text.chars().take_while(|c| c.is_whitespace()).count();
    let mut rest = text.trim_start();
    let mut column = indent;
    let cursor = rest.starts_with(['❯', '›']);
    if cursor {
        let mark = rest.chars().next()?;
        let after = &rest[mark.len_utf8()..];
        column += 1 + after.chars().take_while(|c| c.is_whitespace()).count();
        rest = after.trim_start();
    }
    let mut chars = rest.chars();
    let digit = chars.next().filter(|c| ('1'..='9').contains(c))?;
    let after = chars.as_str().strip_prefix('.')?;
    if !after.starts_with(char::is_whitespace) || after.trim().is_empty() {
        return None;
    }
    Some((cursor, column, digit, after.trim().to_owned()))
}

/// A lo sumo `max` caracteres; si sobra, corta y termina en `…`.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let keep: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", keep.trim_end())
}

/// Las líneas no vacías justo encima de `above` (excluida), unidas con « · »
/// en su orden, sin marcos; se detiene en un marco y en `MAX_QUESTION`.
fn question(lines: &[&str], above: usize) -> String {
    let mut picked: Vec<String> = Vec::new();
    let mut size = 0;
    for line in lines[..above].iter().rev() {
        if is_rule(line) {
            break;
        }
        let text = inner(line).trim();
        if text.is_empty() {
            continue;
        }
        let extra = text.chars().count() + if picked.is_empty() { 0 } else { 3 };
        if !picked.is_empty() && size + extra > MAX_QUESTION {
            break;
        }
        size += extra;
        picked.push(text.to_owned());
        if size >= MAX_QUESTION {
            break;
        }
    }
    picked.reverse();
    clip(&picked.join(" · "), MAX_QUESTION)
}

/// El último menú numerado que empieza dentro de las últimas líneas: índice
/// de su primera opción y las opciones `(cursor, tecla, etiqueta)`.
/// Una opción leída: (cursor, tecla, etiqueta).
type Choice = (bool, char, String);

fn menu(lines: &[&str], from: usize) -> Option<(usize, Vec<Choice>)> {
    let last = (from..lines.len())
        .rev()
        .find(|&i| option_line(lines[i]).is_some())?;
    // Hacia arriba hasta la primera opción del bloque; hasta dos líneas de
    // continuación (etiquetas largas) entre opciones.
    let mut first = last;
    let mut i = last;
    while i > 0 {
        i -= 1;
        if option_line(lines[i]).is_some() {
            first = i;
        } else if inner(lines[i]).trim().is_empty() || is_rule(lines[i]) || first - i > 2 {
            break;
        }
    }
    let mut options: Vec<(bool, usize, char, String)> = Vec::new();
    for line in &lines[first..] {
        if let Some(option) = option_line(line) {
            options.push(option);
            continue;
        }
        let text = inner(line);
        let indent = text.chars().take_while(|c| c.is_whitespace()).count();
        let more = text.trim();
        // Una continuación va sangrada al menos como el número de su opción;
        // un hueco, un marco o un pie (`Esc to cancel`) cierran el menú.
        match options.last_mut() {
            Some(prev) if !more.is_empty() && !is_rule(line) && indent >= prev.1 => {
                prev.3.push(' ');
                prev.3.push_str(more);
            }
            _ => break,
        }
    }
    let numbered = options
        .iter()
        .enumerate()
        .all(|(n, o)| o.2.to_digit(10) == u32::try_from(n + 1).ok());
    if options.len() < 2 || !numbered {
        return None;
    }
    Some((
        first,
        options
            .into_iter()
            .map(|(cursor, _, key, label)| (cursor, key, clip(&label, MAX_LABEL)))
            .collect(),
    ))
}

/// Lee una captura de `capture-pane -p -S -14`.
pub fn read(capture: &str) -> Screen {
    let mut lines: Vec<&str> = capture.lines().collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let from = lines.len().saturating_sub(TAIL_LINES);
    let tail = lines[from..].join("\n");
    let dialog = found(&QUESTION, &tail) && found(&CONFIRM, &tail);
    let ask = match menu(&lines, from) {
        // Una lista numerada es menú si tiene cursor o si la pantalla ya es un
        // diálogo de confirmación (`QUESTION` y `CONFIRM` de `/state`).
        Some((first, options)) if dialog || options.iter().any(|o| o.0) => Some(json!({
            "question": question(&lines, first),
            "options": options
                .iter()
                .map(|(_, key, label)| json!({"key": key.to_string(), "label": label}))
                .collect::<Vec<_>>(),
        })),
        _ if dialog => {
            // Diálogo sin opciones numeradas (`(y)`, `Enter to confirm`): la
            // pregunta es la última línea que lo pide y lo que tiene encima.
            let anchor = (from..lines.len())
                .rev()
                .find(|&i| found(&QUESTION, inner(lines[i])) || found(&CONFIRM, lines[i]))
                .map_or(lines.len(), |i| i + 1);
            Some(json!({"question": question(&lines, anchor), "options": []}))
        }
        _ => None,
    };
    Screen {
        ask,
        limit: found(&LIMIT, &tail),
        work: found(&INTERRUPT, &tail) || found(&WORKING, &tail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_BOX: &str = "\
● Voy a subirlo.

╭──────────────────────────────────────────────╮
│ Bash command                                 │
│                                              │
│   git push origin main                       │
│   Push the fix                               │
│                                              │
│ Do you want to proceed?                      │
│ ❯ 1. Yes                                     │
│   2. Yes, and don't ask again for git push   │
│      commands in /home/x                     │
│   3. No, and tell Claude what to do          │
│   differently (esc)                          │
╰──────────────────────────────────────────────╯


";

    #[test]
    fn claude_permission_dialog_is_a_menu_with_its_question() {
        let s = read(CLAUDE_BOX);
        let ask = s.ask.expect("ask");
        assert_eq!(
            ask["question"],
            "Bash command · git push origin main · Push the fix · Do you want to proceed?"
        );
        assert_eq!(
            ask["options"],
            json!([
                {"key": "1", "label": "Yes"},
                {"key": "2", "label": "Yes, and don't ask again for git push commands in /home/x"},
                {"key": "3", "label": "No, and tell Claude what to do differently (esc)"},
            ])
        );
        assert!(!s.limit && !s.work);
    }

    #[test]
    fn claude_unboxed_dialog_stops_at_rule_and_footer() {
        let screen = "\
 Lo subo ahora.
────────────────────────────────────────
 Bash command

   git push origin main
   Push the fix

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for git push commands in /home/x
   3. No, and tell Claude what to do differently (esc)

 Esc to cancel · Tab to amend
";
        let ask = read(screen).ask.expect("ask");
        assert_eq!(
            ask["question"],
            "Bash command · git push origin main · Push the fix · Do you want to proceed?"
        );
        let options = ask["options"].as_array().unwrap();
        assert_eq!(options.len(), 3);
        assert_eq!(
            options[2]["label"],
            "No, and tell Claude what to do differently (esc)"
        );
    }

    #[test]
    fn codex_approval_prompt_is_a_menu() {
        let screen = "\
• Voy a subir los cambios.

  Would you like to run the following command?

  $ git push origin main

› 1. Yes, proceed (y)
  2. Yes, and don't ask again for this command (a)
  3. No, and tell Codex what to do differently (esc)

  Press enter to confirm or esc to cancel
";
        let ask = read(screen).ask.expect("ask");
        assert_eq!(
            ask["options"][0],
            json!({"key": "1", "label": "Yes, proceed (y)"})
        );
        assert_eq!(ask["options"].as_array().unwrap().len(), 3);
        assert!(
            ask["question"]
                .as_str()
                .unwrap()
                .ends_with("Would you like to run the following command? · $ git push origin main"),
            "{}",
            ask["question"]
        );
    }

    #[test]
    fn plain_lists_prompts_and_old_menus_are_not_asks() {
        assert_eq!(
            read("Pasos:\n1. Compilar\n2. Probar\n\n> escribe aquí").ask,
            None
        );
        assert_eq!(read("❯ 2. algo\n  3. otro").ask, None, "no empieza en 1");
        assert_eq!(read("❯ 1. solo una").ask, None);
        // Un menú que quedó más arriba de las últimas 14 líneas ya no cuenta.
        let mut old = String::from("❯ 1. Yes\n  2. No\n");
        for n in 0..TAIL_LINES {
            old.push_str(&format!("línea {n}\n"));
        }
        assert_eq!(read(&old).ask, None);
        // El composer de Codex con texto numerado no es menú.
        assert_eq!(read("› 1. haz esto\n\n  gpt-6 high · ~/x").ask, None);
    }

    #[test]
    fn confirm_dialog_without_numbers_still_asks() {
        let s = read("Hola\n\nDo you want to overwrite a.rs? (y)\n");
        let ask = s.ask.expect("ask");
        assert_eq!(ask["question"], "Hola · Do you want to overwrite a.rs? (y)");
        assert_eq!(ask["options"], json!([]));
    }

    #[test]
    fn limit_and_work_come_from_the_tail() {
        assert!(read("x\nClaude usage limit reached. Your limit will reset at 5pm").limit);
        assert!(read("■ You've hit your usage limit. Upgrade or try again later").limit);
        assert!(read("5-hour limit reached ∙ resets 3am").limit);
        assert!(!read("⚠ Heads up, you have less than 5% of your weekly limit left.").limit);
        assert!(read("✻ Pondering… (12s · esc to interrupt)\n\n> ").work);
        assert!(read("• Working (3s • esc to interrupt)").work);
        assert!(!read("> listo").work);
    }
}
