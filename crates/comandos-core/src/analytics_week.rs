//! Portable account-separated eight-day analytics; supplied rows and time only.
use crate::{
    allocation::{
        Error, Result, add_integers, default, divide_number, divide_positive, error, integer,
        integer_float, integer_value, number, numeric, numeric_add, numeric_cmp, numeric_sub,
        object, pyfloat, pymax, pymin, required, round_digits, round_int, round_value, string,
    },
    json::truthy,
};
use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike,
};
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap},
};
pub const TZ: &str = "America/Mexico_City";
pub const WINDOW_DAYS: i64 = 8;
pub const GAP_S: f64 = 900.;
pub const WASTE_CYCLES: usize = 4;
const PROVIDERS: [&str; 4] = ["claude", "codex", "grok", "agy"];
const CLI: [&str; 4] = ["Claude", "Codex", "Grok", "Antigravity"];
const EXTRA_COLORS: [&str; 5] = ["#2fd3c0", "#FF6B5B", "#FFAE1A", "#FF9AD5", "#9AA6BF"];
const WD: [&str; 7] = ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"];
const MON: [&str; 12] = [
    "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic",
];
pub struct WeekInput<'a> {
    pub now: f64,
    pub offset: i64,
    pub limits: &'a [Value],
    /// Turnos y tramos de la ventana ya reducidos (`WeekRows`).
    pub rows: &'a WeekRows,
    pub snapshots: &'a [Value],
    pub records: &'a [Value],
    pub tz_name: &'a str,
}
pub fn account_of(account: &Value) -> Result<String> {
    let alias = if truthy(account) {
        string(account)?
    } else {
        ""
    };
    let alias = alias.trim_matches(python_whitespace);
    Ok(if alias.is_empty() || alias == "unknown" {
        "main"
    } else {
        alias
    }
    .into())
}
pub fn account_id(provider: &str, account: &Value) -> Result<String> {
    Ok(format!("{provider}:{}", account_of(account)?))
}
fn python_whitespace(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}
pub fn project_of(path: &Value) -> Result<String> {
    let path = if truthy(path) { string(path)? } else { "" }.trim_end_matches('/');
    Ok(if path.is_empty() {
        "Sin carpeta"
    } else {
        path.rsplit('/').next().expect("path segment")
    }
    .into())
}
pub fn pomodoro_project(name: &Value) -> Result<String> {
    let name = if truthy(name) {
        name.as_str()
            .ok_or_else(|| error("TypeError", "proyecto sin texto"))?
    } else {
        ""
    };
    let mut start = None;
    let mut cut = name.len();
    for (i, c) in name.char_indices() {
        if python_whitespace(c) {
            if start.is_none() {
                start = Some(i)
            }
        } else {
            if matches!(c, '⎇' | '⫽')
                && let Some(at) = start
            {
                cut = at;
                break;
            }
            start = None;
        }
    }
    let name = name[..cut].trim_matches(python_whitespace);
    Ok(if name.is_empty() {
        "Sin proyecto"
    } else {
        name
    }
    .into())
}
pub fn fmt_left(seconds: &Value) -> Result<String> {
    let raw = integer(seconds)?;
    let raw = if raw.starts_with('-') { "0" } else { &raw };
    let (days, rest) = divide_positive(raw, 86400);
    let (h, m) = (rest / 3600, rest % 3600 / 60);
    Ok(if days != "0" {
        format!("{days}d {h}h")
    } else if h != 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m} min")
    })
}
fn zone(name: &str) -> Result<chrono_tz::Tz> {
    name.parse().map_err(|_| {
        error(
            "ZoneInfoNotFoundError",
            format!("zona horaria inválida: {name}"),
        )
    })
}
#[derive(Clone, Copy)]
struct LocalStamp {
    civil: NaiveDateTime,
    epoch: f64,
}
fn from_timestamp(at: f64, tz: chrono_tz::Tz) -> Result<LocalStamp> {
    if !at.is_finite() {
        return Err(error(
            if at.is_nan() {
                "ValueError"
            } else {
                "OverflowError"
            },
            "timestamp no finito",
        ));
    }
    let floor = at.floor();
    if floor < i64::MIN as f64 || floor >= i64::MAX as f64 {
        return Err(error("OverflowError", "timestamp fuera de rango"));
    }
    let micro = ((at - floor) * 1e6).round_ties_even() as u32;
    let (sec, micro) = if micro == 1000000 {
        (
            (floor as i64)
                .checked_add(1)
                .ok_or_else(|| error("OverflowError", "timestamp fuera de rango"))?,
            0,
        )
    } else {
        (floor as i64, micro)
    };
    let utc = DateTime::from_timestamp(sec, micro * 1000)
        .ok_or_else(|| error("ValueError", "fecha fuera de rango"))?;
    if !(1..=9999).contains(&utc.year()) {
        return Err(error("ValueError", "año fuera de rango"));
    }
    let civil = utc.with_timezone(&tz).naive_local();
    if !(1..=9999).contains(&civil.year()) {
        return Err(error("ValueError", "año fuera de rango"));
    }
    Ok(LocalStamp {
        civil,
        epoch: sec as f64 + micro as f64 / 1e6,
    })
}
// Python's same-zone datetime ordering compares civil fields, including at DST folds.
// A freshly combined midnight uses fold=0, selecting the pre-gap offset if absent.
fn local_midnight(date: NaiveDate, tz: chrono_tz::Tz) -> Result<LocalStamp> {
    let civil = date.and_hms_opt(0, 0, 0).expect("midnight");
    if !(1..=9999).contains(&civil.year()) {
        return Err(error("ValueError", "año fuera de rango"));
    }
    let offset = match tz.offset_from_local_datetime(&civil) {
        LocalResult::Single(o) => o.fix().local_minus_utc(),
        LocalResult::Ambiguous(a, b) => a.fix().local_minus_utc().max(b.fix().local_minus_utc()),
        LocalResult::None => {
            let mut prior = civil;
            loop {
                prior = prior
                    .checked_sub_signed(Duration::minutes(1))
                    .ok_or_else(|| error("OverflowError", "medianoche fuera de rango"))?;
                match tz.offset_from_local_datetime(&prior) {
                    LocalResult::Single(o) => break o.fix().local_minus_utc(),
                    LocalResult::Ambiguous(a, b) => {
                        break a.fix().local_minus_utc().max(b.fix().local_minus_utc());
                    }
                    LocalResult::None => {}
                }
            }
        }
    };
    Ok(LocalStamp {
        civil,
        epoch: civil.and_utc().timestamp() as f64 - offset as f64,
    })
}
pub fn fmt_reset(ts: f64, tz: &str) -> Result<String> {
    fmt_reset_in(ts, zone(tz)?)
}
fn fmt_reset_in(ts: f64, tz: chrono_tz::Tz) -> Result<String> {
    let t = from_timestamp(ts, tz)?.civil;
    Ok(format!(
        "{} {} {}, {:02}:{:02}",
        WD[t.weekday().num_days_from_monday() as usize],
        t.day(),
        MON[t.month0() as usize],
        t.hour(),
        t.minute()
    ))
}
pub fn window_dates(now: f64, offset: i64, tz: &str) -> Result<Vec<NaiveDate>> {
    window_dates_in(now, offset, zone(tz)?)
}
fn window_dates_in(now: f64, offset: i64, tz: chrono_tz::Tz) -> Result<Vec<NaiveDate>> {
    let days = offset
        .checked_mul(WINDOW_DAYS)
        .and_then(Duration::try_days)
        .ok_or_else(|| error("OverflowError", "ventana fuera de rango"))?;
    let end = from_timestamp(now, tz)?
        .civil
        .date()
        .checked_add_signed(days)
        .ok_or_else(|| error("OverflowError", "fecha fuera de rango"))?;
    (0..WINDOW_DAYS)
        .map(|i| {
            let d = end
                .checked_sub_signed(Duration::days(i))
                .ok_or_else(|| error("OverflowError", "fecha fuera de rango"))?;
            if !(1..=9999).contains(&d.year()) {
                return Err(error("OverflowError", "fecha fuera de rango"));
            }
            Ok(d)
        })
        .collect()
}
pub fn day_rows(dates: &[NaiveDate]) -> Vec<Value> {
    dates
        .iter()
        .enumerate()
        .map(|(i, d)| {
            json!([
                d.to_string(),
                WD[d.weekday().num_days_from_monday() as usize],
                if i == 0 || d.day() == 1 {
                    format!("{} {}", d.day(), MON[d.month0() as usize])
                } else {
                    d.day().to_string()
                }
            ])
        })
        .collect()
}
pub fn week_label(dates: &[NaiveDate]) -> Result<String> {
    let b = dates
        .first()
        .ok_or_else(|| error("IndexError", "fechas vacías"))?;
    let a = dates.last().expect("first exists");
    Ok(if a.month() == b.month() {
        format!("{} – {} {}", a.day(), b.day(), MON[b.month0() as usize])
    } else {
        format!(
            "{} {} – {} {}",
            a.day(),
            MON[a.month0() as usize],
            b.day(),
            MON[b.month0() as usize]
        )
    })
}
fn provider(row: &Value, required_key: bool) -> Result<Option<&str>> {
    let value = if required_key {
        required(row, "provider")?
    } else {
        &row["provider"]
    };
    Ok(value.as_str().filter(|p| PROVIDERS.contains(p)))
}
#[derive(Clone)]
struct Interval {
    start: f64,
    end: f64,
    tokens: Tokens,
}
/// Los tokens de un intervalo (`int(...)` del Python, sin tope). Casi siempre
/// caben en un `i64`: así no hace falta un texto en el montón por cada turno.
#[derive(Clone)]
enum Tokens {
    Small(i64),
    Big(Box<str>),
}
impl Tokens {
    /// Desde la grafía canónica de `integer` (sin ceros a la izquierda ni `-0`),
    /// que es la de `i64::to_string` cuando cabe.
    fn from_integer(raw: String) -> Self {
        raw.parse()
            .map_or_else(|_| Self::Big(raw.into_boxed_str()), Self::Small)
    }
    fn text(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Small(n) => n.to_string().into(),
            Self::Big(raw) => (&**raw).into(),
        }
    }
    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Small(a), Self::Small(b)) => a.cmp(b),
            _ => {
                crate::json::number_cmp(&integer_value(&self.text()), &integer_value(&other.text()))
            }
        }
    }
}
type GroupKey = (String, String);
struct MergedInterval {
    start: f64,
    end: f64,
    tokens: Vec<(f64, Tokens)>,
}
/// Los intervalos de una clase de filas (turnos o tramos) por `(cuenta, proyecto)`,
/// en el orden en que aparece cada grupo, y el primer error de esa clase.
#[derive(Default)]
struct IntervalGroups {
    groups: Vec<(GroupKey, Vec<Interval>)>,
    index: HashMap<GroupKey, usize>,
    error: Option<Error>,
}
impl IntervalGroups {
    /// Una fila del bucle de `_intervals`: tras el primer error el Python ya no
    /// sigue, así que las filas siguientes se ignoran.
    fn push(&mut self, row: &Value, is_span: bool) {
        if self.error.is_some() {
            return;
        }
        match interval(row, is_span) {
            Ok(Some((key, item))) => match self.index.get(&key) {
                Some(&i) => {
                    if let Some((_, items)) = self.groups.get_mut(i) {
                        items.push(item)
                    }
                }
                None => {
                    self.index.insert(key.clone(), self.groups.len());
                    self.groups.push((key, vec![item]));
                }
            },
            Ok(None) => {}
            Err(e) => self.error = Some(e),
        }
    }
    fn items(&self, key: &GroupKey) -> &[Interval] {
        self.index
            .get(key)
            .and_then(|&i| self.groups.get(i))
            .map_or(&[], |(_, items)| items.as_slice())
    }
}
fn interval(row: &Value, is_span: bool) -> Result<Option<(GroupKey, Interval)>> {
    let Some(p) = provider(row, true)? else {
        return Ok(None);
    };
    let project = if is_span {
        project_of(&row["git_root"])?
    } else {
        project_of(default(&row["git_root"], &row["pane_pwd"]))?
    };
    let key = (account_id(p, &row["account"])?, project);
    let end = pyfloat(required(row, "finished")?)?;
    let start = pymin(
        end,
        pyfloat(if is_span {
            required(row, "started")?
        } else {
            default(&row["started"], &row["finished"])
        })?,
    );
    let tokens = if is_span {
        Tokens::Small(0)
    } else {
        Tokens::from_integer(integer(default(&row["tokens"], &json!(0)))?)
    };
    Ok(Some((key, Interval { start, end, tokens })))
}
/// Turnos y tramos de la semana reducidos fila a fila a lo que `build_week` y
/// `sidebar_accounts` leen de ellos: los intervalos por grupo y, de los turnos
/// de los últimos 7 días, el consumo por cuenta. Así no hace falta tener cada
/// turno de la ventana (decenas de miles) como objeto JSON a la vez.
///
/// El resultado no depende del orden en que se mezclen turnos y tramos: el Python
/// recorre todos los turnos y después todos los tramos, y eso se reconstruye al
/// leer. `now` es el de `sidebar_accounts` (la ventana de 7 días).
pub struct WeekRows {
    intervals: Intervals,
    sidebar: SidebarTurns,
}
/// Los intervalos de turnos y de tramos por separado (ver `intervals`).
#[derive(Default)]
struct Intervals {
    turns: IntervalGroups,
    spans: IntervalGroups,
}
impl WeekRows {
    pub fn new(now: f64) -> Self {
        Self {
            intervals: Intervals::default(),
            sidebar: SidebarTurns::new(now),
        }
    }
    /// Todas las filas a la vez (la forma de las pruebas y de los datos de ejemplo).
    pub fn from_rows(turns: &[Value], spans: &[Value], now: f64) -> Self {
        let mut rows = Self::new(now);
        for row in turns {
            rows.push_turn(row);
        }
        for row in spans {
            rows.push_span(row);
        }
        rows
    }
    pub fn push_turn(&mut self, row: &Value) {
        self.intervals.turns.push(row, false);
        self.sidebar.push(row);
    }
    pub fn push_span(&mut self, row: &Value) {
        self.intervals.spans.push(row, true);
    }
}
impl Intervals {
    /// `_intervals(turns, spans)`: los grupos de los turnos en su orden y después
    /// los que solo tienen tramos; en cada grupo, los turnos antes que los tramos.
    /// Cada grupo se copia al pedirlo (lo ordena quien lo recibe), no todos a la vez.
    fn intervals(&self) -> Result<impl Iterator<Item = (&GroupKey, Vec<Interval>)> + '_> {
        if let Some(e) = self.turns.error.as_ref().or(self.spans.error.as_ref()) {
            return Err(e.clone());
        }
        let turns = self.turns.groups.iter().map(|(key, items)| {
            let mut all = Vec::with_capacity(items.len() + self.spans.items(key).len());
            all.extend_from_slice(items);
            all.extend_from_slice(self.spans.items(key));
            (key, all)
        });
        let spans = self
            .spans
            .groups
            .iter()
            .filter(|(key, _)| !self.turns.index.contains_key(key))
            .map(|(key, items)| (key, items.clone()));
        Ok(turns.chain(spans))
    }
}
pub fn sessions(turns: &[Value], spans: &[Value], tz: &str) -> Result<Vec<Value>> {
    let mut rows = Intervals::default();
    for row in turns {
        rows.turns.push(row, false);
    }
    for row in spans {
        rows.spans.push(row, true);
    }
    sessions_in(&rows, zone(tz)?)
}
fn sessions_in(rows: &Intervals, tz: chrono_tz::Tz) -> Result<Vec<Value>> {
    let mut out = vec![];
    for ((acc, proj), mut items) in rows.intervals()? {
        items.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.end.partial_cmp(&b.end).unwrap_or(Ordering::Equal))
                .then_with(|| a.tokens.compare(&b.tokens))
        });
        let mut merged: Vec<MergedInterval> = vec![];
        for item in items {
            if let Some(last) = merged
                .last_mut()
                .filter(|last| item.start - last.end <= GAP_S)
            {
                last.end = pymax(last.end, item.end);
                last.tokens.push((item.end, item.tokens));
            } else {
                merged.push(MergedInterval {
                    start: item.start,
                    end: item.end,
                    tokens: vec![(item.end, item.tokens)],
                });
            }
        }
        for MergedInterval { start, end, tokens } in merged {
            let mut cur = from_timestamp(start, tz)?;
            let stop = from_timestamp(end, tz)?;
            loop {
                let midnight = local_midnight(
                    cur.civil
                        .date()
                        .succ_opt()
                        .ok_or_else(|| error("OverflowError", "fecha fuera de rango"))?,
                    tz,
                )?;
                let piece_end = if midnight.civil < stop.civil {
                    midnight
                } else {
                    stop
                };
                let (a, b) = (cur.epoch, piece_end.epoch);
                let mut total = "0".to_string();
                for (at, tok) in &tokens {
                    if a <= *at && *at <= b && (*at < b || piece_end.civil == stop.civil) {
                        total = add_integers(&total, &tok.text());
                    }
                }
                let st = cur.civil.hour() as f64
                    + cur.civil.minute() as f64 / 60.
                    + cur.civil.second() as f64 / 3600.;
                let en = st + (b - a) / 3600.;
                out.push(json!({"d":cur.civil.date().to_string(),"acc":acc,"proj":proj,"st":number(st),"en":number(en),"tok":number(integer_float(&total)?/1e6)}));
                if piece_end.civil >= stop.civil {
                    break;
                }
                cur = piece_end;
            }
        }
    }
    out.sort_by(|a, b| {
        a["d"]
            .as_str()
            .cmp(&b["d"].as_str())
            .then_with(|| {
                a["st"]
                    .as_f64()
                    .partial_cmp(&b["st"].as_f64())
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| a["acc"].as_str().cmp(&b["acc"].as_str()))
            .then_with(|| a["proj"].as_str().cmp(&b["proj"].as_str()))
    });
    Ok(out)
}
#[derive(Default)]
struct Slot<'a> {
    week: Option<&'a Value>,
    model: Option<&'a Value>,
    h5: Option<&'a Value>,
}
fn limit_slots(limits: &[Value]) -> Result<BTreeMap<String, Slot<'_>>> {
    let mut slots: BTreeMap<String, Slot<'_>> = BTreeMap::new();
    for row in limits {
        let Some(p) = provider(row, false)? else {
            continue;
        };
        if row["percent"].is_null() {
            continue;
        }
        let acc = account_id(p, &row["account"])?;
        let slot = slots.entry(acc).or_default();
        if row["window"] == "5h" {
            slot.h5 = Some(row)
        } else if row["window"] == "7d" && truthy(&row["scope"]) {
            if slot.model.is_none()
                || numeric_cmp(&row["percent"], &slot.model.expect("model")["percent"])?
                    == Some(Ordering::Greater)
            {
                slot.model = Some(row)
            }
        } else if row["window"] == "7d" {
            slot.week = Some(row)
        }
    }
    Ok(slots)
}
fn cycles<'a>(snapshots: &'a [Value], acc: &str) -> Result<Vec<&'a Value>> {
    let mut out = vec![];
    for row in snapshots {
        let provider = required(row, "provider")?;
        let p = provider
            .as_str()
            .map_or_else(|| provider.to_string(), str::to_string);
        if account_id(&p, required(row, "account")?)? == acc
            && required(row, "window")? == "7d"
            && !truthy(&row["scope"])
        {
            if !matches!(
                required(row, "resets_at")?,
                Value::Number(_) | Value::Bool(_)
            ) {
                return Err(error("TypeError", "reset no numérico"));
            }
            out.push(row)
        }
    }
    out.sort_by(|a, b| {
        numeric_cmp(&b["resets_at"], &a["resets_at"])
            .expect("validated numbers")
            .unwrap_or(Ordering::Equal)
    });
    Ok(out)
}
pub fn waste(snapshots: &[Value], accounts: &[Value], now: f64) -> Result<Vec<Value>> {
    let mut out = vec![];
    for a in accounts {
        let acc = string(required(a, "id")?)?;
        let mut cyc = vec![];
        for row in cycles(snapshots, acc)? {
            if numeric_cmp(&row["resets_at"], &number(now))?.is_some_and(|o| !o.is_gt()) {
                let percent = required(row, "percent")?;
                let unused = if matches!(percent, Value::Bool(_))
                    || percent
                        .as_number()
                        .is_some_and(|n| crate::pomodoro::integer_token(n.as_str()))
                {
                    let raw = integer(percent)?;
                    integer_value(&add_integers(
                        "100",
                        &if let Some(stripped) = raw.strip_prefix('-') {
                            stripped.into()
                        } else {
                            format!("-{raw}")
                        },
                    ))
                } else {
                    round_int(100. - numeric(percent)?)?
                };
                cyc.push(unused);
                if cyc.len() == WASTE_CYCLES {
                    break;
                }
            }
        }
        out.push(json!({"id":acc,"cyc":cyc}));
    }
    Ok(out)
}
fn reset(row: Option<&Value>, tz: chrono_tz::Tz) -> Result<Value> {
    if let Some(row) = row {
        let zero = json!(0);
        let value = default(&row["resets_at"], &zero);
        if numeric_cmp(value, &json!(0))? == Some(Ordering::Greater) {
            return Ok(json!(fmt_reset_in(numeric(value)?, tz)?));
        }
    }
    Ok(Value::Null)
}
fn left(row: Option<&Value>, now: f64) -> Result<Value> {
    if let Some(row) = row {
        let zero = json!(0);
        let value = default(&row["resets_at"], &zero);
        if numeric_cmp(value, &json!(0))? == Some(Ordering::Greater) {
            return Ok(json!(fmt_left(&number(numeric(value)? - now))?));
        }
    }
    Ok(Value::Null)
}
fn used_at(snapshots: &[Value], acc: &str, window_end: f64, now: f64) -> Result<Value> {
    for row in cycles(snapshots, acc)?.into_iter().rev() {
        if numeric_cmp(&row["resets_at"], &number(window_end))? == Some(Ordering::Greater) {
            if numeric(&row["resets_at"])? - window_end > 608400. {
                return Ok(Value::Null);
            }
            return if numeric_cmp(&row["resets_at"], &number(now))?.is_some_and(|o| !o.is_gt()) {
                round_value(required(row, "percent")?)
            } else {
                Ok(Value::Null)
            };
        }
    }
    Ok(Value::Null)
}
pub struct AccountsInput<'a> {
    pub limits: &'a [Value],
    pub sessions: &'a [Value],
    pub today: Option<&'a str>,
    pub window_end: f64,
    pub snapshots: &'a [Value],
    pub now: f64,
    pub tz_name: &'a str,
    pub past: bool,
}
pub fn build_accounts(input: &AccountsInput<'_>) -> Result<Vec<Value>> {
    build_accounts_in(input, zone(input.tz_name)?)
}
fn build_accounts_in(input: &AccountsInput<'_>, tz: chrono_tz::Tz) -> Result<Vec<Value>> {
    let slots = limit_slots(input.limits)?;
    let mut ids = slots.keys().cloned().collect::<BTreeSet<_>>();
    for s in input.sessions {
        ids.insert(string(required(s, "acc")?)?.into());
    }
    let mut order = ids.into_iter().collect::<Vec<_>>();
    let rank = |a: &str| -> Result<(usize, bool, String)> {
        let parts = a.split(':').collect::<Vec<_>>();
        let p = PROVIDERS
            .iter()
            .position(|p| *p == parts[0])
            .ok_or_else(|| error("ValueError", "proveedor desconocido"))?;
        Ok((
            p,
            *parts
                .get(1)
                .ok_or_else(|| error("IndexError", "cuenta sin alias"))?
                != "main",
            a.into(),
        ))
    };
    let mut ranked = order
        .drain(..)
        .map(|a| Ok((rank(&a)?, a)))
        .collect::<Result<Vec<_>>>()?;
    ranked.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = vec![];
    let mut extra = 0;
    for (_, acc) in ranked {
        let (p, alias) = acc
            .split_once(':')
            .ok_or_else(|| error("ValueError", "cuenta sin alias"))?;
        let idx = PROVIDERS
            .iter()
            .position(|v| *v == p)
            .expect("validated provider");
        let color = match (p, alias) {
            ("claude", "main") => "#8B7CFF",
            ("claude", "relotto") => "#FF9A5C",
            ("codex", "main") => "#4CC2FF",
            ("grok", "main") => "#C5E35A",
            ("agy", "main") => "#5BD6A0",
            _ => {
                let color = EXTRA_COLORS[extra % 5];
                extra += 1;
                color
            }
        };
        let empty = Slot::default();
        let slot = slots.get(&acc).unwrap_or(&empty);
        let mine = input
            .sessions
            .iter()
            .filter(|s| s["acc"] == acc)
            .collect::<Vec<_>>();
        let today = mine
            .iter()
            .copied()
            .filter(|s| s["d"].as_str() == input.today)
            .collect::<Vec<_>>();
        let stats = |rows: &[&Value], daily: bool| -> Result<Value> {
            let (mut hours, mut tokens) = (0., 0.);
            for s in rows {
                hours += numeric(required(s, "en")?)? - numeric(required(s, "st")?)?;
                tokens += numeric(required(s, "tok")?)?;
            }
            // `sum(...)` sin elementos es el entero 0 del Python, no `0.0`.
            let hours = if rows.is_empty() {
                json!(0)
            } else {
                number(hours)
            };
            Ok(object([
                ("h", hours),
                (
                    "tok",
                    if daily {
                        round_int(tokens)?
                    } else {
                        number(round_digits(tokens / 1000., 1))
                    },
                ),
                ("ses", json!(rows.len())),
            ]))
        };
        let week = slot
            .week
            .map(|r| round_value(&r["percent"]))
            .transpose()?
            .unwrap_or(Value::Null);
        let model = if let Some(r) = slot.model {
            object([
                ("n", r["scope"].clone()),
                ("v", round_value(&r["percent"])?),
                ("reset", reset(Some(r), tz)?),
                ("left", left(Some(r), input.now)?),
            ])
        } else {
            Value::Null
        };
        // El orden de claves del `dict` del Python (`weekUsed` se añade al final).
        let week_used = if input.past {
            used_at(input.snapshots, &acc, input.window_end, input.now)?
        } else {
            week.clone()
        };
        let item = object([
            ("id", json!(acc)),
            ("provider", json!(p)),
            ("cli", json!(CLI[idx])),
            ("alias", json!(alias)),
            ("color", json!(color)),
            ("week", week),
            ("model", model),
            (
                "h5",
                slot.h5
                    .map(|r| round_value(&r["percent"]))
                    .transpose()?
                    .unwrap_or(Value::Null),
            ),
            ("reset", reset(slot.week, tz)?),
            ("left", left(slot.week, input.now)?),
            ("h5Reset", reset(slot.h5, tz)?),
            ("h5Left", left(slot.h5, input.now)?),
            ("hoy", stats(&today, true)?),
            ("sem", stats(&mine, false)?),
            ("weekUsed", week_used),
        ]);
        out.push(item);
    }
    Ok(out)
}
#[derive(Clone, Default)]
struct Measured {
    sessions: Vec<Value>,
    tokens: String,
    cost: f64,
    models: Vec<Value>,
}
fn scalar_equal(a: &Value, b: &Value) -> bool {
    if matches!(a, Value::Number(_) | Value::Bool(_))
        && matches!(b, Value::Number(_) | Value::Bool(_))
    {
        numeric_cmp(a, b).is_ok_and(|cmp| cmp == Some(Ordering::Equal))
    } else {
        a == b
    }
}
fn insert_scalar(values: &mut Vec<Value>, value: &Value) -> Result<()> {
    if matches!(value, Value::Array(_) | Value::Object(_)) {
        return Err(error("TypeError", "valor no hashable"));
    }
    if !values.iter().any(|v| scalar_equal(v, value)) {
        values.push(value.clone())
    }
    Ok(())
}
/// Lo medido de una cuenta en el orden de sus turnos y el primer error (con el
/// número de turno: el Python para en el primero de todos los que le afectan).
#[derive(Clone)]
struct Tally {
    measured: Measured,
    error: Option<(usize, Error)>,
}
impl Tally {
    fn new() -> Self {
        Self {
            measured: Measured {
                tokens: "0".into(),
                ..Measured::default()
            },
            error: None,
        }
    }
    fn add(&mut self, seq: usize, row: &Value) {
        if self.error.is_none()
            && let Err(e) = measure(&mut self.measured, row)
        {
            self.error = Some((seq, e))
        }
    }
}
/// El cuerpo del bucle de `sidebar_accounts` para un turno ya contado.
fn measure(group: &mut Measured, row: &Value) -> Result<()> {
    if truthy(&row["session"]) {
        insert_scalar(&mut group.sessions, &row["session"])?
    }
    let tok = integer(default(&row["tokens"], &json!(0)))?;
    if !tok.starts_with('-') {
        group.tokens = add_integers(&group.tokens, &tok);
    }
    group.cost += pymax(0., pyfloat(default(&row["cost"], &json!(0)))?);
    if truthy(&row["model"]) {
        insert_scalar(&mut group.models, &row["model"])?
    }
    Ok(())
}
/// Una cuenta de la ventana. Si ya está en la columna cuenta todos sus turnos
/// (`all`); si no, solo desde su primer turno de OpenCode, que la añade (`opencode`).
struct AccountTurns {
    all: Tally,
    opencode: Option<OpencodeTurns>,
}
struct OpencodeTurns {
    /// Turno que añade la cuenta: fija el orden de inserción en la columna.
    seq: usize,
    alias: String,
    tally: Tally,
}
/// Los turnos de `sidebar_accounts` reducidos por cuenta, sin saber aún qué
/// cuentas tiene la columna: se lleva lo de las dos posibilidades.
struct SidebarTurns {
    now: f64,
    seen: usize,
    /// Primer error que no depende de la columna (`float(finished)`, cuenta).
    error: Option<(usize, Error)>,
    accounts: BTreeMap<String, AccountTurns>,
}
impl SidebarTurns {
    fn new(now: f64) -> Self {
        Self {
            now,
            seen: 0,
            error: None,
            accounts: BTreeMap::new(),
        }
    }
    fn push(&mut self, row: &Value) {
        let seq = self.seen;
        self.seen += 1;
        // Tras ese error el Python ya no lee más turnos.
        if self.error.is_some() {
            return;
        }
        if let Err(e) = self.account_turn(seq, row) {
            self.error = Some((seq, e))
        }
    }
    fn account_turn(&mut self, seq: usize, row: &Value) -> Result<()> {
        let finished = pyfloat(default(&row["finished"], &json!(0)))?;
        if !(self.now - 604800. <= finished && finished <= self.now) {
            return Ok(());
        }
        let p = if row["agent"] == "opencode" {
            "opencode".into()
        } else {
            row["provider"].as_str().map_or_else(
                || {
                    if row["provider"].is_null() {
                        "None".into()
                    } else {
                        row["provider"].to_string()
                    }
                },
                str::to_string,
            )
        };
        let acc = account_id(&p, &row["account"])?;
        let opencode = if p == "opencode" {
            Some(account_of(&row["account"])?)
        } else {
            None
        };
        let entry = self.accounts.entry(acc).or_insert_with(|| AccountTurns {
            all: Tally::new(),
            opencode: None,
        });
        entry.all.add(seq, row);
        if let (None, Some(alias)) = (&entry.opencode, opencode) {
            entry.opencode = Some(OpencodeTurns {
                seq,
                alias,
                tally: Tally::new(),
            });
        }
        if let Some(o) = &mut entry.opencode {
            o.tally.add(seq, row)
        }
        Ok(())
    }
}
pub fn sidebar_accounts(
    accounts: &[Value],
    limits: &[Value],
    turns: &[Value],
    now: f64,
) -> Result<Vec<Value>> {
    let mut sidebar = SidebarTurns::new(now);
    for row in turns {
        sidebar.push(row);
    }
    sidebar_in(accounts, limits, &sidebar)
}
/// `sidebar_accounts` con los turnos ya reducidos (`WeekRows::push_turn`).
pub fn sidebar_accounts_from(
    accounts: &[Value],
    limits: &[Value],
    rows: &WeekRows,
) -> Result<Vec<Value>> {
    sidebar_in(accounts, limits, &rows.sidebar)
}
fn sidebar_in(accounts: &[Value], limits: &[Value], turns: &SidebarTurns) -> Result<Vec<Value>> {
    let mut out = accounts.to_vec();
    let mut by_id = BTreeMap::new();
    for (i, a) in out.iter().enumerate() {
        by_id.insert(string(required(a, "id")?)?.to_string(), i);
    }
    for row in limits {
        let provider = &row["provider"];
        let p = provider.as_str().map_or_else(
            || {
                if provider.is_null() {
                    "None".into()
                } else {
                    provider.to_string()
                }
            },
            str::to_string,
        );
        let id = account_id(&p, &row["account"])?;
        let plan = default(&row["plan_type"], &row["plan"]);
        if truthy(plan)
            && let Some(account) = by_id
                .get(&id)
                .and_then(|&i| out.get_mut(i))
                .and_then(Value::as_object_mut)
        {
            account.insert("plan".into(), plan.clone());
        }
    }
    // Las cuentas que cuentan: las de la columna con todos sus turnos y las de
    // OpenCode nuevas desde el turno que las añade, en ese orden de inserción.
    let mut fresh: Vec<(&String, &OpencodeTurns)> = vec![];
    let mut groups: BTreeMap<&String, &Tally> = BTreeMap::new();
    for (acc, entry) in &turns.accounts {
        if by_id.contains_key(acc) {
            groups.insert(acc, &entry.all);
        } else if let Some(o) = &entry.opencode {
            fresh.push((acc, o));
            groups.insert(acc, &o.tally);
        }
    }
    // El Python para en el primer turno que lanza de todos los que recorre.
    let first_error = groups
        .values()
        .filter_map(|t| t.error.as_ref())
        .chain(turns.error.as_ref())
        .min_by_key(|(seq, _)| *seq);
    if let Some((_, e)) = first_error {
        return Err(e.clone());
    }
    fresh.sort_by_key(|(_, o)| o.seq);
    for (acc, o) in fresh {
        by_id.insert(acc.clone(), out.len());
        out.push(json!({"id":acc,"provider":"opencode","cli":"OpenCode","alias":o.alias,"color":"#2fd3c0","week":null,"model":null,"h5":null}));
    }
    for (acc, tally) in groups {
        let mut group = tally.measured.clone();
        let mut sort_error = None;
        group.models.sort_by(|a, b| match (a, b) {
            (Value::String(a), Value::String(b)) => a.cmp(b),
            _ => match numeric_cmp(a, b) {
                Ok(o) => o.unwrap_or(Ordering::Equal),
                Err(e) => {
                    sort_error = Some(e);
                    Ordering::Equal
                }
            },
        });
        if let Some(e) = sort_error {
            return Err(e);
        }
        let mut measured = json!({"sessions":group.sessions.len()});
        measured["tokens"] = integer_value(&group.tokens);
        measured["costUsd"] = number(round_digits(group.cost, 6));
        measured["models"] = Value::Array(group.models);
        // Toda cuenta medida está en la columna (`by_id`) y es un objeto (tiene `id`).
        if let Some(account) = by_id
            .get(acc)
            .and_then(|&i| out.get_mut(i))
            .and_then(Value::as_object_mut)
        {
            account.insert("measured".into(), measured);
        }
    }
    Ok(out)
}
pub fn pomodoros(records: &[Value], days: &[String], tz: &str) -> Result<Vec<Value>> {
    pomodoros_in(records, days, zone(tz)?)
}
fn pomodoros_in(records: &[Value], days: &[String], tz: chrono_tz::Tz) -> Result<Vec<Value>> {
    let keep = days.iter().collect::<BTreeSet<_>>();
    let mut out = vec![];
    for r in records {
        if r["mode"] != "focus"
            || !matches!(
                r["status"].as_str(),
                Some("completed" | "cancelled" | "skipped")
            )
        {
            continue;
        }
        let start_ms = required(r, "startedAtMs")?;
        let start = from_timestamp(divide_number(start_ms, 1000)?, tz)?.civil;
        let day = start.date().to_string();
        if !keep.contains(&day) {
            continue;
        }
        let st = start.hour() as f64 + start.minute() as f64 / 60. + start.second() as f64 / 3600.;
        let zero = json!(0);
        let active = default(&r["activeMs"], &zero);
        let ended = if truthy(&r["endedAtMs"]) {
            r["endedAtMs"].clone()
        } else {
            numeric_add(start_ms, active)?
        };
        let target = default(&r["targetMs"], default(&r["plannedMs"], &zero));
        // `min(24, x)`: con `x >= 24` devuelve el primer argumento, el entero 24.
        let until = st + divide_number(&numeric_sub(&ended, start_ms)?, 3600000)?;
        let en = if until < 24. {
            number(until)
        } else {
            json!(24)
        };
        out.push(json!({"d":day,"st":number(st),"en":en,"plan":round_int(divide_number(target,60000)?)?,"act":round_int(divide_number(default(&r["activeMs"],&zero),60000)?)?,"pause":0,"status":if r["status"]=="completed"{"completed"}else{"cancelled"},"proj":pomodoro_project(&r["project"])?}));
    }
    out.sort_by(|a, b| {
        a["d"].as_str().cmp(&b["d"].as_str()).then_with(|| {
            a["st"]
                .as_f64()
                .partial_cmp(&b["st"].as_f64())
                .unwrap_or(Ordering::Equal)
        })
    });
    Ok(out)
}
/// Desde cuándo hacen falta turnos y tramos para la columna (`sidebar_week`):
/// la medianoche que abre la ventana menos `GAP_S` o los 7 días del consumo
/// medido, lo que sea antes. Un turno que terminó antes no cambia ninguna pieza
/// de sesión de la ventana: no puede unirse a uno que empieza ya dentro (los
/// separa más de `GAP_S`) y lo que aporte a un tramo que cruza la medianoche
/// queda antes de ella (la pieza de la ventana empieza en la medianoche), así
/// que `hoy`, `sem` y `measured` salen iguales que con la ventana entera.
pub fn sidebar_since(now: f64, offset: i64, tz_name: &str) -> Result<f64> {
    let tz = zone(tz_name)?;
    let dates = window_dates_in(now, offset, tz)?;
    let open = local_midnight(dates[WINDOW_DAYS as usize - 1], tz)?.epoch;
    Ok(pymin(open - GAP_S, now - 604800.))
}
/// Solo las cuentas de la columna: lo mismo que `build_week` + `sidebar_accounts`
/// dejan en `accounts`, sin sesiones a la vista, semana anterior, desperdicio ni
/// Pomodoro. `rows` puede venir recortado a `sidebar_since`; `snapshots` solo se
/// leen con `offset < 0` (`weekUsed` de una semana pasada).
pub fn sidebar_week(input: &WeekInput<'_>) -> Result<Vec<Value>> {
    let tz = zone(input.tz_name)?;
    let dates = window_dates_in(input.now, input.offset, tz)?;
    let iso = dates
        .iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let sess = sessions_in(&input.rows.intervals, tz)?
        .into_iter()
        .filter(|s| s["d"].as_str().is_some_and(|d| iso.contains(d)))
        .collect::<Vec<_>>();
    let window_end = local_midnight(
        dates[0]
            .succ_opt()
            .ok_or_else(|| error("OverflowError", "fecha fuera de rango"))?,
        tz,
    )?
    .epoch;
    let today = if input.offset == 0 {
        Some(dates[0].to_string())
    } else {
        None
    };
    let accounts = build_accounts_in(
        &AccountsInput {
            limits: input.limits,
            sessions: &sess,
            today: today.as_deref(),
            window_end,
            snapshots: input.snapshots,
            now: input.now,
            tz_name: input.tz_name,
            past: input.offset < 0,
        },
        tz,
    )?;
    sidebar_in(&accounts, input.limits, &input.rows.sidebar)
}
pub fn build_week(input: &WeekInput<'_>) -> Result<Value> {
    let tz = zone(input.tz_name)?;
    let dates = window_dates_in(input.now, input.offset, tz)?;
    let prev = window_dates_in(
        input.now,
        input
            .offset
            .checked_sub(1)
            .ok_or_else(|| error("OverflowError", "offset fuera de rango"))?,
        tz,
    )?;
    let days = day_rows(&dates);
    let iso = dates
        .iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let prev_iso = prev
        .iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let all = sessions_in(&input.rows.intervals, tz)?;
    let mut sess = vec![];
    let mut last: BTreeMap<String, f64> = BTreeMap::new();
    for s in all {
        let day = string(&s["d"])?;
        if iso.contains(day) {
            sess.push(s.clone())
        }
        if prev_iso.contains(day) {
            // `last.get(p, 0) + s["en"] - s["st"]`: suma y resta en ese orden.
            let acc = last.entry(string(&s["proj"])?.into()).or_default();
            *acc = (*acc + numeric(&s["en"])?) - numeric(&s["st"])?;
        }
    }
    let window_end = local_midnight(
        dates[0]
            .succ_opt()
            .ok_or_else(|| error("OverflowError", "fecha fuera de rango"))?,
        tz,
    )?
    .epoch;
    let today = if input.offset == 0 {
        Some(dates[0].to_string())
    } else {
        None
    };
    let accounts = build_accounts_in(
        &AccountsInput {
            limits: input.limits,
            sessions: &sess,
            today: today.as_deref(),
            window_end,
            snapshots: input.snapshots,
            now: input.now,
            tz_name: input.tz_name,
            past: input.offset < 0,
        },
        tz,
    )?;
    let local = from_timestamp(input.now, tz)?.civil;
    let last = last
        .into_iter()
        .map(|(k, v)| (k, number(v)))
        .collect::<serde_json::Map<_, _>>();
    let waste = waste(input.snapshots, &accounts, input.now)?;
    let pomodoros = pomodoros_in(input.records, &iso.into_iter().collect::<Vec<_>>(), tz)?;
    // El orden de claves del `dict` del Python: `accounts` va tras `days`.
    Ok(object([
        (
            "week",
            json!({"offset":input.offset,"label":week_label(&dates)? ,"start":dates[7].to_string(),"end":dates[0].to_string(),"today":today,"now":if input.offset==0{number(local.hour()as f64+local.minute()as f64/60.)}else{Value::Null},"measuredAt":format!("{:02}:{:02}",local.hour(),local.minute())}),
        ),
        ("days", Value::Array(days)),
        ("accounts", Value::Array(accounts)),
        ("sessions", Value::Array(sess)),
        ("lastWeek", Value::Object(last)),
        ("waste", Value::Array(waste)),
        ("pomodoros", Value::Array(pomodoros)),
    ]))
}
