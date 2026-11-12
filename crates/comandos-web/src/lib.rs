//! `comandos-web`: el WASM del tablero (Fase 3b). Lo compila
//! `cargo run -p xtask -- web-build --crate comandos-web`; el manifiesto
//! (`comandos_core::web_assets::Manifest`) lo nombra `comandos_web.js`,
//! `comandos_web_bg.wasm` y `comandos_web_boot.js` (el cargador que inyecta el
//! frente, B2).
//!
//! ## Registro de componentes (preflight R11)
//!
//! Cada componente portado tiene:
//!
//! - su `mount` en [`registry::COMPONENTS`], y
//! - su entrada en `crates/comandos-web/components/<id>.json` (un archivo por
//!   componente, sin lista común): `id`, `kind` (`script`, `region` o `page`),
//!   `source`, `marker_start`/`marker_end`/`script_index` (regiones), `sha256`
//!   del origen portado, `exports` y `deps`. La crea y la refresca
//!   `cargo run -p xtask -- web-port pin <id>`; el frente (B2) junta los
//!   archivos por ruta (sin depender de este crate, que arrastraría `web-sys`).
//!
//! `tests/registry.rs` exige que ambos lados coincidan y que `exports` declare
//! todos los globales del componente que consume algo de fuera
//! (otra unidad, `cc-app`, `cc-app-mac` o un iframe).
//!
//! ## `interop.json`
//!
//! No hay copia en este crate: la fuente única es `xtask/web/interop.json`
//! (y `inventory.json`), que genera `xtask web-inventory` (B3). Las pruebas la
//! leen de su sitio con `include_str!`, así que regenerar el inventario
//! revalida los `exports` sin duplicar nada a mano.

//! ## Pánicos
//!
//! `release-wasm` compila con `panic = "abort"`: un pánico de Rust es una
//! trampa (`unreachable`). La frontera de cada `mount`/`attach` (una llamada
//! desde JS con `try/catch`, ver `registry`) la convierte en una entrada de
//! `failed` y los demás componentes siguen, pero tras una trampa el estado de
//! Rust (préstamos de `RefCell`, el asignador) no es fiable. Por eso los tres
//! crates web niegan a nivel de crate `unwrap`, `expect`, la indexación y el corte
//! de `str` que pueden fallar, `panic!`, `unreachable!`, `todo!` y `unimplemented!`
//! (y `scripts/rust-check` corre ese clippy también para `wasm32`). Eso cierra las
//! vías explícitas en código nuestro; no cubre `assert!`, `RefCell::borrow*`
//! (se usa `try_borrow*`), un `OnceCell` reentrante, el desbordamiento de
//! capacidad de `Vec`/`String` ni el código de las dependencias.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::panic,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

pub mod registry;

pub mod components {
    pub mod app_coordinator;
    pub mod chat_view;
    pub mod command_sidebar;
    #[cfg(target_arch = "wasm32")]
    pub(crate) mod content_runtime;
    pub mod device_drafts;
    pub mod extensions;
    pub mod foundation;
    pub mod foundation_helpers;
    pub mod foundation_lifecycle;
    pub mod foundation_navigation;
    pub mod foundation_remote;
    pub mod foundation_snippets;
    pub mod foundation_system;
    pub mod foundation_tail;
    pub mod foundation_ui;
    pub mod news_reader;
    pub mod push_settings;
    pub mod quick_terminal;
    pub mod servers;
    pub mod session_config;
    #[cfg(target_arch = "wasm32")]
    pub(crate) mod session_feed;
    pub mod theme_preferences;
    pub mod ui_sounds;
    #[cfg(target_arch = "wasm32")]
    pub(crate) mod web_support;
    pub mod work_marks;
    pub mod workspace;
    pub mod workspace_dock;
    pub mod workspace_layout;
}

#[cfg(target_arch = "wasm32")]
pub use components::content_runtime::{install_content_loader, prepare_content};
#[cfg(target_arch = "wasm32")]
pub use registry::{boot, dependency_failed, needs_content, needs_sound};
