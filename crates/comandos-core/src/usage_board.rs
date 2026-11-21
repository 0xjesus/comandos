//! Tablero de Consumos (Ajustes › Consumos): proyectos → sesiones y, por
//! cuenta, la semana dibujada como curva con su proyección hasta el reinicio.
//!
//! Nunca hay dinero. El peso de un turno para repartir el cupo de una cuenta
//! es una mezcla de sus tokens (la salida pesa más que la entrada, y leer de
//! la caché casi nada); con ese peso se estima qué parte de la semana se llevó
//! cada proyecto y la forma de la curva, cuyo último punto es el porcentaje
//! real que da el proveedor.
use crate::analytics_week::{TZ, account_id, project_of};
use chrono::{Datelike, NaiveDate, TimeZone, Timelike};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

/// Días que cubre la tabla y su tendencia.
pub const DAYS: usize = 14;
const HOUR: f64 = 3600.0;
const DAY: f64 = 86_400.0;
const WEEK: f64 = 7.0 * DAY;
/// Dos turnos separados por más de esto no cuentan como tiempo seguido.
const GAP_S: f64 = 900.0;

/// Un turno de la base (`usage_turns`): una respuesta del modelo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Turn {
    pub provider: String,
    pub account: String,
    pub git_root: String,
    pub pane_pwd: String,
    pub session: String,
    pub at: f64,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub model: String,
}

/// Un tramo (`usage_spans`): lo que tardó un turno pedido por la persona.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Span {
    pub provider: String,
    pub account: String,
    pub git_root: String,
    pub session: String,
    pub started: f64,
    pub finished: f64,
}

impl Turn {
    /// Tokens que se generaron o se leyeron por primera vez (sin la caché leída).
    pub fn new_tokens(&self) -> i64 {
        self.input.max(0) + self.output.max(0) + self.cache_write.max(0)
    }
    /// Peso del turno en el cupo: proporcional a lo que cuesta generarlo.
    pub fn weight(&self) -> f64 {
        self.input.max(0) as f64
            + 1.25 * self.cache_write.max(0) as f64
            + 0.1 * self.cache_read.max(0) as f64
            + 5.0 * self.output.max(0) as f64
    }
}

pub struct BoardInput<'a> {
    pub now: f64,
    /// Filas de cuota (`LimitsCache`): provider, account, window, scope,
    /// percent, resets_at.
    pub limits: &'a [Value],
    /// Cuentas de la columna de Consumos (color, nombre de la herramienta, plan).
    pub accounts: &'a [Value],
    pub turns: &'a [Turn],
    pub spans: &'a [Span],
}

fn tz() -> chrono_tz::Tz {
    TZ.parse().unwrap_or(chrono_tz::America::Mexico_City)
}

const DAYS_ES: [&str; 7] = ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"];
const DAYS_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn local(at: f64) -> Option<chrono::DateTime<chrono_tz::Tz>> {
    tz().timestamp_opt(at as i64, 0).single()
}

/// «jue 04:45» / «Thu 04:45».
pub fn label(at: f64) -> Value {
    match local(at) {
        Some(t) => {
            let d = t.weekday().num_days_from_monday() as usize;
            let hm = format!("{:02}:{:02}", t.hour(), t.minute());
            json!({
                "es": format!("{} {hm}", DAYS_ES.get(d).copied().unwrap_or_default()),
                "en": format!("{} {hm}", DAYS_EN.get(d).copied().unwrap_or_default()),
            })
        }
        None => Value::Null,
    }
}

fn day_of(at: f64) -> Option<NaiveDate> {
    local(at).map(|t| t.date_naive())
}

fn midnight(date: NaiveDate) -> Option<f64> {
    let civil = date.and_hms_opt(0, 0, 0)?;
    tz().from_local_datetime(&civil)
        .earliest()
        .map(|t| t.timestamp() as f64)
}

