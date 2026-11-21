//! Temas propios: un tema de fábrica con sus colores, acabado y botones
//! cambiados, guardado en `prefs.json` → `user_themes`.
//!
//! Los colores son los mismos 12 tokens y 16 colores de terminal de
//! `desktop-themes.json` (la paleta del escritorio). Lo que un tema propio no
//! trae (estados, brillo, color sobre el acento) sale de su tema `base`.
use crate::escape::{attr_esc, md_esc};
use crate::theme_preferences::{BUTTONS, THEMES, metadata};
use serde_json::{Map, Value, json};

/// Máximo de temas propios guardados.
pub const MAX_THEMES: usize = 40;
/// Prefijo de los ids de temas propios: nunca choca con uno de fábrica.
pub const PREFIX: &str = "u-";
/// Los 12 colores de la interfaz y de la terminal, en el orden de la lista.
pub const UI_TOKENS: &[(&str, &str, &str)] = &[
    ("bg", "Fondo", "Background"),
    ("bar", "Barra superior", "Top bar"),
    ("panel", "Paneles", "Panels"),
    ("panel2", "Elemento elegido", "Selected item"),
    ("line", "Líneas", "Lines"),
    ("line2", "Líneas marcadas", "Strong lines"),
    ("text", "Texto", "Text"),
    ("dim", "Texto secundario", "Secondary text"),
    ("faint", "Texto tenue", "Faint text"),
    ("brand", "Acento", "Accent"),
];
pub const TERM_TOKENS: &[(&str, &str, &str)] = &[
    ("fg", "Texto de la terminal", "Terminal text"),
    ("cursor", "Cursor", "Cursor"),
];
pub const ANSI: &[(&str, &str)] = &[
    ("negro", "black"),
    ("rojo", "red"),
    ("verde", "green"),
    ("amarillo", "yellow"),
    ("azul", "blue"),
    ("magenta", "magenta"),
    ("cian", "cyan"),
    ("blanco", "white"),
];
pub const TREATMENTS: &[(&str, &str, &str)] = &[
    ("solid", "Sólido", "Solid"),
    ("glass", "Vidrio", "Glass"),
    ("neon", "Neón", "Neon"),
    ("contrast", "Contraste", "Contrast"),
];
/// Colores de superficie: su contraste se mide con el texto encima.
const SURFACES: &[&str] = &["bg", "bar", "panel", "panel2"];
/// Líneas: no llevan texto, no se mide su contraste.
const LINES: &[&str] = &["line", "line2"];

fn factory() -> &'static Value {
    static F: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    F.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../comandos-app/src/ui/desktop-themes.json"
        ))
        .unwrap_or(Value::Null)
    })
}

/// Los colores de un tema de fábrica (`None` si el id no es de fábrica).
pub fn factory_colors(id: &str) -> Option<Map<String, Value>> {
    THEMES
        .contains(&id)
        .then(|| factory().get(id)?.as_object().cloned())
        .flatten()
}

