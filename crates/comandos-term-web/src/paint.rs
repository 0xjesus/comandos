//! Pintado por daños: qué filas repintar y cuándo. Sin DOM: [`Painter`] es
//! el canvas (o, en las pruebas de host, un registro de llamadas).
//!
//! - [`Scheduler`] junta los daños del motor y los cambios del cursor en un
//!   conjunto de filas sucias y decide si hace falta pedir un cuadro
//!   (`requestAnimationFrame`): como mucho uno en vuelo y ninguno si no hay
//!   nada que pintar, así una terminal quieta no gasta CPU.
//! - [`Blink`] es el parpadeo del cursor de xterm.js 5.5
//!   (`CursorBlinkStateManager`): 600 ms visible, 600 oculto, y cada cambio
//!   de la rejilla lo reinicia visible.
//!
//! Los búferes (filas sucias, fila renderizada, lista de filas vacías) se
//! reutilizan entre cuadros: pintar no deja basura.
use crate::metrics::CellMetrics;
use comandos_term::{
    engine::{Engine, Palette},
    render::{
        CursorShape, CursorView, RenderOpts, RowRender, Run, cursor, preferred_cursor_shape,
        render_row_into,
    },
};

/// Intervalo de parpadeo del cursor de xterm.js (`BLINK_INTERVAL`).
pub const BLINK_INTERVAL_MS: f64 = 600.0;

/// Celdas seleccionadas de una fila: `cols` es `inicio..fin` (fin
/// excluido) y `color` el de la selección opaca (`selectionBackgroundOpaque`).
/// addon-canvas rasteriza los glifos de esas celdas contra ese color
/// (`CellColorResolver`), y la capa de selección los tiñe después.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selected {
    pub cols: (u16, u16),
    pub color: [u8; 3],
}

impl Selected {
    pub fn contains(&self, col: u16) -> bool {
        self.cols.0 <= col && col < self.cols.1
    }
}

/// Quien pinta las filas en pantalla.
pub trait Painter {
    /// Deja las filas de la vista con solo el fondo de la terminal.
    fn clear_rows(&mut self, rows: &[usize]);
    /// Deja en el atlas los glifos que la fila va a usar, antes de pintar
    /// ninguna: escribir un glifo nuevo en el atlas y copiarlo enseguida
    /// sincroniza la GPU en cada letra (≈3 ms); juntos, ≈0,1 ms.
    fn prepare_row(&mut self, _row: &RowRender, _selected: Option<Selected>, _m: &CellMetrics) {}
    /// Repinta una fila entera (fondo, fondos de celda, glifos); `selected`
    /// son sus celdas seleccionadas.
    fn paint_row(&mut self, row: &RowRender, selected: Option<Selected>, m: &CellMetrics);
    /// Pinta el cursor encima de su fila ya pintada; `under` es la tira
    /// de la celda del cursor (su glifo va en el color de acento).
    fn paint_cursor(&mut self, c: &CursorView, under: Option<&Run>, m: &CellMetrics);
    /// Nuevo tamaño de la rejilla o de la celda.
    fn resize(&mut self, cols: u16, rows: u16, m: &CellMetrics);
}

/// El cursor parpadea: hay foco (sin foco xterm.js lo pausa), la aplicación
/// lo muestra dentro de la vista y su estilo parpadea (`cursorBlink` de la
/// página o DECSCUSR). Sin cursor visible no hay temporizador.
pub fn blink_enabled(engine: &Engine, focused: bool) -> bool {
    let view = cursor(engine);
    focused
        && view.visible
        && view.shape != CursorShape::Hidden
        && engine.term().cursor_style().blinking
}

/// Estado del cursor que no viene del motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorInput {
    /// Sin foco no parpadea; conserva la barra o el subrayado preferidos.
    pub focused: bool,
    /// Fase visible del parpadeo (siempre `true` si no parpadea).
    pub blink_on: bool,
    /// Preferencia explícita del usuario, por encima de DECSCUSR.
    pub preferred_shape: Option<CursorShape>,
}