fn acc_id(provider: &str, account: &str) -> String {
    account_id(provider, &json!(account)).unwrap_or_else(|_| format!("{provider}:main"))
}

fn project(git_root: &str, pane_pwd: &str) -> String {
    let path = if git_root.is_empty() { pane_pwd } else { git_root };
    project_of(&json!(path)).unwrap_or_else(|_| "Sin carpeta".into())
}

/// Ventana semanal de una cuenta: la fila `7d` sin alcance (la general).
#[derive(Debug, Clone, PartialEq)]
struct Window {
    percent: f64,
    reset: f64,
    /// Última lectura guardada: el proveedor no respondió ahora.
    stale: bool,
}

fn windows(limits: &[Value], win: &str) -> HashMap<String, Window> {
    let mut out = HashMap::new();
    for row in limits {
        let get = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or_default();
        if get("window") != win || !get("scope").is_empty() {
            continue;
        }
        let percent = row.get("percent").and_then(Value::as_f64);
        let reset = row.get("resets_at").and_then(Value::as_f64).unwrap_or(0.0);
        if let Some(percent) = percent {
            out.insert(
                acc_id(get("provider"), get("account")),
                Window {
                    percent,
                    reset,
                    stale: row.get("stale").and_then(Value::as_bool) == Some(true),
                },
            );
        }
    }
    out
}

/// La semana de una cuenta: curva estimada, proyección y ejes.
fn week_curve(w: &Window, turns: &[&Turn], now: f64) -> Value {
    let reset = if w.reset > now { w.reset } else { now };
    let start = reset - WEEK;
    let inside: Vec<&&Turn> = turns.iter().filter(|t| t.at >= start && t.at <= now).collect();
    let total: f64 = inside.iter().map(|t| t.weight()).sum();
    // Una cubeta por hora; el último punto es siempre el porcentaje real.
    let mut buckets: BTreeMap<i64, f64> = BTreeMap::new();
    for t in &inside {
        *buckets.entry(((t.at - start) / HOUR) as i64).or_default() += t.weight();
    }
    let mut curve = vec![json!([start, 0.0])];
    let mut acc = 0.0;
    if total > 0.0 {
        for (hour, weight) in &buckets {
            acc += weight;
            let at = (start + (*hour as f64 + 1.0) * HOUR).min(now);
            curve.push(json!([at, round1(acc / total * w.percent)]));
        }
    }
    curve.push(json!([now, round1(w.percent)]));
    // Ritmo de las últimas 24 h (o desde que empezó la semana).
    let from = (now - DAY).max(start);
    let recent: f64 = inside.iter().filter(|t| t.at >= from).map(|t| t.weight()).sum();
    let pace = if total > 0.0 && now > from {
        recent / total * w.percent / ((now - from) / HOUR)
    } else if now > start {
        w.percent / ((now - start) / HOUR)
    } else {
        0.0
    };
    let at_reset = w.percent + pace * (reset - now) / HOUR;
    let out = (pace > 0.0 && w.percent < 100.0)
        .then(|| now + (100.0 - w.percent) / pace * HOUR)
        .filter(|at| *at < reset);
    let out = if w.percent >= 100.0 { Some(now) } else { out };
    let mut axis = Vec::new();
    if let Some(mut d) = day_of(start) {
        for _ in 0..8 {
            d = d.succ_opt().unwrap_or(d);
            if let Some(at) = midnight(d).filter(|at| *at > start && *at < reset) {
                let i = d.weekday().num_days_from_monday() as usize;
                axis.push(json!({
                    "at": at,
                    "es": DAYS_ES.get(i).copied().unwrap_or_default(),
                    "en": DAYS_EN.get(i).copied().unwrap_or_default(),
                }));
            }
        }
    }
    json!({
        "percent": round1(w.percent),
        "stale": w.stale,
        "start": start,
        "reset": reset,
        "resetLabel": label(reset),
        "curve": curve,
        "estimated": total > 0.0,
        "pace": (pace * 100.0).round() / 100.0,
        "atReset": round1(at_reset.min(999.0)),
        "out": out,
        "outLabel": out.map(label).unwrap_or(Value::Null),
        "axis": axis,
    })
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

#[derive(Default)]
struct Tally {
    new_tokens: i64,
    turns: usize,
    responses: usize,
    span_secs: f64,
    stamps: Vec<f64>,
    weight_by_acc: HashMap<String, f64>,
    spark: [i64; DAYS],
    sessions: std::collections::BTreeSet<String>,
    first: f64,
    last: f64,
    accounts: HashMap<String, usize>,
    models: HashMap<String, usize>,
    by_hour: BTreeMap<i64, usize>,
}

impl Tally {
    fn hours(&self) -> f64 {
        if self.turns > 0 {
            return self.span_secs / HOUR;
        }
        // Sin tramos: tiempo seguido entre respuestas (huecos de hasta 15 min).
        let mut s = self.stamps.clone();
        s.sort_by(f64::total_cmp);
        s.windows(2)
            .map(|w| {
                let gap = w.get(1).copied().unwrap_or(0.0) - w.first().copied().unwrap_or(0.0);
                if gap <= GAP_S { gap } else { 0.0 }
            })
            .sum::<f64>()
            / HOUR
    }
    fn top<'a>(m: &'a HashMap<String, usize>) -> &'a str {
        m.iter()
            .max_by_key(|(k, n)| (**n, std::cmp::Reverse((*k).clone())))
            .map(|(k, _)| k.as_str())
            .unwrap_or_default()
    }
}