pub fn is_user_id(id: &str) -> bool {
    id.strip_prefix(PREFIX).is_some_and(|tail| {
        (1..=24).contains(&tail.len())
            && tail
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

/// `#RRGGBB` en mayúsculas, o `None`.
pub fn hex(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    let tail = s.strip_prefix('#')?;
    (tail.len() == 6 && tail.bytes().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", tail.to_ascii_uppercase()))
}

fn label(v: Option<&Value>) -> Option<String> {
    let s: String = v?.as_str()?.trim().chars().take(48).collect();
    (!s.is_empty() && !s.chars().any(char::is_control)).then_some(s)
}

/// Normaliza los colores: los 12 tokens y los 16 de la terminal, todos
/// `#RRGGBB`. Un color que falte o no sea válido rechaza el tema.
pub fn colors(v: &Value) -> Option<Map<String, Value>> {
    let mut out = Map::new();
    for (key, _, _) in UI_TOKENS.iter().chain(TERM_TOKENS) {
        out.insert((*key).into(), Value::String(hex(v.get(*key)?)?));
    }
    let pal = v.get("pal")?.as_array()?;
    if pal.len() != 16 {
        return None;
    }
    let pal = pal.iter().map(hex).collect::<Option<Vec<_>>>()?;
    out.insert("pal".into(), json!(pal));
    Some(out)
}

/// Un tema propio válido en su forma canónica, o el motivo del rechazo.
pub fn validate(v: &Value) -> Result<Value, &'static str> {
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| is_user_id(id))
        .ok_or("Tema inválido: id")?;
    let label = label(v.get("label")).ok_or("Ponle un nombre al tema")?;
    let base = v
        .get("base")
        .and_then(Value::as_str)
        .filter(|b| THEMES.contains(b))
        .ok_or("Tema inválido: base")?;
    let colors = v
        .get("colors")
        .and_then(colors)
        .ok_or("Tema inválido: colores")?;
    let treatment = v
        .get("treatment")
        .and_then(Value::as_str)
        .filter(|t| TREATMENTS.iter().any(|(id, _, _)| id == t))
        .ok_or("Tema inválido: acabado")?;
    let buttons = v
        .get("button_style")
        .and_then(Value::as_str)
        .filter(|b| BUTTONS.iter().any(|(id, _, _)| id == b))
        .ok_or("Tema inválido: botones")?;
    Ok(json!({
        "id": id,
        "label": label,
        "base": base,
        "colors": colors,
        "treatment": treatment,
        "button_style": buttons,
    }))
}

/// Los temas propios válidos de unas preferencias (los demás se ignoran).
pub fn list(prefs: &Value) -> Vec<Value> {
    prefs
        .get("user_themes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| validate(t).ok())
        .take(MAX_THEMES)
        .collect()
}

pub fn find(prefs: &Value, id: &str) -> Option<Value> {
    list(prefs).into_iter().find(|t| field(t, "id") == id)
}

/// Un texto de un objeto JSON, o `""`.
pub fn field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Inserta o reemplaza un tema propio en la lista guardada.
pub fn upsert(stored: &Value, theme: &Value) -> Result<Value, &'static str> {
    let theme = validate(theme)?;
    let mut out: Vec<Value> = stored
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| validate(t).ok())
        .collect();
    if let Some(slot) = out.iter_mut().find(|t| t.get("id") == theme.get("id")) {
        *slot = theme;
    } else if out.len() >= MAX_THEMES {
        return Err("Ya tienes 40 temas propios; borra uno antes");
    } else {
        out.push(theme);
    }
    Ok(Value::Array(out))
}

pub fn remove(stored: &Value, id: &str) -> Value {
    Value::Array(
        stored
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| field(t, "id") != id)
            .cloned()
            .collect(),
    )
}

/// Lo que hace falta para pintar un tema, sea de fábrica o propio.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub id: String,
    pub base: String,
    pub label: String,
    pub colors: Map<String, Value>,
    pub treatment: String,
    /// `None` en uno de fábrica: los botones siguen la preferencia global.
    pub button_style: Option<String>,
    pub user: bool,
}

pub fn resolve(prefs: &Value, id: &str) -> Resolved {
    if let Some(t) = find(prefs, id) {
        return Resolved {
            id: id.into(),
            base: field(&t, "base").into(),
            label: field(&t, "label").into(),
            colors: t
                .get("colors")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            treatment: field(&t, "treatment").into(),
            button_style: Some(field(&t, "button_style").into()),
            user: true,
        };
    }
    let id = crate::theme_preferences::theme(id);
    let meta = metadata();
    Resolved {
        id: id.into(),
        base: id.into(),
        label: meta
            .get(id)
            .map(|m| field(m, "label"))
            .unwrap_or_default()
            .into(),
        colors: factory_colors(id).unwrap_or_default(),
        treatment: treatment_of(meta.get(id).and_then(|m| m.get("treatment")).and_then(Value::as_str)),
        button_style: None,
        user: false,
    }
}

fn treatment_of(t: Option<&str>) -> String {
    match t {
        Some(t) if TREATMENTS.iter().any(|(id, _, _)| *id == t) => t.into(),
        _ => "solid".into(),
    }
}

// ---------------------------------------------------------------- color

fn rgb(hex: &str) -> [f64; 3] {
    let p = |i: usize| {
        f64::from(u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0))
    };
    [p(1), p(3), p(5)]
}

