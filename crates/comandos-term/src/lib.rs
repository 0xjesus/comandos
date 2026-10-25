//! `comandos-term`: terminal de ComandOS sin ttyd ni xterm.js. Crate puro
//! (host y `wasm32-unknown-unknown`): no abre PTY, no habla con tmux.
//!
//! ## Diferencias deliberadas con xterm.js 5.5.0
//!
//! El motor (`engine`) se comporta como xterm.js donde la aplicación lo nota
//! (respuestas DA1/DA2, paleta), con una ampliación a propósito: entiende las
//! actualizaciones sincronizadas (modo privado 2026, `CSI ? 2026 h/l`), que
//! xterm.js 5.5.0 no tiene, y por tanto contesta a DECRQM `CSI ? 2026 $ p`
//! con `CSI ? 2026 ; 2 $ y` (admitido, desactivado) donde xterm.js diría
//! `; 0 $ y` (desconocido). Claude Code y otras TUI lo usan para no parpadear
//! al redibujar; una aplicación que no lo pida no ve ninguna diferencia.

pub mod clock;
pub mod crop;
pub mod engine;
pub mod glyphs;
pub mod input;
pub mod osc;
pub mod proto;
pub mod render;
pub mod select;
