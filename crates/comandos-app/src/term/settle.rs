//! `_spawn_when_settled` (760): engancha cuando la asignación deja de cambiar
//! (`quiet_ms`) o, como tope, a los `cap_ms`. Así tmux redimensiona una sola vez
//! (al arrancar la ventana se acomoda en pasos 174→168→163 columnas).
pub const SETTLE_QUIET_MS: u64 = 250;
pub const SETTLE_CAP_MS: u64 = 1500;
/// Con la terminal ya enganchada, el pty se redimensiona cuando la asignación lleva
/// este tiempo quieta: arrastrar la ventana o el panel mandaba un SIGWINCH por paso y
/// Claude Code y Codex repintaban cada ancho intermedio encima del historial.
pub const RESIZE_QUIET_MS: u64 = 180;
/// `child-exited` → `GLib.timeout_add(800, spawn)` (884).
pub const RESPAWN_MS: u64 = 800;

#[derive(Debug, Clone)]
pub struct Settle {
    quiet: u64,
    cap_at: u64,
    last_alloc: Option<u64>,
    done: bool,
}

impl Settle {
    pub fn new(quiet_ms: u64, cap_ms: u64, start_ms: u64) -> Settle {
        Settle {
            quiet: quiet_ms,
            cap_at: start_ms.saturating_add(cap_ms),
            last_alloc: None,
            done: false,
        }
    }

    pub fn on_alloc(&mut self, now_ms: u64) {
        self.last_alloc = Some(now_ms);
    }

    pub fn due(&self, now_ms: u64) -> bool {
        !self.done
            && (now_ms >= self.cap_at
                || self
                    .last_alloc
                    .is_some_and(|t| now_ms >= t.saturating_add(self.quiet)))
    }

    pub fn fired(&mut self) -> bool {
        let first = !self.done;
        self.done = true;
        first
    }

    /// Cuándo volver a mirar (ms absolutos), o `None` si ya disparó.
    pub fn next_check_ms(&self, now_ms: u64) -> Option<u64> {
        if self.done {
            return None;
        }
        let quiet = self
            .last_alloc
            .map_or(self.cap_at, |t| t.saturating_add(self.quiet));
        Some(quiet.min(self.cap_at).max(now_ms.saturating_add(1)))
    }
}

/// Tamaño del primer attach (`make_term.spawn`, 861–866): con la asignación real si
/// existe; si GTK aún no dio tamaño (1 px, también con la pantalla bloqueada), el
/// que la sesión ya tiene en tmux; si no, 80×24 como VTE.
pub fn initial_size(
    (w, h): (i32, i32),
    cell_w: f64,
    cell_h: f64,
    (pad_x, pad_y): (f64, f64),
    tmux: Option<(u16, u16)>,
) -> (u16, u16) {
    if w <= 1 {
        return tmux.unwrap_or((80, 24));
    }
    let cells = |px: i32, pad: f64, cell: f64, min: f64| -> u16 {
        let n = ((f64::from(px) - pad) / cell.max(1.0))
            .floor()
            .max(min)
            .min(f64::from(u16::MAX));
        // n es entero y está en [min, u16::MAX]: el `as` es exacto.
        n as u16
    };
    (cells(w, pad_x, cell_w, 2.0), cells(h, pad_y, cell_h, 1.0))
}