pub fn luminance(hex: &str) -> f64 {
    let [r, g, b] = rgb(hex).map(|v| {
        let v = v / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

pub fn ratio(a: &str, b: &str) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn get<'a>(c: &'a Map<String, Value>, key: &str) -> &'a str {
    if let Some(i) = key
        .strip_prefix("pal")
        .and_then(|i| i.parse::<usize>().ok())
    {
        return c
            .get("pal")
            .and_then(|p| p.get(i))
            .and_then(Value::as_str)
            .unwrap_or("#000000");
    }
    c.get(key).and_then(Value::as_str).unwrap_or("#000000")
}

/// Contraste de un color: superficies con el texto encima, el resto sobre el
/// fondo. `None` para las líneas, que no llevan texto.
pub fn contrast(c: &Map<String, Value>, key: &str) -> Option<f64> {
    if LINES.contains(&key) {
        return None;
    }
    let against = if SURFACES.contains(&key) {
        "text"
    } else {
        "bg"
    };
    Some(ratio(get(c, key), get(c, against)))
}

/// ✓ se lee bien, ≈ sólo para cosas grandes, ! cuesta leerlo.
pub fn mark(r: f64) -> (&'static str, &'static str) {
    if r >= 4.5 {
        ("good", "✓")
    } else if r >= 3.0 {
        ("fair", "≈")
    } else {
        ("poor", "!")
    }
}

pub fn mark_text(r: f64, en: bool) -> String {
    let what = match (r >= 7.0, r >= 4.5, r >= 3.0, en) {
        (true, _, _, false) => "se lee de sobra",
        (true, _, _, true) => "very easy to read",
        (_, true, _, false) => "se lee bien",
        (_, true, _, true) => "easy to read",
        (_, _, true, false) => "sólo para cosas grandes",
        (_, _, true, true) => "only for large things",
        (_, _, _, false) => "cuesta leerlo",
        (_, _, _, true) => "hard to read",
    };
    format!("{r:.1} : 1 · {what}")
}

/// Variables CSS del tablero para unos colores de escritorio: lo que un tema
/// propio cambia sobre su base.
pub fn css_vars(c: &Map<String, Value>) -> Vec<(&'static str, String)> {
    let brand = get(c, "brand");
    let on_brand = if luminance(brand) > 0.36 {
        "#080B11"
    } else {
        "#FFFFFF"
    };
    vec![
        ("--bg", get(c, "bg").into()),
        ("--panel", get(c, "panel").into()),
        ("--panel2", get(c, "panel2").into()),
        ("--line", get(c, "line").into()),
        ("--line2", get(c, "line2").into()),
        ("--text", get(c, "text").into()),
        ("--dim", get(c, "dim").into()),
        ("--faint", get(c, "faint").into()),
        ("--brand", brand.into()),
        ("--on-brand", on_brand.into()),
    ]
}

/// Paleta de la terminal web (mismas claves que `page_theme`).
pub fn term_palette(c: &Map<String, Value>) -> Value {
    let mut out = json!({
        "background": get(c, "bg"),
        "foreground": get(c, "fg"),
        "cursor": get(c, "cursor"),
        "cursorAccent": get(c, "bg"),
        "selectionBackground": get(c, "line2"),
        "panel": get(c, "panel"),
        "panel2": get(c, "panel2"),
        "line": get(c, "line"),
        "line2": get(c, "line2"),
        "dim": get(c, "dim"),
        "faint": get(c, "faint"),
        "brand": get(c, "brand"),
    });
    const NAMES: [&str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "brightBlack",
        "brightRed",
        "brightGreen",
        "brightYellow",
        "brightBlue",
        "brightMagenta",
        "brightCyan",
        "brightWhite",
    ];
    if let Some(map) = out.as_object_mut() {
        for (i, name) in NAMES.iter().enumerate() {
            map.insert((*name).into(), json!(get(c, &format!("pal{i}"))));
        }
    }
    out
}

// ---------------------------------------------------------------- editor

/// Lo que se está editando en la tarjeta abierta.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// Tema del que se partió (de fábrica o propio).
    pub from: Resolved,
    pub colors: Map<String, Value>,
    pub treatment: String,
    /// Botones del borrador; en uno de fábrica empieza en la preferencia global.
    pub button_style: String,
    pub tab: String,
}