/// El cursor tal como se dibuja: `None` si no se ve.
pub fn cursor_to_paint(view: CursorView, input: CursorInput) -> Option<CursorView> {
    if !view.visible || view.shape == CursorShape::Hidden {
        return None;
    }
    let view = CursorView {
        shape: preferred_cursor_shape(view.shape, input.preferred_shape, input.focused),
        ..view
    };
    (!input.focused || input.blink_on).then_some(view)
}

/// Parpadeo del cursor. El tiempo es el de `performance.now()` (ms).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blink {
    /// Parpadea (opción de la página y estilo de la aplicación) y hay foco.
    pub enabled: bool,
    /// Último reinicio: desde aquí, 600 ms visible y 600 oculto.
    epoch_ms: f64,
}

impl Blink {
    pub fn new(enabled: bool, now_ms: f64) -> Blink {
        Blink {
            enabled,
            epoch_ms: now_ms,
        }
    }

    /// Activa o desactiva el parpadeo. Al pasar a activo el ciclo vuelve a
    /// empezar en la fase visible; devuelve si se reinició.
    pub fn set_enabled(&mut self, enabled: bool, now_ms: f64) -> bool {
        let restarted = enabled && !self.enabled;
        self.enabled = enabled;
        if restarted {
            self.restart(now_ms);
        }
        restarted
    }

    /// Vuelve a la fase visible (`restartBlinkAnimation`).
    pub fn restart(&mut self, now_ms: f64) {
        self.epoch_ms = now_ms;
    }

    /// Fase visible en `now_ms`.
    pub fn visible(&self, now_ms: f64) -> bool {
        if !self.enabled {
            return true;
        }
        let elapsed = (now_ms - self.epoch_ms).max(0.0);
        let phase = (elapsed / BLINK_INTERVAL_MS).floor();
        phase.is_finite() && phase % 2.0 == 0.0
    }

    /// Instante del próximo cambio de fase (si parpadea).
    pub fn next_toggle(&self, now_ms: f64) -> Option<f64> {
        if !self.enabled {
            return None;
        }
        let elapsed = (now_ms - self.epoch_ms).max(0.0);
        let phase = (elapsed / BLINK_INTERVAL_MS).floor();
        phase
            .is_finite()
            .then_some(self.epoch_ms + (phase + 1.0) * BLINK_INTERVAL_MS)
    }
}

/// Filas sucias de la vista, sin asignar memoria por cuadro.
#[derive(Debug, Clone, Default)]
struct DirtyRows {
    rows: Vec<bool>,
    count: usize,
}

impl DirtyRows {
    fn resize(&mut self, rows: usize) {
        self.rows.clear();
        self.rows.resize(rows, false);
        self.count = 0;
    }

    fn mark(&mut self, row: usize) {
        if let Some(slot) = self.rows.get_mut(row)
            && !*slot
        {
            *slot = true;
            self.count += 1;
        }
    }

    fn mark_all(&mut self) {
        for slot in &mut self.rows {
            *slot = true;
        }
        self.count = self.rows.len();
    }

    fn is_dirty(&self, row: usize) -> bool {
        self.rows.get(row).copied().unwrap_or(false)
    }
}

/// Lo que hizo un cuadro (para pruebas y diagnóstico).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Filas repintadas con contenido.
    pub painted: usize,
    /// Filas que solo llevaban fondo.
    pub cleared: usize,
    /// Se dibujó el cursor.
    pub cursor: bool,
    /// Primera y última fila de la vista con daño de contenido (escritura,
    /// desplazamiento, tema), sin contar el cursor solo: el
    /// `onRenderedViewportChange` con el que xterm.js decide si el enlace
    /// bajo el ratón sigue valiendo.
    pub content: Option<(usize, usize)>,
}

