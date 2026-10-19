pub mod auth;
pub mod cli;
pub mod mutations;
pub mod python_json;
use serde_json::Value;
use std::{env, path::PathBuf, process::Command};

pub type Result<T> = std::result::Result<T, String>;

pub fn expand_vars(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            result.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let (name, end) = if chars.get(i) == Some(&'{') {
            let begin = i + 1;
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            if i == chars.len() {
                result.extend(&chars[start..]);
                break;
            }
            (chars[begin..i].iter().collect::<String>(), i + 1)
        } else {
            let begin = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            (chars[begin..i].iter().collect::<String>(), i)
        };
        result.push_str(&env::var(&name).unwrap_or_else(|_| chars[start..end].iter().collect()));
        i = end;
    }
    result
}

pub fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| {
            nix::unistd::User::from_uid(nix::unistd::getuid())
                .ok()
                .flatten()
                .map(|u| u.dir)
        })
        .ok_or_else(|| "Home unavailable".into())
}

pub fn expand_user(value: &str) -> String {
    if !value.starts_with('~') {
        return value.into();
    }
    let end = value.find('/').unwrap_or(value.len());
    let home = if end == 1 {
        home_dir().ok()
    } else {
        nix::unistd::User::from_name(&value[1..end])
            .ok()
            .flatten()
            .map(|u| u.dir)
    };
    home.map(|h| {
        format!(
            "{}{}",
            h.to_string_lossy().trim_end_matches('/'),
            &value[end..]
        )
    })
    .unwrap_or_else(|| value.into())
}

pub fn command(spec: &Value, direct: bool) -> Result<Command> {
    command_env(spec, direct, None)
}

/// `env` del catálogo con `${VAR}` expandido en el entorno de este proceso, en su orden.
pub fn spec_env(spec: &Value) -> Result<Vec<(String, String)>> {
    let Some(vars) = spec.get("env") else {
        return Ok(Vec::new());
    };
    let vars = vars.as_object().ok_or("Invalid environment")?;
    Ok(vars
        .iter()
        .map(|(key, value)| (key.clone(), expand_vars(&python_string(value))))
        .collect())
}

/// Como [`command`], pero con el `env` ya resuelto por el llamador si se da (el broker recibe
/// el de la sesión cliente); `None` lo resuelve aquí. Hereda el entorno de este proceso y le
/// superpone `env`. Valida en el mismo orden que siempre: comando, argumentos, entorno.
pub fn command_env(
    spec: &Value,
    direct: bool,
    env: Option<&[(String, String)]>,
) -> Result<Command> {
    let mut command = Command::new(expand_user(
        spec["command"].as_str().ok_or("Invalid command")?,
    ));
    if let Some(args) = spec.get("args") {
        for arg in args.as_array().ok_or("Invalid arguments")? {
            command.arg(arg.as_str().ok_or("Invalid argument")?);
        }
    }
    let own;
    let env = match env {
        Some(env) => env,
        None => {
            own = spec_env(spec)?;
            &own
        }
    };
    command.envs(env.iter().map(|(k, v)| (k, v)));
    // Python solo cambia de directorio si `cwd` es verdadero (una cadena vacía se omite).
    if let Some(cwd) = spec
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !direct || !c.is_empty())
    {
        command.current_dir(if direct { expand_user(cwd) } else { cwd.into() });
    }
    command.stderr(std::process::Stdio::null());
    Ok(command)
}

/// Verdad de Python (`bool(x)`) para un valor JSON.
pub fn py_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_str().parse::<f64>() != Ok(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `str(valor)` de Python: la cadena tal cual y, para el resto, su `repr`.
pub fn python_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => python_repr(value),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::String(s) => python_str_repr(s),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => python_number(n.as_str()),
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(python_repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| format!("{}: {}", python_str_repr(k), python_repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn python_str_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || (0x7f..0xa0).contains(&(c as u32)) => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Número JSON tal como lo imprime Python: enteros intactos, `float` con su `repr`.
fn python_number(text: &str) -> String {
    match text {
        "NaN" => return "nan".into(),
        "Infinity" => return "inf".into(),
        "-Infinity" => return "-inf".into(),
        _ => {}
    }
    if !text.contains(['.', 'e', 'E']) {
        return text.into();
    }
    let Ok(f) = text.parse::<f64>() else {
        return text.into();
    };
    if f.is_infinite() {
        return if f < 0.0 { "-inf" } else { "inf" }.into();
    }
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').expect("formato científico");
    let exp: i32 = exp.parse().expect("exponente");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let sign = if f.is_sign_negative() { "-" } else { "" };
    let body = if !(-4..16).contains(&exp) {
        let frac = if digits.len() > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        format!(
            "{}{frac}e{}{:02}",
            &digits[..1],
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        )
    } else if exp >= 0 {
        let int_len = exp as usize + 1;
        if digits.len() <= int_len {
            format!("{digits}{}.0", "0".repeat(int_len - digits.len()))
        } else {
            format!("{}.{}", &digits[..int_len], &digits[int_len..])
        }
    } else {
        format!("0.{}{digits}", "0".repeat((-exp - 1) as usize))
    };
    format!("{sign}{body}")
}

/// Ruta del servidor stdio directo: reproduce el `serve` del proxy Python. Cambia de
/// directorio (si falla, imprime lo mismo que el envoltorio Python y sale con 1), manda stderr a `/dev/null` y
/// reemplaza el proceso. Solo vuelve si el `exec` falla.
pub fn exec_direct(spec: &Value) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let mut spec = spec.clone();
    if let Some(cwd) = spec
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
    {
        let path = expand_user(cwd);
        if let Err(error) = env::set_current_dir(&path) {
            // El envoltorio del proxy Python captura la excepción y solo imprime su clase.
            let class = match error.raw_os_error() {
                Some(2) => "FileNotFoundError",
                Some(13) => "PermissionError",
                Some(20) => "NotADirectoryError",
                _ => "OSError",
            };
            eprintln!("Extension operation failed: {class}");
            std::process::exit(1);
        }
        spec.as_object_mut().map(|o| o.remove("cwd"));
    }
    let _ = command(&spec, true)?.exec();
    Ok(())
}

pub mod serve;
pub mod transport;

pub mod metadata;
pub mod tokenizer;

pub mod catalog;
pub mod config;
pub mod skills;

pub mod check;
pub mod hub;

mod check_protocol;

pub mod broker;