impl Draft {
    pub fn new(from: Resolved, global_buttons: &str) -> Self {
        let button_style = from
            .button_style
            .clone()
            .unwrap_or_else(|| global_buttons.into());
        Self {
            colors: from.colors.clone(),
            treatment: from.treatment.clone(),
            button_style,
            tab: "colors".into(),
            from,
        }
    }
    /// Hay cambios sin guardar. Los botones de un tema de fábrica no cuentan:
    /// ahí siguen siendo la preferencia global.
    pub fn edited(&self) -> bool {
        self.colors != self.from.colors
            || self.treatment != self.from.treatment
            || (self.from.user
                && Some(self.button_style.as_str()) != self.from.button_style.as_deref())
    }
    pub fn set_color(&mut self, key: &str, value: &str) -> bool {
        let Some(v) = hex(&json!(value)) else {
            return false;
        };
        if let Some(i) = key
            .strip_prefix("pal")
            .and_then(|i| i.parse::<usize>().ok())
        {
            match self.colors.get_mut("pal").and_then(Value::as_array_mut) {
                Some(pal) => match pal.get_mut(i) {
                    Some(slot) => *slot = Value::String(v),
                    None => return false,
                },
                None => return false,
            }
            return true;
        }
        if !UI_TOKENS
            .iter()
            .chain(TERM_TOKENS)
            .any(|(k, _, _)| *k == key)
        {
            return false;
        }
        self.colors.insert(key.into(), Value::String(v));
        true
    }
    /// El tema propio que se guardaría con este nombre y este id.
    pub fn theme(&self, id: &str, label: &str) -> Value {
        json!({
            "id": id,
            "label": label,
            "base": self.from.base,
            "colors": self.colors,
            "treatment": self.treatment,
            "button_style": self.button_style,
        })
    }
}

fn tr<'a>(en: bool, es: &'a str, en_text: &'a str) -> &'a str {
    if en { en_text } else { es }
}

fn mark_html(c: &Map<String, Value>, key: &str, en: bool) -> String {
    match contrast(c, key) {
        None => "<span class=\"ut-cr\"></span>".into(),
        Some(r) => {
            let (cls, sym) = mark(r);
            format!(
                "<span class=\"ut-cr {cls}\" data-ut-cr=\"{key}\" title=\"{}\">{sym} {r:.1}</span>",
                attr_esc(&mark_text(r, en))
            )
        }
    }
}

fn row(c: &Map<String, Value>, key: &str, name: &str, en: bool) -> String {
    let v = get(c, key);
    format!(
        "<label class=\"ut-row\"><input type=\"color\" data-ut-color=\"{key}\" value=\"{v}\" aria-label=\"{n}\"><span>{n}</span><code data-ut-hex=\"{key}\">{v}</code>{}</label>",
        mark_html(c, key, en),
        n = md_esc(name),
    )
}

fn ansi_html(c: &Map<String, Value>, from: usize, en: bool) -> String {
    let mut out = String::from("<div class=\"ut-ansi\">");
    for (i, (es, en_name)) in ANSI.iter().enumerate() {
        let key = format!("pal{}", from + i);
        let v = get(c, &key);
        let name = format!(
            "{}{}",
            tr(en, es, en_name),
            if from > 0 {
                tr(en, " claro", " bright")
            } else {
                ""
            }
        );
        let r = contrast(c, &key).unwrap_or(0.0);
        let label = if from == 0 {
            md_esc(tr(en, es, en_name))
        } else {
            String::new()
        };
        out.push_str(&format!(
            "<label title=\"{t}\"><input type=\"color\" data-ut-color=\"{key}\" value=\"{v}\" aria-label=\"{a}\"><span>{label}</span></label>",
            t = attr_esc(&format!("{name} · {}", mark_text(r, en))),
            a = attr_esc(&name),
        ));
    }
    out.push_str("</div>");
    out
}

fn colors_tab(d: &Draft, en: bool) -> String {
    let c = &d.colors;
    let rows = |list: &[(&str, &str, &str)]| {
        list.iter()
            .map(|(k, es, en_name)| row(c, k, tr(en, es, en_name), en))
            .collect::<String>()
    };
    format!(
        "<p class=\"ut-hint\">{}</p><div class=\"ut-grp\"><span class=\"ut-eyebrow\">{}</span>{}</div><div class=\"ut-grp\"><span class=\"ut-eyebrow\">{}</span>{}</div><div class=\"ut-grp\"><span class=\"ut-eyebrow\">{}</span><span class=\"ut-sub\">{}</span>{}<span class=\"ut-sub\">{}</span>{}</div>",
        tr(
            en,
            "Clic en una muestra para elegir el color.",
            "Click a swatch to pick the colour."
        ),
        tr(en, "Interfaz", "Interface"),
        rows(UI_TOKENS),
        tr(en, "Terminal", "Terminal"),
        rows(TERM_TOKENS),
        tr(en, "16 colores de la terminal", "16 terminal colours"),
        tr(en, "normales", "normal"),
        ansi_html(c, 0, en),
        tr(en, "claros", "bright"),
        ansi_html(c, 8, en),
    )
}