/// Junta daños y decide cuándo pintar.
#[derive(Debug, Clone, Default)]
pub struct Scheduler {
    dirty: DirtyRows,
    in_flight: bool,
    /// Cursor dibujado en el último cuadro.
    painted_cursor: Option<CursorView>,
    /// Cursor que se pintará en el próximo cuadro.
    wanted_cursor: Option<CursorView>,
    row: RowRender,
    empty: Vec<usize>,
    /// Índices de daño del motor (reutilizado).
    damage: Vec<usize>,
    /// Rango de filas con daño de contenido desde el último cuadro.
    content: Option<(usize, usize)>,
    /// Celdas seleccionadas de cada fila de la vista.
    selected: Vec<Option<(u16, u16)>>,
    /// Color de la selección opaca.
    selection_color: [u8; 3],
}

impl Scheduler {
    /// Planificador para `rows` filas, todas sucias (primer cuadro).
    pub fn new(rows: u16) -> Scheduler {
        let mut s = Scheduler::default();
        s.resize(rows);
        s
    }

    /// Nuevo alto de la vista: todo sucio y sin cursor pintado.
    pub fn resize(&mut self, rows: u16) {
        self.selected.clear();
        self.selected.resize(usize::from(rows), None);
        self.dirty.resize(usize::from(rows));
        self.dirty.mark_all();
        self.note_all();
        self.painted_cursor = None;
    }

    /// Repintar todo (tema, fuente, tamaño de celda).
    pub fn invalidate_all(&mut self) {
        self.dirty.mark_all();
        self.note_all();
    }

    /// Nuevas celdas seleccionadas por fila de la vista (`None` = ninguna;
    /// lo que falte al final, tampoco). Solo se marcan para repintar las
    /// filas cuyo tramo cambió; si cambia el color, todas las que tienen
    /// selección. No cuenta como daño de contenido (no suelta el enlace).
    pub fn set_selection(&mut self, rows: &[Option<(u16, u16)>], color: [u8; 3]) {
        let recolor = color != self.selection_color;
        self.selection_color = color;
        for line in 0..self.selected.len() {
            let new = rows.get(line).copied().flatten();
            let Some(slot) = self.selected.get_mut(line) else {
                continue;
            };
            if *slot != new || (recolor && new.is_some()) {
                *slot = new;
                self.dirty.mark(line);
            }
        }
    }

    /// Suma `lo..=hi` al rango de contenido del próximo cuadro.
    fn note(&mut self, lo: usize, hi: usize) {
        self.content = Some(match self.content {
            Some((a, b)) => (a.min(lo), b.max(hi)),
            None => (lo, hi),
        });
    }

    fn note_all(&mut self) {
        if let Some(last) = self.dirty.rows.len().checked_sub(1) {
            self.note(0, last);
        }
    }

    /// Hay algo que pintar.
    pub fn has_work(&self) -> bool {
        self.dirty.count > 0
    }

    /// Recoge los daños del motor y el estado del cursor. Llamar tras cada
    /// cambio del motor (bytes, desplazamiento, vencimiento de 2026).
    pub fn absorb(&mut self, engine: &mut Engine, input: CursorInput) {
        if engine.take_damage_into(&mut self.damage) {
            self.dirty.mark_all();
            self.note_all();
        } else {
            // alacritty da `línea de pantalla + display_offset`, que es justo
            // la fila de la vista donde se ve esa línea; lo que queda debajo
            // de la vista ya viene filtrado y `mark` descarta cualquier
            // índice fuera de la pantalla.
            for i in 0..self.damage.len() {
                if let Some(&line) = self.damage.get(i) {
                    self.dirty.mark(line);
                    if line < self.dirty.rows.len() {
                        self.note(line, line);
                    }
                }
            }
        }
        self.absorb_cursor(engine, input);
    }

    /// Solo el cursor (foco, parpadeo): no toca los daños del motor, que
    /// siempre incluyen la fila del cursor. Si el cursor dibujado no
    /// cambia, no hay nada que pintar.
    pub fn absorb_cursor(&mut self, engine: &Engine, input: CursorInput) {
        let wanted = cursor_to_paint(cursor(engine), input);
        if wanted != self.painted_cursor {
            if let Some(old) = self.painted_cursor {
                self.dirty.mark(old.line);
            }
            if let Some(new) = wanted {
                self.dirty.mark(new.line);
            }
        }
        self.wanted_cursor = wanted;
    }