/// El tablero completo (ver el módulo).
pub fn build(input: &BoardInput<'_>) -> Value {
    let now = input.now;
    let week = windows(input.limits, "7d");
    let block = windows(input.limits, "5h");
    let first_day = day_of(now).map(|d| d - chrono::Days::new(DAYS as u64 - 1));
    let day_index = |at: f64| -> Option<usize> {
        let d = day_of(at)?;
        let i = d.signed_duration_since(first_day?).num_days();
        (0..DAYS as i64).contains(&i).then_some(i as usize)
    };
    let since = first_day.and_then(midnight).unwrap_or(now - DAYS as f64 * DAY);

    // Totales semanales por cuenta para repartir su porcentaje.
    let mut acc_turns: HashMap<String, Vec<&Turn>> = HashMap::new();
    for t in input.turns {
        acc_turns.entry(acc_id(&t.provider, &t.account)).or_default().push(t);
    }
    let window_of = |acc: &str| week.get(acc).map(|w| (w.reset.max(now) - WEEK, w));
    let acc_total: HashMap<String, f64> = acc_turns
        .iter()
        .filter_map(|(acc, ts)| {
            let (start, _) = window_of(acc)?;
            Some((
                acc.clone(),
                ts.iter().filter(|t| t.at >= start).map(|t| t.weight()).sum(),
            ))
        })
        .collect();

    // Proyecto → sesión.
    let mut projects: BTreeMap<String, (Tally, BTreeMap<String, Tally>)> = BTreeMap::new();
    let mut session_project: HashMap<String, String> = HashMap::new();
    for t in input.turns.iter().filter(|t| t.at >= since) {
        let proj = project(&t.git_root, &t.pane_pwd);
        let acc = acc_id(&t.provider, &t.account);
        let sess = if t.session.is_empty() { format!("{proj}·{acc}") } else { t.session.clone() };
        session_project.insert(sess.clone(), proj.clone());
        let in_week = window_of(&acc).is_some_and(|(start, _)| t.at >= start);
        let (p, sessions) = projects.entry(proj).or_default();
        let s = sessions.entry(sess.clone()).or_default();
        for tally in [p, s] {
            tally.new_tokens += t.new_tokens();
            tally.responses += 1;
            tally.stamps.push(t.at);
            tally.sessions.insert(sess.clone());
            if in_week {
                *tally.weight_by_acc.entry(acc.clone()).or_default() += t.weight();
            }
            if let Some(i) = day_index(t.at)
                && let Some(slot) = tally.spark.get_mut(i)
            {
                *slot += t.new_tokens();
            }
            tally.first = if tally.first == 0.0 { t.at } else { tally.first.min(t.at) };
            tally.last = tally.last.max(t.at);
            *tally.accounts.entry(acc.clone()).or_default() += 1;
            if !t.model.is_empty() {
                *tally.models.entry(t.model.clone()).or_default() += 1;
            }
        }
    }
    for sp in input.spans.iter().filter(|s| s.finished >= since) {
        let proj = session_project
            .get(&sp.session)
            .cloned()
            .unwrap_or_else(|| project(&sp.git_root, ""));
        let Some((p, sessions)) = projects.get_mut(&proj) else {
            continue;
        };
        let secs = (sp.finished - sp.started).clamp(0.0, 6.0 * HOUR);
        let hour = (sp.finished / HOUR) as i64;
        p.turns += 1;
        p.span_secs += secs;
        if let Some(s) = sessions.get_mut(&sp.session) {
            s.turns += 1;
            s.span_secs += secs;
            *s.by_hour.entry(hour).or_default() += 1;
        }
    }

    let share = |t: &Tally| -> Map<String, Value> {
        t.weight_by_acc
            .iter()
            .filter_map(|(acc, weight)| {
                let total = acc_total.get(acc).copied().filter(|v| *v > 0.0)?;
                let w = week.get(acc)?;
                Some((acc.clone(), json!(round1(weight / total * w.percent))))
            })
            .filter(|(_, v)| v.as_f64().is_some_and(|v| v >= 0.1))
            .collect()
    };
    let turns_of = |t: &Tally| if t.turns > 0 { t.turns } else { t.responses };
    let mut project_rows: Vec<Value> = projects
        .iter()
        .map(|(name, (p, sessions))| {
            let mut rows: Vec<Value> = sessions
                .iter()
                .map(|(id, s)| {
                    json!({
                        "id": id,
                        "account": Tally::top(&s.accounts),
                        "model": Tally::top(&s.models),
                        "started": s.first,
                        "last": s.last,
                        "lastLabel": label(s.last),
                        "newTokens": s.new_tokens,
                        "turns": turns_of(s),
                        "hours": round1(s.hours()),
                        "share": share(s),
                        "spark": s.spark,
                        "byHour": s.by_hour.iter().rev().take(48).rev()
                            .map(|(h, n)| json!([*h as f64 * HOUR, n])).collect::<Vec<_>>(),
                    })
                })
                .collect();
            rows.sort_by(|a, b| {
                b["last"].as_f64().unwrap_or(0.0).total_cmp(&a["last"].as_f64().unwrap_or(0.0))
            });
            json!({
                "name": name,
                "newTokens": p.new_tokens,
                "turns": turns_of(p),
                "hours": round1(p.hours()),
                "sessions": p.sessions.len(),
                "share": share(p),
                "spark": p.spark,
                "last": p.last,
                "rows": rows,
            })
        })
        .collect();
    // Primero lo que más pesa en la semana; luego lo más reciente.
    project_rows.sort_by(|a, b| {
        let w = |v: &Value| {
            v["share"]
                .as_object()
                .map(|m| m.values().filter_map(Value::as_f64).sum::<f64>())
                .unwrap_or(0.0)
        };
        w(b).total_cmp(&w(a)).then(
            b["newTokens"].as_i64().unwrap_or(0).cmp(&a["newTokens"].as_i64().unwrap_or(0)),
        )
    });

    // Cuentas con cupo semanal, en el orden de la columna de Consumos.
    let mut accounts = Vec::new();
    for a in input.accounts {
        let id = a.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(w) = week.get(&id) else {
            continue;
        };
        let turns = acc_turns.get(&id).map(Vec::as_slice).unwrap_or_default();
        let h5 = block.get(&id).filter(|b| b.reset > now).map(|b| {
            json!({"percent": round1(b.percent), "reset": b.reset, "resetLabel": label(b.reset)})
        });
        accounts.push(json!({
            "id": id,
            "alias": a.get("alias").cloned().unwrap_or(Value::Null),
            "cli": a.get("cli").cloned().unwrap_or(Value::Null),
            "color": a.get("color").cloned().unwrap_or(Value::Null),
            "plan": a.get("plan").cloned().unwrap_or(Value::Null),
            "week": week_curve(w, turns, now),
            "h5": h5,
        }));
    }

    // Dónde hay cupo: la cuenta con más semana libre y sin el bloque agotado.
    let mut free: Vec<Value> = accounts
        .iter()
        .filter_map(|a| {
            let w = a.get("week")?;
            let left = 100.0 - w["percent"].as_f64()?;
            let blocked = a["h5"]["percent"].as_f64().is_some_and(|p| p >= 100.0);
            Some(json!({
                "account": a["id"],
                "left": round1(left.max(0.0)),
                "blocked": blocked,
                "out": w["out"],
                "outLabel": w["outLabel"],
                "resetLabel": w["resetLabel"],
            }))
        })
        .collect();
    free.sort_by(|a, b| {
        let key = |v: &Value| (!v["blocked"].as_bool().unwrap_or(false), v["left"].as_f64().unwrap_or(0.0));
        let (ba, la) = key(a);
        let (bb, lb) = key(b);
        bb.cmp(&ba).then(lb.total_cmp(&la))
    });

    let days: Vec<Value> = (0..DAYS)
        .filter_map(|i| {
            let d = first_day? + chrono::Days::new(i as u64);
            let w = d.weekday().num_days_from_monday() as usize;
            Some(json!({
                "date": d.to_string(),
                "es": DAYS_ES.get(w).copied().unwrap_or_default(),
                "en": DAYS_EN.get(w).copied().unwrap_or_default(),
            }))
        })
        .collect();
    json!({
        "now": now,
        "days": days,
        "accounts": accounts,
        "projects": project_rows,
        "free": free,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const NOW: f64 = 1_795_200_000.0; // vie 20 nov 2026, 12:40 en Ciudad de México

    fn turn(acc: &str, proj: &str, sess: &str, at: f64, out: i64) -> Turn {
        Turn {
            provider: "claude".into(),
            account: acc.into(),
            git_root: format!("/srv/{proj}"),
            session: sess.into(),
            at,
            input: 100,
            output: out,
            cache_read: 1000,
            cache_write: 10,
            model: "claude-opus-5-5".into(),
            ..Turn::default()
        }
    }

    fn limits() -> Vec<Value> {
        vec![
            json!({"provider":"claude","account":"main","window":"7d","scope":"","percent":60.0,"resets_at":NOW + 2.0 * DAY}),
            json!({"provider":"claude","account":"main","window":"7d","scope":"Fable","percent":30.0,"resets_at":NOW + 2.0 * DAY}),
            json!({"provider":"claude","account":"main","window":"5h","scope":"","percent":100.0,"resets_at":NOW + HOUR}),
            json!({"provider":"claude","account":"work","window":"7d","scope":"","percent":10.0,"resets_at":NOW + 5.0 * DAY}),
        ]
    }

    fn accounts() -> Vec<Value> {
        vec![
            json!({"id":"claude:main","alias":"main","cli":"Claude","color":"#8B7CFF"}),
            json!({"id":"claude:work","alias":"work","cli":"Claude","color":"#2fd3c0"}),
            json!({"id":"opencode:main","alias":"main","cli":"OpenCode"}),
        ]
    }

    #[test]
    fn shares_split_the_weekly_percent_by_weight() {
        let turns = vec![
            turn("main", "web", "s1", NOW - 3.0 * HOUR, 300),
            turn("main", "api", "s2", NOW - 2.0 * HOUR, 100),
            turn("main", "api", "s2", NOW - 30.0 * DAY, 100),
            turn("work", "api", "s3", NOW - HOUR, 50),
        ];
        let spans = vec![Span {
            provider: "claude".into(),
            account: "main".into(),
            session: "s2".into(),
            started: NOW - 2.0 * HOUR - 600.0,
            finished: NOW - 2.0 * HOUR,
            ..Span::default()
        }];
        let b = build(&BoardInput {
            now: NOW,
            limits: &limits(),
            accounts: &accounts(),
            turns: &turns,
            spans: &spans,
        });
        let projects = b["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2, "the 30-day-old turn is out");
        let web = projects.iter().find(|p| p["name"] == "web").unwrap();
        let api = projects.iter().find(|p| p["name"] == "api").unwrap();
        let main = web["share"]["claude:main"].as_f64().unwrap() + api["share"]["claude:main"].as_f64().unwrap();
        assert!((main - 60.0).abs() < 0.2, "{main}");
        assert!(web["share"]["claude:main"].as_f64().unwrap() > api["share"]["claude:main"].as_f64().unwrap());
        assert_eq!(api["turns"], 1, "spans count as turns");
        assert_eq!(api["hours"], 0.2);
        assert_eq!(web["turns"], 1, "without spans, responses");
        assert_eq!(api["sessions"], 2);
        assert_eq!(web["newTokens"], 410);
        assert_eq!(web["spark"].as_array().unwrap().len(), DAYS);
        assert_eq!(web["spark"][DAYS - 1], 410);
        assert_eq!(projects[0]["name"], "web", "the heaviest first");
    }

    #[test]
    fn a_stored_reading_is_marked_stale() {
        let mut l = limits();
        l[0]["stale"] = json!(true);
        let b = build(&BoardInput {
            now: NOW,
            limits: &l,
            accounts: &accounts(),
            turns: &[],
            spans: &[],
        });
        assert_eq!(b["accounts"][0]["week"]["stale"], true);
        assert_eq!(b["accounts"][1]["week"]["stale"], false);
        assert_eq!(b["accounts"][0]["week"]["estimated"], false);
    }

    #[test]
    fn accounts_get_a_curve_ending_at_the_real_percent() {
        let turns = vec![
            turn("main", "web", "s1", NOW - 4.0 * DAY, 300),
            turn("main", "web", "s1", NOW - 2.0 * HOUR, 300),
        ];
        let b = build(&BoardInput {
            now: NOW,
            limits: &limits(),
            accounts: &accounts(),
            turns: &turns,
            spans: &[],
        });
        let accs = b["accounts"].as_array().unwrap();
        assert_eq!(accs.len(), 2, "only accounts with a weekly window");
        let w = &accs[0]["week"];
        let curve = w["curve"].as_array().unwrap();
        assert_eq!(curve.first().unwrap()[1], 0.0);
        assert_eq!(curve.last().unwrap(), &json!([NOW, 60.0]));
        assert_eq!(w["reset"], NOW + 2.0 * DAY);
        assert!(w["estimated"].as_bool().unwrap());
        assert!(w["out"].as_f64().unwrap() > NOW, "half the week in the last day runs out");
        assert!(w["out"].as_f64().unwrap() < NOW + 2.0 * DAY);
        assert!(!w["axis"].as_array().unwrap().is_empty());
        assert_eq!(accs[0]["h5"]["percent"], 100.0);
        // Free room: the blocked 5 h account goes last.
        assert_eq!(b["free"][0]["account"], "claude:work");
        assert_eq!(b["free"][0]["left"], 90.0);
    }

    #[test]
    fn labels_are_local_weekday_and_time() {
        let l = label(NOW);
        assert_eq!(l["es"], "vie 12:40");
        assert_eq!(l["en"], "Fri 12:40");
    }
}