fn finish_tab(d: &Draft, en: bool) -> String {
    let segs = TREATMENTS
        .iter()
        .map(|(id, es, en_name)| {
            format!(
                "<button type=\"button\" data-ut-treatment=\"{id}\" aria-pressed=\"{}\">{}</button>",
                d.treatment == *id,
                tr(en, es, en_name)
            )
        })
        .collect::<String>();
    format!(
        "<div class=\"ut-crow\"><span>{}</span><div class=\"ut-seg\">{segs}</div></div><p class=\"ut-hint\">{}</p>",
        tr(en, "Acabado", "Finish"),
        tr(
            en,
            "Vidrio difumina las barras y paneles; neón ilumina el acento; contraste marca los bordes.",
            "Glass blurs bars and panels; neon makes the accent glow; contrast strengthens the edges."
        ),
    )
}

fn buttons_tab(d: &Draft, en: bool, icon: &dyn Fn(&str, u32) -> String) -> String {
    let cards = BUTTONS
        .iter()
        .map(|(id, label, desc)| {
            format!(
                "<button type=\"button\" class=\"btn-style-card\" role=\"radio\" aria-checked=\"{}\" data-ut-buttons=\"{id}\"><span class=\"btn-style-demo\" data-btn-style=\"{id}\"><span class=\"hdr-btn demo-key\">{}</span><span class=\"hdr-btn demo-key on\">{}</span></span><span><b>{}</b><small>{}</small></span></button>",
                d.button_style == *id,
                icon("bell", 17),
                icon("smartphone", 17),
                md_esc(label),
                md_esc(desc),
            )
        })
        .collect::<String>();
    let note = if d.from.user {
        tr(
            en,
            "Los botones van con este tema.",
            "Buttons belong to this theme.",
        )
    } else {
        tr(
            en,
            "En un tema de fábrica, los botones se aplican a todos los temas de fábrica.",
            "On a built-in theme, buttons apply to every built-in theme.",
        )
    };
    format!("<p class=\"ut-hint\">{note}</p><div class=\"btn-style-gallery ut-btns\">{cards}</div>")
}

/// Vista previa con los colores, el acabado y los botones del borrador.
pub fn preview(d: &Draft) -> String {
    let c = &d.colors;
    let vars = [
        "bg", "bar", "panel", "panel2", "line", "line2", "text", "dim", "faint", "brand", "fg",
        "cursor",
    ]
    .iter()
    .map(|k| format!("--p-{k}:{};", get(c, k)))
    .chain((0..16).map(|i| format!("--p-a{i}:{};", get(c, &format!("pal{i}")))))
    .collect::<String>();
    format!(
        "<div class=\"ut-pv\" data-ut-pv data-treatment=\"{t}\" style=\"{vars}\" aria-hidden=\"true\">\
<div class=\"ut-pvbar\"><b>COMANDOS</b><span class=\"ut-pvk\"><span class=\"btn-style-card\"><span class=\"btn-style-demo\" data-btn-style=\"{b}\"><span class=\"hdr-btn demo-key\">+</span><span class=\"hdr-btn demo-key on\">⚙</span></span></span><span>14:52</span></span></div>\
<div class=\"ut-pvbody\"><div class=\"ut-pvside\"><span class=\"on\"><b>tienda-web</b></span><span>api-pagos</span><span class=\"d\">panel-admin</span><span class=\"f\">hace 2 h</span></div>\
<div class=\"ut-pvterm\"><div><span style=\"color:var(--p-a4)\">~/tienda-web</span> <span style=\"color:var(--p-a5)\">main</span> ❯ git status</div>\
<div style=\"color:var(--p-a2)\">+ pagos/api.ts</div><div style=\"color:var(--p-a1)\">- pagos/viejo.ts</div>\
<div style=\"color:var(--p-a3)\">⚠ 2 avisos de tipos</div><div><span style=\"color:var(--p-a6)\">●</span> Listo: 42 pruebas pasan</div>\
<div style=\"color:var(--p-a8)\">// probado en 3.1 s</div><div>❯ <i class=\"ut-cur\"></i></div></div></div></div>",
        t = d.treatment,
        b = d.button_style,
    )
}