    /// `true` si hay que pedir un cuadro ahora; queda uno en vuelo hasta
    /// [`Scheduler::frame`]. Sin trabajo no se pide nada.
    pub fn wants_frame(&mut self) -> bool {
        if self.in_flight || !self.has_work() {
            return false;
        }
        self.in_flight = true;
        true
    }

    /// El cuadro pedido no llegará (por ejemplo, se canceló).
    pub fn frame_cancelled(&mut self) {
        self.in_flight = false;
    }

    /// Pinta las filas sucias y el cursor. Llamar desde el cuadro.
    pub fn frame<P: Painter>(
        &mut self,
        engine: &mut Engine,
        palette: &Palette,
        opts: &RenderOpts,
        input: CursorInput,
        m: &CellMetrics,
        painter: &mut P,
    ) -> FrameStats {
        self.in_flight = false;
        // Los daños ya llegaron con cada `absorb`; aquí solo el cursor (la
        // fase del parpadeo puede haber cambiado mientras tanto).
        self.absorb_cursor(engine, input);
        let mut stats = FrameStats {
            content: self.content.take(),
            ..FrameStats::default()
        };
        if !self.has_work() {
            return stats;
        }
        self.empty.clear();
        let wanted = self.wanted_cursor.filter(|c| self.dirty.is_dirty(c.line));
        for line in 0..self.dirty.rows.len() {
            if self.dirty.is_dirty(line) {
                render_row_into(engine, line, palette, opts, &mut self.row);
                let selected = self.selected.get(line).copied().flatten().map(|cols| Selected {
                    cols,
                    color: self.selection_color,
                });
                painter.prepare_row(&self.row, selected, m);
            }
        }
        for line in 0..self.dirty.rows.len() {
            if !self.dirty.is_dirty(line) {
                continue;
            }
            render_row_into(engine, line, palette, opts, &mut self.row);
            let selected = self
                .selected
                .get(line)
                .copied()
                .flatten()
                .map(|cols| Selected {
                    cols,
                    color: self.selection_color,
                });
            let blank = self.row.bg_runs.is_empty() && self.row.runs.is_empty();
            let Some(c) = wanted.filter(|c| c.line == line) else {
                if blank {
                    // Las filas vacías se limpian juntas al final.
                    self.empty.push(line);
                } else {
                    painter.paint_row(&self.row, selected, m);
                    stats.painted += 1;
                }
                continue;
            };
            // La fila del cursor se pinta ya y el cursor encima: las filas
            // son franjas independientes (cada una recorta a la suya).
            if blank {
                painter.clear_rows(&[line]);
                stats.cleared += 1;
            } else {
                painter.paint_row(&self.row, selected, m);
                stats.painted += 1;
            }
            let under = self
                .row
                .runs
                .iter()
                .find(|run| run.col <= c.col && c.col < run.col.saturating_add(run.cells));
            painter.paint_cursor(&c, under, m);
            stats.cursor = true;
        }
        if !self.empty.is_empty() {
            painter.clear_rows(&self.empty);
            stats.cleared += self.empty.len();
        }
        self.painted_cursor = self.wanted_cursor;
        self.dirty.mark_none();
        stats
    }
}

impl DirtyRows {
    fn mark_none(&mut self) {
        for slot in &mut self.rows {
            *slot = false;
        }
        self.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::from_measure;
    use comandos_term::engine::GridSize;

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Clear(Vec<usize>),
        Row(usize, String),
        Sel(usize, (u16, u16), [u8; 3]),
        Cursor(usize, u16, CursorShape, Option<String>),
        Resize(u16, u16),
    }

    #[derive(Default)]
    struct Recorder(Vec<Call>);

