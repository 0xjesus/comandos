//! Shared dashboard theme parsing plus desktop-only CSS.
use comandos_core::pomodoro::python_str;
use serde_json::{Map, Value};

pub use comandos_notifyd::theme::{themes_from_file, tokens};

pub use comandos_desktop::DEFAULT_THEME;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeTokens {
    pub values: Map<String, Value>,
    pub ansi: Vec<String>,
}

pub fn desktop_theme(name: &str, _themes: &Value) -> Option<ThemeTokens> {
    // El escritorio usa THEMES del original; el JSON del tablero tiene otros tonos.
    let themes: Value = serde_json::from_str(include_str!("ui/desktop-themes.json")).ok()?;
    let mut values = themes
        .get(name)
        .or_else(|| themes.get("noche"))?
        .as_object()?
        .clone();
    let ansi = values
        .remove("pal")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    values.insert("name".into(), Value::String(name.into()));
    Some(ThemeTokens { ansi, values })
}

/// The theme chosen in the preferences, including a user theme saved in
/// `user_themes`: its colours over those of the built-in theme it came from.
/// The state roles keep coming from that built-in theme.
pub fn prefs_theme(prefs: &Value, themes: &Value) -> Option<ThemeTokens> {
    let name = prefs
        .get("theme")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_THEME);
    let user = name.starts_with("u-").then(|| {
        prefs
            .get("user_themes")?
            .as_array()?
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some(name))
    });
    let Some(user) = user.flatten() else {
        return desktop_theme(name, themes);
    };
    let base = user
        .get("base")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_THEME);
    let mut theme = desktop_theme(base, themes)?;
    let colors = user.get("colors").and_then(Value::as_object);
    for (key, value) in colors.into_iter().flatten() {
        if key == "pal" {
            let pal: Option<Vec<String>> = value
                .as_array()
                .filter(|p| p.len() == 16)
                .and_then(|p| p.iter().map(|c| hex_color(c)).collect());
            if let Some(pal) = pal {
                theme.ansi = pal;
            }
        } else if theme.values.contains_key(key)
            && key != "name"
            && let Some(color) = hex_color(value)
        {
            theme.values.insert(key.clone(), Value::String(color));
        }
    }
    Some(theme)
}

fn hex_color(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    let tail = s.strip_prefix('#')?;
    (tail.len() == 6 && tail.bytes().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", tail.to_ascii_uppercase()))
}

/// Whether a theme's background is dark (GTK's own widgets follow it).
pub fn is_dark(theme: &ThemeTokens) -> bool {
    let bg = theme
        .values
        .get("bg")
        .and_then(Value::as_str)
        .unwrap_or("#000000");
    let c = |i: usize| {
        f64::from(u8::from_str_radix(bg.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0)) / 255.0
    };
    0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5) < 0.35
}

fn state_role(t: &Map<String, Value>, key: &str, fallback: &str) -> String {
    static DASH: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/themes.json"
        )))
        .unwrap_or(Value::Null)
    });
    let own = token(t, key);
    if !own.is_empty() {
        return own;
    }
    let name = t.get("name").and_then(Value::as_str).unwrap_or("noche");
    DASH.get("themes")
        .and_then(|themes| themes.get(name))
        .and_then(|theme| theme.get(key))
        .and_then(Value::as_str)
        .filter(|v| v.starts_with('#') && v.len() == 7)
        .unwrap_or(fallback)
        .to_string()
}

pub fn base_css(theme: &ThemeTokens) -> String {
    let t = &theme.values;
    let mut css = include_str!("ui/app-original.css").to_string();
    for (placeholder, key) in [
        ("BAR", "bar"),
        ("BG", "bg"),
        ("TEXT", "text"),
        ("DIM", "dim"),
        ("FAINT", "faint"),
        ("BRAND", "brand"),
        ("LINE", "line"),
        ("LINE2", "line2"),
        ("PANEL2", "panel2"),
        ("PANEL", "panel"),
    ] {
        css = css.replace(&format!("@{placeholder}@"), &token(t, key));
    }
    // The desktop palettes have no state roles; take them from the same
    // theme of `config/themes.json` so every theme keeps its own states.
    for (placeholder, key, fallback) in [
        ("WAITING", "waiting", "#FFAE1A"),
        ("WORKING", "working", "#5CC8FF"),
        ("DONE", "done", "#5BD69A"),
        ("ERR", "err", "#E5484D"),
    ] {
        css = css.replace(&format!("@{placeholder}@"), &state_role(t, key, fallback));
    }
    css = css.replace(
        "@ICONS@",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../dash/icons"),
    );
    css
}

pub fn theme_css(theme: &ThemeTokens) -> String {
    let t = &theme.values;
    let mut css = base_css(theme);
    css.push_str(&format!(
        "
window {{ background-color: {}; color: {}; }}
.terminal {{ background-color: {}; color: {}; }}
.selection {{ background-color: {}; color: {}; }}
",
        token(t, "bg"),
        token(t, "text"),
        token(t, "bg"),
        token(t, "fg"),
        token(t, "brand"),
        token(t, "bg")
    ));
    css
}

pub fn header_css(theme: &ThemeTokens) -> String {
    // Plantilla literal del AST de _build_hb_css; no evalúa Python en ejecución.
    let mut css = include_str!("ui/foundation.css").trim_end().to_string();
    for key in [
        "bg", "line2", "line", "bar", "dim", "fg", "panel", "waiting", "text", "brand", "panel2",
        "cursor", "err",
    ] {
        let fallback = match key {
            "line2" | "panel2" => "line",
            "bar" | "panel" => "bg",
            "fg" => "text",
            _ => key,
        };
        let value = theme
            .values
            .get(key)
            .or_else(|| theme.values.get(fallback))
            .map(python_str)
            .unwrap_or_else(|| {
                match key {
                    "waiting" => "#FFAE1A",
                    "dim" => "#8a98ab",
                    "err" => "#E5484D",
                    _ => "#000000",
                }
                .into()
            });
        css = css.replace(&format!("@CC_THEME_{key}@"), &value);
    }
    css
}

pub fn button_style_css(style: &str, theme: &ThemeTokens) -> String {
    let template = match style {
        "arcade" => include_str!("ui/buttons-arcade.css"),
        "tecla" => include_str!("ui/buttons-tecla.css"),
        "pixel" => include_str!("ui/buttons-pixel.css"),
        "consola" => include_str!("ui/buttons-consola.css"),
        _ => include_str!("ui/buttons-sutil.css"),
    };
    let mut css = template.to_string();
    for key in ["line", "line2", "text", "dim", "bar", "bg"] {
        let value = theme
            .values
            .get(key)
            .or_else(|| (key == "line2").then(|| theme.values.get("line")).flatten())
            .map(python_str)
            .unwrap_or_else(|| {
                if key == "bg" {
                    "#0A0D13".into()
                } else {
                    "#222A3A".into()
                }
            });
        css = css.replace(&format!("@CC_THEME_{key}@"), &value);
    }
    css
}

fn token(t: &Map<String, Value>, key: &str) -> String {
    t.get(key).map_or_else(String::new, python_str)
}