/// La tarjeta abierta: vista previa, guardar y las tres pestañas.
pub fn editor(d: &Draft, en: bool, icon: &dyn Fn(&str, u32) -> String) -> String {
    let tabs = [
        ("colors", "Colores", "Colours"),
        ("finish", "Acabado", "Finish"),
        ("buttons", "Botones", "Buttons"),
    ]
    .iter()
    .map(|(id, es, en_name)| {
        format!(
            "<button type=\"button\" role=\"tab\" data-ut-tab=\"{id}\" aria-selected=\"{}\">{}</button>",
            d.tab == *id,
            tr(en, es, en_name)
        )
    })
    .collect::<String>();
    let body = match d.tab.as_str() {
        "finish" => finish_tab(d, en),
        "buttons" => buttons_tab(d, en, icon),
        _ => colors_tab(d, en),
    };
    let name = if d.from.user {
        d.from.label.clone()
    } else {
        format!("{}{}", d.from.label, tr(en, " (mío)", " (mine)"))
    };
    let factory_note = if d.from.user {
        String::new()
    } else {
        format!(
            "<p class=\"ut-hint\">{}</p>",
            md_esc(&if en {
                format!(
                    "«{}» is built in and never changes: your changes are saved as a new theme.",
                    d.from.label
                )
            } else {
                format!(
                    "«{}» es de fábrica y no cambia: tus cambios se guardan como tema nuevo.",
                    d.from.label
                )
            })
        )
    };
    let buttons = if d.from.user {
        format!(
            "<button type=\"button\" class=\"ut-btn\" data-ut-save=\"here\">{}</button><button type=\"button\" class=\"ut-btn ghost\" data-ut-save=\"new\">{}</button>",
            tr(en, "Guardar cambios", "Save changes"),
            tr(en, "Guardar como otro", "Save as another")
        )
    } else {
        format!(
            "<button type=\"button\" class=\"ut-btn\" data-ut-save=\"new\">{}</button>",
            tr(en, "Guardar como tema nuevo", "Save as a new theme")
        )
    };
    let delete = if d.from.user {
        format!(
            "<button type=\"button\" class=\"ut-btn ghost danger\" data-ut-delete>{}</button>",
            tr(en, "Borrar tema", "Delete theme")
        )
    } else {
        String::new()
    };
    let edited = d.edited();
    format!(
        "<div class=\"ut-open\" data-ut-open=\"{id}\"><div class=\"ut-head\"><b>{label}</b><span class=\"ut-pill\" data-ut-edited{hidden}>{ed}</span><button type=\"button\" class=\"ut-btn ghost\" data-ut-close>{close}</button></div>\
<div class=\"ut-body\"><div class=\"ut-col\">{pv}<div class=\"ut-save\">{factory_note}<input class=\"ut-name\" id=\"ut-name\" maxlength=\"48\" value=\"{name}\" aria-label=\"{nlabel}\"><div class=\"ut-actions\">{buttons}<button type=\"button\" class=\"ut-btn ghost\" data-ut-undo>{undo}</button>{delete}</div></div></div>\
<div class=\"ut-col\"><div class=\"ut-tabs\" role=\"tablist\">{tabs}</div><div class=\"ut-pane\">{body}</div><p class=\"ut-note\">{global}</p></div></div></div>",
        id = attr_esc(&d.from.id),
        label = md_esc(&d.from.label),
        hidden = if edited { "" } else { " hidden" },
        ed = tr(en, "editado", "edited"),
        close = tr(en, "Cerrar", "Close"),
        pv = preview(d),
        name = attr_esc(&name),
        nlabel = tr(en, "Nombre del tema", "Theme name"),
        undo = tr(en, "Deshacer cambios", "Undo changes"),
        global = tr(
            en,
            "Letra, tamaño, cursor y margen son tuyos y valen para todos los temas: están en la pestaña Terminal.",
            "Font, size, cursor and padding are yours for every theme: they live in the Terminal tab."
        ),
    )
}