    impl Painter for Recorder {
        fn clear_rows(&mut self, rows: &[usize]) {
            self.0.push(Call::Clear(rows.to_vec()));
        }
        fn paint_row(&mut self, row: &RowRender, selected: Option<Selected>, _m: &CellMetrics) {
            let text: String = row.runs.iter().map(|r| r.text.as_str()).collect();
            self.0.push(Call::Row(row.line, text));
            if let Some(sel) = selected {
                self.0.push(Call::Sel(row.line, sel.cols, sel.color));
            }
        }
        fn paint_cursor(&mut self, c: &CursorView, under: Option<&Run>, _m: &CellMetrics) {
            self.0.push(Call::Cursor(
                c.line,
                c.col,
                c.shape,
                under.map(|r| r.text.clone()),
            ));
        }
        fn resize(&mut self, cols: u16, rows: u16, _m: &CellMetrics) {
            self.0.push(Call::Resize(cols, rows));
        }
    }

    const FOCUSED: CursorInput = CursorInput {
        focused: true,
        blink_on: true,
        preferred_shape: None,
    };

    fn setup(rows: u16) -> (Engine, Palette, Scheduler, CellMetrics) {
        let palette = Palette::xterm_default([255; 3], [0; 3], [200; 3], [0; 3], [50; 3]);
        let mut engine = Engine::new(GridSize { cols: 10, rows }, 100, palette.clone());
        let mut s = Scheduler::new(rows);
        // Como `WebTerm`: el daño inicial (todo) se recoge al montar.
        s.absorb(&mut engine, FOCUSED);
        (engine, palette, s, from_measure(7.0, 14.0, 1.0, 1.2, 0.0))
    }

    fn run(
        e: &mut Engine,
        p: &Palette,
        s: &mut Scheduler,
        m: &CellMetrics,
        input: CursorInput,
    ) -> (FrameStats, Vec<Call>) {
        let mut rec = Recorder::default();
        let stats = s.frame(e, p, &RenderOpts::default(), input, m, &mut rec);
        (stats, rec.0)
    }

    #[test]
    fn first_frame_paints_everything_then_idle_paints_nothing() {
        let (mut e, p, mut s, m) = setup(3);
        assert!(s.wants_frame());
        let (stats, calls) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(stats.cleared, 3);
        assert_eq!(
            calls,
            vec![
                Call::Clear(vec![0]),
                Call::Cursor(0, 0, CursorShape::Block, None),
                Call::Clear(vec![1, 2]),
            ]
        );
        // En reposo nadie llama a `absorb`: no hay trabajo ni cuadro.
        assert!(!s.has_work());
        assert!(!s.wants_frame());
        // alacritty siempre daña la fila del cursor, pero sin cambio del
        // cursor eso no pide cuadro ni repinta nada.
        s.absorb_cursor(&e, FOCUSED);
        assert!(!s.wants_frame());
        let (stats, calls) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(stats, FrameStats::default());
        assert!(calls.is_empty());
    }

    #[test]
    fn at_most_one_frame_in_flight() {
        let (mut e, p, mut s, m) = setup(3);
        assert!(s.wants_frame());
        assert!(!s.wants_frame());
        run(&mut e, &p, &mut s, &m, FOCUSED);
        e.advance(b"x", 0.0);
        s.absorb(&mut e, FOCUSED);
        assert!(s.wants_frame());
        assert!(!s.wants_frame());
        s.frame_cancelled();
        assert!(s.wants_frame());
    }

    #[test]
    fn writing_repaints_only_the_touched_rows() {
        let (mut e, p, mut s, m) = setup(4);
        run(&mut e, &p, &mut s, &m, FOCUSED);
        e.advance(b"\x1b[3;1Hhola", 0.0);
        s.absorb(&mut e, FOCUSED);
        let (stats, calls) = run(&mut e, &p, &mut s, &m, FOCUSED);
        // Fila 0 (el cursor se fue) y fila 2 (texto y cursor nuevo).
        assert_eq!(stats.painted, 1);
        assert_eq!(
            calls,
            vec![
                Call::Row(2, "hola".into()),
                Call::Cursor(2, 4, CursorShape::Block, None),
                Call::Clear(vec![0]),
            ]
        );
        assert!(!calls.iter().any(|c| matches!(c, Call::Row(1 | 3, _))));
    }

