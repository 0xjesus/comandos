//! Trabajo fuera del hilo de GTK. `Jobs` es un grupo de hilos fijos para tareas
//! cortas (HTTP ≤ 15 s, tmux ≤ 5 s); los bucles largos (`poll_state_loop`,
//! `ws_poll_loop`, `tmux_clipboard_loop`, esperas de cuenta) van en su propio hilo
//! con `spawn_loop`. El resultado vuelve al bucle de GLib por `async_channel`.
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

type Work = Box<dyn FnOnce() + Send + 'static>;

#[derive(Clone)]
pub struct Jobs {
    inner: Arc<Inner>,
}
struct Inner {
    tx: Mutex<Option<mpsc::Sender<Work>>>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    cancelled: Arc<AtomicBool>,
}

impl Jobs {
    pub fn new(workers: usize, name: &str) -> Jobs {
        let (tx, rx) = mpsc::channel::<Work>();
        let rx = Arc::new(Mutex::new(rx));
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        for i in 0..workers.max(2) {
            let rx = Arc::clone(&rx);
            let cancel = cancelled.clone();
            let handle = std::thread::Builder::new()
                .name(format!("comandos-{name}-{i}"))
                .spawn(move || {
                    loop {
                        let next = match rx.lock() {
                            Ok(guard) => guard.recv(),
                            Err(_) => break,
                        };
                        match next {
                            Ok(work) if !cancel.load(Ordering::Acquire) => work(),
                            Ok(_) => break,
                            Err(_) => break,
                        }
                    }
                });
            if let Ok(handle) = handle {
                handles.push(handle);
            }
        }
        Jobs {
            inner: Arc::new(Inner {
                tx: Mutex::new(Some(tx)),
                handles: Mutex::new(handles),
                cancelled,
            }),
        }
    }

    /// `work` en un hilo del grupo; `done(resultado)` después, en el hilo de GLib.
    pub fn spawn<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(T) + 'static,
    ) {
        let back = to_main(done);
        if let Ok(tx) = self.inner.tx.lock()
            && let Some(tx) = tx.as_ref()
        {
            let _ = tx.send(Box::new(move || {
                let _ = back.send_blocking(work());
            }));
        }
    }

    pub fn shutdown(&self, timeout: Duration) {
        self.inner.cancelled.store(true, Ordering::Release);
        if let Ok(mut tx) = self.inner.tx.lock() {
            tx.take();
        }
        let deadline = Instant::now() + timeout;
        if let Ok(mut handles) = self.inner.handles.lock() {
            while handles.iter().any(|h| !h.is_finished()) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            for handle in handles.drain(..) {
                if handle.is_finished() {
                    let _ = handle.join();
                }
            }
        }
    }
}

/// Canal hacia el hilo de GLib: lo que se envíe llega a `done` una vez.
pub fn to_main<T: Send + 'static>(done: impl FnOnce(T) + 'static) -> async_channel::Sender<T> {
    let (tx, rx) = async_channel::bounded::<T>(1);
    let context = glib::MainContext::ref_thread_default();
    let delivery_context = context.clone();
    context.spawn_local(async move {
        if let Ok(value) = rx.recv().await {
            // A completion can run a modal MainLoop. Leave the future executor
            // before calling it, so jobs arriving inside that loop can be polled.
            let mut completion = Some(glib::thread_guard::ThreadGuard::new(move || done(value)));
            let source = glib::idle_source_new(
                Some("comandos-job-completion"),
                glib::Priority::DEFAULT,
                move || {
                    if let Some(completion) = completion.take() {
                        completion.into_inner()();
                    }
                    glib::ControlFlow::Break
                },
            );
            source.attach(Some(&delivery_context));
        }
    });
    tx
}

/// Hilo propio con nombre para un bucle largo (equivale a `threading.Thread(daemon=True)`).
pub fn spawn_loop(name: &str, body: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(format!("comandos-{name}"))
        .spawn(body)
        .map(|_| ())
}