/// Galería: los 9 de fábrica y los propios; la tarjeta del tema abierto se
/// despliega a lo ancho justo después de la suya.
pub fn gallery(
    prefs: &Value,
    current: &str,
    open: Option<&Draft>,
    en: bool,
    icon: &dyn Fn(&str, u32) -> String,
) -> String {
    let meta = metadata();
    let mut out = String::new();
    let mut index = 0usize;
    let mut card = |id: &str, label: &str, desc: &str, ico: &str, sw: [String; 3], mine: bool| {
        let open_here = open.is_some_and(|d| d.from.id == id);
        let edited = open.is_some_and(|d| d.from.id == id && d.edited());
        let pills = format!(
            "{}{}",
            if mine {
                format!(
                    " <span class=\"ut-pill\">{}</span>",
                    tr(en, "tuyo", "yours")
                )
            } else {
                String::new()
            },
            if edited {
                format!(
                    " <span class=\"ut-pill\">{}</span>",
                    tr(en, "editado", "edited")
                )
            } else {
                String::new()
            },
        );
        let swatches = sw
            .iter()
            .map(|c| format!("<i style=\"background:{c}\"></i>"))
            .collect::<String>();
        out.push_str(&format!(
            "<button type=\"button\" class=\"theme-card{}\" role=\"radio\" aria-checked=\"{}\" aria-expanded=\"{open_here}\" data-theme-id=\"{}\" data-index=\"{index}\">\n    <span class=\"theme-ico\">{}</span><span><b>{}</b>{pills}<small>{}</small></span>\n    <span class=\"theme-swatches\">{swatches}</span><span class=\"theme-check\">{}</span></button>",
            if mine { " mine" } else { "" },
            id == current,
            attr_esc(id),
            icon(ico, 18),
            md_esc(label),
            md_esc(desc),
            icon("check", 15),
        ));
        index += 1;
        if let Some(d) = open.filter(|_| open_here) {
            out.push_str(&editor(d, en, icon));
        }
    };
    for id in THEMES {
        let m = meta.get(*id).cloned().unwrap_or_default();
        let text = |k: &str| field(&m, k).to_string();
        let sw = m
            .get("sw")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let s = |i: usize| {
            sw.get(i)
                .and_then(Value::as_str)
                .unwrap_or("#000000")
                .to_string()
        };
        card(
            id,
            &text("label"),
            &text("desc"),
            &text("icon"),
            [s(0), s(1), s(2)],
            false,
        );
    }
    for t in list(prefs) {
        let c = t
            .get("colors")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let base_label = meta
            .get(field(&t, "base"))
            .map(|m| field(m, "label"))
            .unwrap_or_default()
            .to_string();
        let desc = if en {
            format!("yours · from {base_label}")
        } else {
            format!("tuyo · sale de {base_label}")
        };
        card(
            field(&t, "id"),
            field(&t, "label"),
            &desc,
            "pencil",
            [
                get(&c, "bg").into(),
                get(&c, "panel2").into(),
                get(&c, "brand").into(),
            ],
            true,
        );
    }
    out
}