    #[test]
    fn frames_report_the_rows_damaged_by_content_not_by_the_cursor() {
        let (mut e, p, mut s, m) = setup(5);
        let (stats, _) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(stats.content, Some((0, 4)), "primer cuadro: todo");
        e.advance(b"\x1b[2;1Ha\x1b[4;1Hb", 0.0);
        s.absorb(&mut e, FOCUSED);
        e.advance(b"\x1b[3;1Hc", 0.0);
        s.absorb(&mut e, FOCUSED);
        let (stats, _) = run(&mut e, &p, &mut s, &m, FOCUSED);
        let (lo, hi) = stats.content.unwrap_or_default();
        assert!(lo <= 1 && hi >= 3, "{:?}", stats.content);
        // El parpadeo y el foco solo repintan el cursor.
        let hidden = CursorInput {
            focused: true,
            blink_on: false,
            preferred_shape: None,
        };
        s.absorb_cursor(&e, hidden);
        let (stats, _) = run(&mut e, &p, &mut s, &m, hidden);
        assert!(stats.cursor || stats.cleared + stats.painted > 0);
        assert_eq!(stats.content, None);
        e.scroll_display(0);
        s.invalidate_all();
        let (stats, _) = run(&mut e, &p, &mut s, &m, hidden);
        assert_eq!(stats.content, Some((0, 4)), "tema o fuente");
    }

    /// Cambiar la selección repinta solo las filas cuyo tramo de celdas
    /// cambió, y cada fila se pinta con su tramo; el foco no repinta nada
    /// (sin `selectionInactiveBackground` el color es el mismo).
    #[test]
    fn selection_changes_repaint_only_the_rows_whose_cells_changed() {
        let (mut e, p, mut s, m) = setup(5);
        e.advance(b"uno\r\ndos\r\ntres\r\ncuatro\r\ncinco", 0.0);
        s.absorb(&mut e, FOCUSED);
        run(&mut e, &p, &mut s, &m, FOCUSED);
        let sel = [0x2E, 0x38, 0x52];
        let rows = |calls: &[Call]| -> Vec<usize> {
            calls
                .iter()
                .filter_map(|c| match c {
                    Call::Row(line, _) => Some(*line),
                    Call::Clear(lines) => lines.first().copied(),
                    _ => None,
                })
                .collect()
        };
        // Filas 1 a 3.
        s.set_selection(&[None, Some((2, 10)), Some((0, 10)), Some((0, 3))], sel);
        assert!(s.wants_frame());
        let (stats, calls) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(rows(&calls), vec![1, 2, 3]);
        assert!(calls.contains(&Call::Sel(1, (2, 10), sel)));
        assert!(calls.contains(&Call::Sel(3, (0, 3), sel)));
        assert_eq!(stats.content, None, "no suelta el enlace");
        // Se alarga por abajo: solo cambian la 3 y la 4.
        s.set_selection(
            &[
                None,
                Some((2, 10)),
                Some((0, 10)),
                Some((0, 10)),
                Some((0, 1)),
            ],
            sel,
        );
        let (_, calls) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(rows(&calls), vec![3, 4]);
        // Lo mismo otra vez: nada.
        s.set_selection(
            &[
                None,
                Some((2, 10)),
                Some((0, 10)),
                Some((0, 10)),
                Some((0, 1)),
            ],
            sel,
        );
        assert!(!s.wants_frame());
        // El foco no toca la selección (solo el cursor de la fila 4).
        let blurred = CursorInput {
            focused: false,
            blink_on: true,
            preferred_shape: None,
        };
        s.absorb_cursor(&e, blurred);
        let (_, calls) = run(&mut e, &p, &mut s, &m, blurred);
        assert_eq!(rows(&calls), vec![4]);
        assert!(
            calls.contains(&Call::Sel(4, (0, 1), sel)),
            "sigue seleccionada"
        );
        // Otro color (tema nuevo): las filas con selección.
        s.set_selection(
            &[
                None,
                Some((2, 10)),
                Some((0, 10)),
                Some((0, 10)),
                Some((0, 1)),
            ],
            [1, 2, 3],
        );
        let (_, calls) = run(&mut e, &p, &mut s, &m, blurred);
        assert_eq!(rows(&calls), vec![1, 2, 3, 4]);
        // Sin selección: las cuatro, sin tramo.
        s.set_selection(&[], [1, 2, 3]);
        let (_, calls) = run(&mut e, &p, &mut s, &m, blurred);
        assert_eq!(rows(&calls), vec![1, 2, 3, 4]);
        assert!(!calls.iter().any(|c| matches!(c, Call::Sel(..))));
    }

