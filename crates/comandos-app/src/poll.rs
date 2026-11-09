//! Pure dashboard polling. GTK bridges consume the receiver from T7.
use crate::dash_client::{DashClient, DashError};
use serde_json::Value;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum PollUpdate {
    State(Value),
    Prefs {
        value: Value,
        favorite_generation: u64,
    },
    Notices {
        revision: String,
        badge: u64,
    },
    Workspace(Value),
    Marks(Value),
    /// `GET /session-feed`; `Null` once the route stops answering (tiles as today).
    Feed(Value),
}

/// A GTK tick keeps only the latest complete update of each kind.
#[derive(Default)]
pub struct PollBatch {
    latest: std::collections::BTreeMap<u8, PollUpdate>,
}
impl PollBatch {
    pub fn push(&mut self, update: PollUpdate) {
        let key = match &update {
            PollUpdate::State(_) => 0,
            PollUpdate::Prefs { .. } => 1,
            PollUpdate::Workspace(_) => 2,
            PollUpdate::Marks(_) => 3,
            PollUpdate::Notices { .. } => 4,
            PollUpdate::Feed(_) => 5,
        };
        self.latest.insert(key, update);
    }
    pub fn drain(self) -> impl Iterator<Item = PollUpdate> {
        self.latest.into_values()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollIntervals {
    pub state_prefs: Duration,
    pub state_timeout: Duration,
    pub workspace: Duration,
    pub workspace_timeout: Duration,
    pub notices_wait_secs: u64,
    pub notices_timeout: Duration,
    pub notices_backoff: Duration,
    pub marks: Duration,
    pub marks_timeout: Duration,
    pub feed: Duration,
    pub feed_timeout: Duration,
}

impl Default for PollIntervals {
    fn default() -> Self {
        Self {
            state_prefs: Duration::from_secs(3),
            state_timeout: Duration::from_secs(3),
            workspace: Duration::from_secs(2),
            workspace_timeout: Duration::from_secs(3),
            notices_wait_secs: 25,
            notices_timeout: Duration::from_secs(35),
            notices_backoff: Duration::from_secs(2),
            marks: Duration::from_secs(5),
            marks_timeout: Duration::from_secs(3),
            feed: Duration::from_millis(2500),
            feed_timeout: Duration::from_secs(3),
        }
    }
}

pub struct Poller {
    stop: Arc<AtomicBool>,
    clients: Vec<DashClient>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Poller {
    pub fn start(
        client: DashClient,
        generation: Arc<AtomicU64>,
    ) -> (Poller, mpsc::Receiver<PollUpdate>) {
        Self::start_with_intervals(client, generation, PollIntervals::default())
    }

    pub fn start_with_intervals(
        client: DashClient,
        generation: Arc<AtomicU64>,
        intervals: PollIntervals,
    ) -> (Poller, mpsc::Receiver<PollUpdate>) {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        let state_client = client.isolated();
        let workspace_client = client.isolated();
        let marks_client = client.isolated();
        let notices_client = client.isolated();
        let feed_client = client.isolated();
        threads.push(spawn_state_prefs(
            state_client.clone(),
            generation,
            tx.clone(),
            stop.clone(),
            intervals,
        ));
        threads.push(spawn_simple(
            workspace_client.clone(),
            tx.clone(),
            stop.clone(),
            "/workspace",
            intervals.workspace,
            intervals.workspace_timeout,
            PollUpdate::Workspace,
        ));
        threads.push(spawn_simple(
            marks_client.clone(),
            tx.clone(),
            stop.clone(),
            "/work-marks",
            intervals.marks,
            intervals.marks_timeout,
            PollUpdate::Marks,
        ));
        threads.push(spawn_feed(
            feed_client.clone(),
            tx.clone(),
            stop.clone(),
            intervals.feed,
            intervals.feed_timeout,
        ));
        threads.push(spawn_notices(
            notices_client.clone(),
            tx,
            stop.clone(),
            intervals.notices_wait_secs,
            intervals.notices_timeout,
            intervals.notices_backoff,
        ));
        (
            Poller {
                stop,
                clients: vec![
                    client,
                    state_client,
                    workspace_client,
                    marks_client,
                    notices_client,
                    feed_client,
                ],
                threads: Mutex::new(threads),
            },
            rx,
        )
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        for client in &self.clients {
            client.cancel_pending();
        }
        if let Ok(mut threads) = self.threads.lock() {
            for thread in threads.iter() {
                thread.thread().unpark();
            }
            for thread in threads.drain(..) {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_state_prefs(
    client: DashClient,
    generation: Arc<AtomicU64>,
    tx: mpsc::Sender<PollUpdate>,
    stop: Arc<AtomicBool>,
    intervals: PollIntervals,
) -> JoinHandle<()> {
    let spawn = std::thread::Builder::new()
        .name("app-poll-state-prefs".into())
        .spawn(move || {
            let mut last_state: Option<Value> = None;
            let mut last_prefs: Option<Value> = None;
            while !stop.load(Ordering::Acquire) {
                if let Ok(value) = client.get("/state", intervals.state_timeout)
                    && last_state.as_ref() != Some(&value)
                {
                    last_state = Some(value.clone());
                    let _ = tx.send(PollUpdate::State(value));
                }
                let favorite_generation = generation.load(Ordering::Acquire);
                if let Ok(value) = client.get("/prefs", intervals.state_timeout) {
                    let snap = serde_json::json!({"live":live_pref_snapshot(&value),
                        "favorites":value.get("favorites"),"generation":favorite_generation});
                    if last_prefs.as_ref() != Some(&snap) {
                        last_prefs = Some(snap);
                        let _ = tx.send(PollUpdate::Prefs {
                            value,
                            favorite_generation,
                        });
                    }
                }
                sleep_cancel(&stop, intervals.state_prefs);
            }
        });
    match spawn {
        Ok(thread) => thread,
        Err(error) => panic!("poll state thread: {error}"),
    }
}

fn spawn_simple(
    client: DashClient,
    tx: mpsc::Sender<PollUpdate>,
    stop: Arc<AtomicBool>,
    path: &'static str,
    interval: Duration,
    timeout: Duration,
    wrap: fn(Value) -> PollUpdate,
) -> JoinHandle<()> {
    let spawn = std::thread::Builder::new()
        .name(format!("app-poll-{path}"))
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if let Ok(value) = client.get(path, timeout) {
                    let _ = tx.send(wrap(value));
                }
                sleep_cancel(&stop, interval);
            }
        });
    match spawn {
        Ok(thread) => thread,
        Err(error) => panic!("poll simple thread: {error}"),
    }
}

/// Ficha viva: only changes are sent. A server without the route (404) never
/// sends anything; after a few failures in a row a `Null` returns the tiles to
/// today's look instead of leaving a stale «te espera» painted.
fn spawn_feed(
    client: DashClient,
    tx: mpsc::Sender<PollUpdate>,
    stop: Arc<AtomicBool>,
    interval: Duration,
    timeout: Duration,
) -> JoinHandle<()> {
    let spawn = std::thread::Builder::new()
        .name("app-poll-feed".into())
        .spawn(move || {
            let mut last: Option<Value> = None;
            let mut failures = 0u32;
            while !stop.load(Ordering::Acquire) {
                match client.get("/session-feed", timeout) {
                    Ok(value) if value.get("sessions").is_some_and(Value::is_array) => {
                        failures = 0;
                        if last.as_ref() != Some(&value) {
                            last = Some(value.clone());
                            let _ = tx.send(PollUpdate::Feed(value));
                        }
                    }
                    Err(DashError::Cancelled) => break,
                    _ => {
                        failures = failures.saturating_add(1);
                        if failures >= crate::feed::FEED_FAILURES_TO_CLEAR && last.take().is_some()
                        {
                            let _ = tx.send(PollUpdate::Feed(Value::Null));
                        }
                    }
                }
                sleep_cancel(&stop, interval);
            }
        });
    match spawn {
        Ok(thread) => thread,
        Err(error) => panic!("poll feed thread: {error}"),
    }
}

fn spawn_notices(
    client: DashClient,
    tx: mpsc::Sender<PollUpdate>,
    stop: Arc<AtomicBool>,
    wait_secs: u64,
    timeout: Duration,
    backoff: Duration,
) -> JoinHandle<()> {
    let spawn = std::thread::Builder::new()
        .name("app-poll-notices".into())
        .spawn(move || {
            let mut rev = String::new();
            while !stop.load(Ordering::Acquire) {
                let path = format!("/notices/watch?rev={}&wait={wait_secs}", encode_query(&rev));
                match client.get(&path, timeout) {
                    Ok(value) => {
                        rev = value
                            .get("rev")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let badge = value.get("badge").and_then(Value::as_u64).unwrap_or(0);
                        let _ = tx.send(PollUpdate::Notices {
                            revision: rev.clone(),
                            badge,
                        });
                    }
                    Err(DashError::Cancelled) => break,
                    Err(_) => sleep_cancel(&stop, backoff),
                }
            }
        });
    match spawn {
        Ok(thread) => thread,
        Err(error) => panic!("poll notices thread: {error}"),
    }
}

pub fn live_pref_snapshot(value: &Value) -> Value {
    const KEYS: &[&str] = &[
        "font_family",
        "font_size",
        "cursor_shape",
        "cursor_blink",
        "terminal_padding",
        "terminal_opacity",
        "theme",
        "tabs_layout",
    ];
    let mut out = serde_json::Map::new();
    for key in KEYS {
        out.insert(
            (*key).to_string(),
            value.get(*key).cloned().unwrap_or(Value::Null),
        );
    }
    Value::Object(out)
}

fn sleep_cancel(stop: &AtomicBool, duration: Duration) {
    let started = std::time::Instant::now();
    while !stop.load(Ordering::Acquire) {
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        // stop() wakes every worker. Preserve the polling interval on a
        // spurious wake without waking idle threads one hundred times a second.
        std::thread::park_timeout(remaining);
    }
}

fn encode_query(raw: &str) -> String {
    let mut out = String::new();
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