/// Un id nuevo para un tema propio a partir de un número (p. ej. la hora).
pub fn new_id(seed: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut n = seed.max(1);
    let mut tail = Vec::new();
    while n > 0 && tail.len() < 24 {
        tail.push(DIGITS.get((n % 36) as usize).copied().unwrap_or(b'0'));
        n /= 36;
    }
    tail.reverse();
    format!(
        "{PREFIX}{}",
        String::from_utf8(tail).unwrap_or_else(|_| "x".into())
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn sample(id: &str) -> Value {
        json!({
            "id": id,
            "label": "  Noche cálida ",
            "base": "noche",
            "colors": factory_colors("noche").unwrap(),
            "treatment": "glass",
            "button_style": "arcade",
        })
    }

    #[test]
    fn every_factory_theme_has_complete_colors() {
        for id in THEMES {
            let c = factory_colors(id).unwrap();
            assert!(colors(&Value::Object(c)).is_some(), "{id}");
        }
        assert!(factory_colors("u-x").is_none());
    }

    #[test]
    fn validate_normalizes_and_rejects() {
        let mut t = sample("u-abc");
        t["colors"]["bg"] = json!("#0a0d13");
        let v = validate(&t).unwrap();
        assert_eq!(v["label"], "Noche cálida");
        assert_eq!(v["colors"]["bg"], "#0A0D13");
        for (key, bad) in [
            ("id", json!("noche")),
            ("id", json!("u-ABC")),
            ("label", json!("   ")),
            ("base", json!("u-abc")),
            ("treatment", json!("matte")),
            ("button_style", json!("round")),
        ] {
            let mut t = sample("u-abc");
            t[key] = bad;
            assert!(validate(&t).is_err(), "{key}");
        }
        let mut t = sample("u-abc");
        t["colors"]["pal"] = json!(["#000000"]);
        assert!(validate(&t).is_err());
        let mut t = sample("u-abc");
        t["colors"]["brand"] = json!("red");
        assert!(validate(&t).is_err());
    }

    #[test]
    fn upsert_replaces_by_id_and_caps_the_list() {
        let one = upsert(&Value::Null, &sample("u-a")).unwrap();
        let mut renamed = sample("u-a");
        renamed["label"] = json!("Otro");
        let same = upsert(&one, &renamed).unwrap();
        assert_eq!(same.as_array().unwrap().len(), 1);
        assert_eq!(same[0]["label"], "Otro");
        let mut many = Value::Array(vec![]);
        for i in 0..MAX_THEMES {
            many = upsert(&many, &sample(&format!("u-{i}"))).unwrap();
        }
        assert!(upsert(&many, &sample("u-zz")).is_err());
        assert_eq!(
            remove(&many, "u-3").as_array().unwrap().len(),
            MAX_THEMES - 1
        );
    }

    #[test]
    fn resolve_user_and_factory() {
        let prefs = json!({"user_themes": [sample("u-a")]});
        let r = resolve(&prefs, "u-a");
        assert!(r.user);
        assert_eq!(r.base, "noche");
        assert_eq!(r.treatment, "glass");
        assert_eq!(r.button_style.as_deref(), Some("arcade"));
        let f = resolve(&prefs, "neon");
        assert!(!f.user);
        assert_eq!(f.treatment, "neon");
        assert_eq!(f.button_style, None);
        assert_eq!(resolve(&prefs, "u-missing").id, "noche");
    }

    #[test]
    fn draft_tracks_edits() {
        let mut d = Draft::new(resolve(&Value::Null, "noche"), "tecla");
        assert!(!d.edited());
        assert_eq!(d.button_style, "tecla");
        d.button_style = "pixel".into();
        assert!(!d.edited(), "factory buttons are global");
        assert!(d.set_color("pal3", "#123456"));
        assert!(d.edited());
        assert!(!d.set_color("nope", "#123456"));
        assert!(!d.set_color("bg", "blue"));
        let saved = d.theme("u-new", "Mío");
        assert!(validate(&saved).is_ok());
    }

    #[test]
    fn contrast_marks() {
        let c = factory_colors("noche").unwrap();
        assert!(contrast(&c, "line").is_none());
        assert!(contrast(&c, "text").unwrap() > 7.0);
        assert!(contrast(&c, "bg").unwrap() > 7.0, "surface against text");
        assert_eq!(mark(2.0).0, "poor");
        assert_eq!(mark(3.5).0, "fair");
        assert_eq!(mark(4.6).0, "good");
    }

    #[test]
    fn gallery_opens_the_card_after_its_own() {
        let prefs = json!({"user_themes": [sample("u-a")]});
        let d = Draft::new(resolve(&prefs, "u-a"), "sutil");
        let html = gallery(&prefs, "u-a", Some(&d), false, &|n, _| format!("[{n}]"));
        let card = html.find("data-theme-id=\"u-a\"").unwrap();
        let open = html.find("data-ut-open=\"u-a\"").unwrap();
        assert!(card < open);
        assert!(html.contains("Guardar cambios"));
        assert!(html.contains("tuyo · sale de Noche Órbita"));
        let d = Draft::new(resolve(&prefs, "dia"), "sutil");
        let html = gallery(&prefs, "dia", Some(&d), false, &|n, _| format!("[{n}]"));
        assert!(html.contains("Guardar como tema nuevo"));
        assert!(!html.contains("data-ut-delete"));
        assert_eq!(html.matches("data-ut-color=").count(), 12 + 16);
    }

    #[test]
    fn new_ids_are_user_ids() {
        assert!(is_user_id(&new_id(1_795_225_760_000)));
        assert!(is_user_id(&new_id(0)));
    }
}