    #[test]
    fn cursor_over_text_carries_its_run() {
        let (mut e, p, mut s, m) = setup(2);
        run(&mut e, &p, &mut s, &m, FOCUSED);
        e.advance(b"abc\x1b[2D", 0.0);
        s.absorb(&mut e, FOCUSED);
        let (_, calls) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(
            calls,
            vec![
                Call::Row(0, "abc".into()),
                Call::Cursor(0, 1, CursorShape::Block, Some("abc".into())),
            ]
        );
    }

    #[test]
    fn blink_off_hides_and_blur_draws_the_outline() {
        let (mut e, p, mut s, m) = setup(2);
        run(&mut e, &p, &mut s, &m, FOCUSED);
        let hidden = CursorInput {
            focused: true,
            blink_on: false,
            preferred_shape: None,
        };
        s.absorb_cursor(&e, hidden);
        assert!(s.wants_frame());
        let (_, calls) = run(&mut e, &p, &mut s, &m, hidden);
        assert_eq!(calls, vec![Call::Clear(vec![0])]);
        let blurred = CursorInput {
            focused: false,
            blink_on: false,
            preferred_shape: None,
        };
        s.absorb_cursor(&e, blurred);
        let (_, calls) = run(&mut e, &p, &mut s, &m, blurred);
        assert_eq!(
            calls.last(),
            Some(&Call::Cursor(0, 0, CursorShape::HollowBlock, None))
        );
    }

    #[test]
    fn preferred_bar_is_thin_after_application_changes_and_blur() {
        let (mut e, p, mut s, m) = setup(2);
        let preferred = CursorInput {
            preferred_shape: Some(CursorShape::Beam),
            ..FOCUSED
        };
        e.advance(b"\x1b[4 q", 0.0);
        s.absorb(&mut e, preferred);
        let (_, calls) = run(&mut e, &p, &mut s, &m, preferred);
        assert!(calls.contains(&Call::Cursor(0, 0, CursorShape::Beam, None)));
        let blurred = CursorInput {
            focused: false,
            ..preferred
        };
        s.absorb_cursor(&e, blurred);
        assert_eq!(
            cursor_to_paint(cursor(&e), blurred).unwrap().shape,
            CursorShape::Beam
        );
        e.advance(b"\x1b[?25l", 0.0);
        assert_eq!(cursor_to_paint(cursor(&e), blurred), None);
    }

    #[test]
    fn hidden_cursor_mode_draws_no_cursor() {
        let (mut e, p, mut s, m) = setup(2);
        e.advance(b"\x1b[?25l", 0.0);
        let (stats, _) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert!(!stats.cursor);
    }

    #[test]
    fn scrolling_the_view_repaints_all_rows() {
        let (mut e, p, mut s, m) = setup(3);
        e.advance(b"1\r\n2\r\n3\r\n4\r\n5", 0.0);
        run(&mut e, &p, &mut s, &m, FOCUSED);
        e.scroll_display(2);
        s.absorb(&mut e, FOCUSED);
        let (stats, calls) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(stats.painted, 3);
        assert!(calls.contains(&Call::Row(0, "1".into())));
        // El cursor quedó debajo de la vista: no se dibuja.
        assert!(!stats.cursor);
    }

    #[test]
    fn partial_damage_while_scrolled_lands_on_the_view_row() {
        let (mut e, p, mut s, m) = setup(3);
        e.advance(b"1\r\n2\r\n3\r\n4\r\n5", 0.0);
        e.scroll_display(1);
        s.absorb(&mut e, FOCUSED);
        run(&mut e, &p, &mut s, &m, FOCUSED);
        // La línea 0 de la pantalla se ve en la fila 1 de la vista.
        e.advance(b"\x1b[1;1HZ", 0.0);
        s.absorb(&mut e, FOCUSED);
        let (_, calls) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(
            calls,
            vec![
                Call::Row(1, "Z".into()),
                Call::Cursor(1, 1, CursorShape::Block, None),
            ]
        );
    }

    #[test]
    fn resize_marks_everything_dirty() {
        let (mut e, p, mut s, m) = setup(2);
        run(&mut e, &p, &mut s, &m, FOCUSED);
        s.resize(5);
        assert!(s.wants_frame());
        let (stats, _) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(stats.cleared + stats.painted, 5);
        let mut rec = Recorder::default();
        rec.resize(80, 24, &m);
        assert_eq!(rec.0, vec![Call::Resize(80, 24)]);
    }

    #[test]
    fn hidden_cursor_does_not_blink_and_idles_while_focused() {
        let (mut e, p, mut s, m) = setup(2);
        let blinking =
            Engine::with_cursor_blink(GridSize { cols: 10, rows: 2 }, 100, p.clone(), true);
        assert!(blink_enabled(&blinking, true));
        assert!(!blink_enabled(&blinking, false));
        // Sin parpadeo de la página (DECSCUSR por omisión) tampoco.
        assert!(!blink_enabled(&e, true));
        let mut e2 = blinking;
        e2.advance(b"\x1b[?25l", 0.0);
        assert!(!blink_enabled(&e2, true));
        assert_eq!(
            Blink::new(blink_enabled(&e2, true), 0.0).next_toggle(10.0),
            None
        );
        // Con el cursor oculto y foco: tras el primer cuadro, nada que hacer.
        e.advance(b"\x1b[?25l", 0.0);
        s.absorb(&mut e, FOCUSED);
        run(&mut e, &p, &mut s, &m, FOCUSED);
        for phase in [false, true, false] {
            let input = CursorInput {
                focused: true,
                blink_on: phase,
                preferred_shape: None,
            };
            s.absorb_cursor(&e, input);
            assert!(!s.wants_frame());
        }
    }

    #[test]
    fn writes_on_the_cursor_row_without_moving_it_still_repaint() {
        let (mut e, p, mut s, m) = setup(2);
        e.advance(b"ab", 0.0);
        run(&mut e, &p, &mut s, &m, FOCUSED);
        // Guarda el cursor, escribe en la misma fila y lo restaura.
        e.advance(b"\x1b7\x1b[1;6HZ\x1b8", 0.0);
        s.absorb(&mut e, FOCUSED);
        assert!(s.wants_frame());
        let (_, calls) = run(&mut e, &p, &mut s, &m, FOCUSED);
        assert_eq!(calls.first(), Some(&Call::Row(0, "ab   Z".into())));
    }

    #[test]
    fn blink_follows_xterm_timing() {
        let mut b = Blink::new(true, 1000.0);
        assert!(b.visible(1000.0));
        assert!(b.visible(1599.0));
        assert!(!b.visible(1600.0));
        assert!(b.visible(2200.0));
        assert_eq!(b.next_toggle(1000.0), Some(1600.0));
        assert_eq!(b.next_toggle(1700.0), Some(2200.0));
        b.restart(1700.0);
        assert!(b.visible(1700.0));
        assert_eq!(b.next_toggle(1700.0), Some(2300.0));
        let steady = Blink::new(false, 0.0);
        assert!(steady.visible(900.0));
        assert_eq!(steady.next_toggle(900.0), None);
    }

    #[test]
    fn reenabled_blink_starts_visible() {
        let mut b = Blink::new(true, 0.0);
        assert!(!b.set_enabled(true, 700.0));
        assert!(!b.visible(700.0));
        // Fuera de la vista (historia desplazada) o sin foco: se apaga.
        assert!(!b.set_enabled(false, 900.0));
        // Al volver, el ciclo empieza en la fase visible.
        assert!(b.set_enabled(true, 1900.0));
        assert!(b.visible(1900.0));
        assert_eq!(b.next_toggle(1900.0), Some(2500.0));
    }
}
