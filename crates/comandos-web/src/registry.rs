//! Componentes portados, su montaje y su enganche al DOM.
//!
//! `boot` lo llama el cargador generado (`boot.js`) con el nonce de la
//! compuerta, en dos fases por componente:
//!
//! En modo gradual, `mount` publica los globales mientras se analiza el head
//! y avisa a la compuerta antes de `attach` en DOMContentLoaded.
//!
//! Una página completa usa `<meta name="comandos-web-mode" content="native">`
//! y un loader de módulo sin gate. Allí ambas fases esperan al DOM: todos los
//! mounts publican primero, los attaches registran controles y UI general
//! inicia al final. Ready exige mounts, attaches y startup asíncrono completos,
//! además de la aceptación del nonce por el servidor. Los errores quedan en
//! `window.__comandosBootReport`; una página fallida no anuncia ready.
//!
//! Cada `mount` y cada `attach` se llama desde JS con `try/catch`
//! (`Function.prototype.call` de `js-sys`, que captura): un `Err`, una
//! excepción de JS o una trampa quedan en `failed` y no impiden los demás.
//! `boot` es idempotente: una segunda llamada en la misma página no hace nada.

use wasm_bindgen::JsValue;

pub use comandos_web_dom::dom_ready;

/// Una fase de un componente (`mount` o `attach`).
pub type Phase = fn() -> Result<(), JsValue>;

/// Un componente portado: su id (el de `components/<id>.json`), su montaje y
/// su enganche al DOM.
pub struct Component {
    pub id: &'static str,
    pub mount: Phase,
    pub attach: Option<Phase>,
}

/// Componentes que este WASM sabe montar. Crece con cada port (B4…B13).
pub const COMPONENTS: &[Component] = &[
    Component {
        id: "app-combinada",
        mount: crate::components::app_coordinator::mount_app,
        attach: None,
    },
    Component {
        id: "identidad-de-fila",
        mount: crate::components::app_coordinator::mount_identity,
        attach: None,
    },
    Component {
        id: "barra-de-comandos",
        mount: crate::components::app_coordinator::mount_sidebar,
        attach: None,
    },
    Component {
        id: "render",
        mount: crate::components::app_coordinator::mount_render,
        attach: None,
    },
    Component {
        id: "servidores",
        mount: crate::components::servers::mount,
        attach: Some(crate::components::servers::attach),
    },
    Component {
        id: "notificaciones-del-sistema",
        mount: crate::components::foundation_system::mount_notifications,
        attach: Some(crate::components::foundation_system::attach_notifications),
    },
    Component {
        id: "modales-en-medio",
        mount: crate::components::foundation_system::mount_panels,
        attach: Some(crate::components::foundation_system::attach_panels),
    },
    Component {
        id: "helpers",
        mount: crate::components::foundation_helpers::mount,
        attach: Some(crate::components::foundation_helpers::attach),
    },
    Component {
        id: "remoto",
        mount: crate::components::foundation_remote::mount,
        attach: Some(crate::components::foundation_remote::attach),
    },
    Component {
        id: "tema",
        mount: crate::components::theme_preferences::mount_theme,
        attach: Some(crate::components::theme_preferences::attach_theme),
    },
    Component {
        id: "favoritos",
        mount: crate::components::theme_preferences::mount_favorites,
        attach: None,
    },
    Component {
        id: "prefs-de-terminal",
        mount: crate::components::theme_preferences::mount_prefs,
        attach: None,
    },
    Component {
        id: "tabs-internos-de-modales",
        mount: crate::components::theme_preferences::mount_tabs,
        attach: Some(crate::components::theme_preferences::attach_tabs),
    },
    Component {
        id: "solid-range-fill",
        mount: crate::components::theme_preferences::mount_ranges,
        attach: Some(crate::components::theme_preferences::attach_ranges),
    },
    Component {
        id: "preview-de-tipografia",
        mount: crate::components::theme_preferences::mount_preview,
        attach: Some(crate::components::theme_preferences::attach_preview),
    },
    Component {
        id: "tail",
        mount: crate::components::foundation_tail::mount,
        attach: None,
    },
    Component {
        id: "snippets",
        mount: crate::components::foundation_snippets::mount,
        attach: Some(crate::components::foundation_snippets::attach),
    },
    Component {
        id: "ui-general",
        mount: crate::components::foundation_ui::mount_ui,
        attach: Some(crate::components::foundation_ui::attach_ui),
    },
    Component {
        id: "funciones-de-ui-globales",
        mount: crate::components::foundation_ui::mount_globals,
        attach: None,
    },
    Component {
        id: "analytics-inline",
        mount: crate::components::foundation_ui::mount_analytics,
        attach: Some(crate::components::foundation_ui::attach_analytics),
    },
    Component {
        id: "conmutador-ctrl-k",
        mount: crate::components::foundation_navigation::mount_switcher,
        attach: Some(crate::components::foundation_navigation::attach_switcher),
    },
    Component {
        id: "iconos",
        mount: crate::components::foundation_navigation::mount_icons,
        attach: Some(crate::components::foundation_navigation::attach_icons),
    },
    Component {
        id: "app-nativa",
        mount: crate::components::foundation_lifecycle::mount_app,
        attach: Some(crate::components::foundation_lifecycle::attach_app),
    },
    Component {
        id: "toasts",
        mount: crate::components::foundation_lifecycle::retire_toasts_separator,
        attach: None,
    },
    Component {
        id: "registro-local-de-uso",
        mount: crate::components::foundation_lifecycle::mount_log,
        attach: Some(crate::components::foundation_lifecycle::attach_log),
    },
    Component {
        id: "avisos-de-eventos",
        mount: crate::components::foundation_lifecycle::mount_notices,
        attach: None,
    },
    Component {
        id: "loop",
        mount: crate::components::foundation_lifecycle::mount_poll,
        attach: Some(crate::components::foundation_lifecycle::attach_poll),
    },
    Component {
        id: "prelude",
        mount: crate::components::foundation::mount_state,
        attach: None,
    },
    Component {
        id: "i18n",
        mount: crate::components::foundation::mount_i18n,
        attach: None,
    },
    Component {
        id: "red",
        mount: crate::components::foundation::mount_network,
        attach: None,
    },
    Component {
        id: "command-sidebar",
        mount: crate::components::command_sidebar::mount,
        attach: None,
    },
    Component {
        id: "workspace",
        mount: crate::components::workspace::mount,
        attach: Some(crate::components::workspace::attach),
    },
    Component {
        id: "workspace-dock",
        mount: crate::components::workspace_dock::mount,
        attach: Some(crate::components::workspace_dock::attach),
    },
    Component {
        id: "extensions",
        mount: crate::components::extensions::mount,
        attach: Some(crate::components::extensions::attach),
    },
    Component {
        id: "vendor-markdown-it",
        mount: crate::components::news_reader::retire_vendor,
        attach: None,
    },
    Component {
        id: "vendor-purify",
        mount: crate::components::news_reader::retire_vendor,
        attach: None,
    },
    Component {
        id: "news-reader",
        mount: crate::components::news_reader::mount,
        attach: Some(crate::components::news_reader::attach),
    },
    Component {
        id: "work-marks",
        mount: crate::components::work_marks::mount,
        attach: Some(crate::components::work_marks::attach),
    },
    Component {
        id: "ui-sounds",
        mount: crate::components::ui_sounds::mount,
        attach: None,
    },
    Component {
        id: "quick-terminal",
        mount: crate::components::quick_terminal::mount,
        attach: None,
    },
    Component {
        id: "device-drafts",
        mount: crate::components::device_drafts::mount,
        attach: None,
    },
    Component {
        id: "session-config",
        mount: crate::components::session_config::mount,
        attach: None,
    },
    Component {
        id: "workspace-layout",
        mount: crate::components::workspace_layout::mount,
        attach: None,
    },
    Component {
        id: "push-settings",
        mount: crate::components::push_settings::mount,
        attach: Some(crate::components::push_settings::attach),
    },
];

/// Error de un id que la página pide y este WASM no tiene.
pub const UNKNOWN: &str = "componente desconocido en este WASM";

pub fn find(id: &str) -> Option<&'static Component> {
    find_in(COMPONENTS, id)
}

pub fn find_in<'a>(components: &'a [Component], id: &str) -> Option<&'a Component> {
    components.iter().find(|c| c.id == id)
}

/// Ids de `<meta name="comandos-web" content="id1 id2 …">`, en orden y sin
/// repetidos (un id repetido se monta una vez).
pub fn meta_ids(content: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for id in content.split_ascii_whitespace() {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Resultado del montaje, para `POST /web/ready`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub mounted: Vec<String>,
    /// `(id, error)`.
    pub failed: Vec<(String, String)>,
}

/// Monta `ids` en orden. `run(id)` monta uno (`None` si este WASM no lo
/// conoce). Un componente que falla queda en `failed` y no impide montar los
/// demás: decide el servidor (B2).
pub fn mount_all<E>(
    ids: &[&str],
    mut run: impl FnMut(&str) -> Option<Result<(), E>>,
    describe: impl Fn(&E) -> String,
) -> Report {
    let mut r = Report::default();
    // `String::from` y no `to_string()`: sobre `&&str` este pasa por
    // `core::fmt` y mete el formateador en el arranque vacío.
    for &id in ids {
        match run(id) {
            Some(Ok(())) => r.mounted.push(String::from(id)),
            Some(Err(e)) => r.failed.push((String::from(id), describe(&e))),
            None => r.failed.push((String::from(id), String::from(UNKNOWN))),
        }
    }
    r
}

/// Los ids que pasan a `attach`: los montados que lo tienen, en su orden.
pub fn attach_order(r: &Report, has_attach: impl Fn(&str) -> bool) -> Vec<&str> {
    r.mounted
        .iter()
        .map(String::as_str)
        .filter(|id| has_attach(id))
        .collect()
}

/// Explicit full-page opt-in; unmarked pages retain the gradual gate protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootMode {
    Gradual,
    Native,
}
pub fn boot_mode(value: &str) -> BootMode {
    if value == "native" {
        BootMode::Native
    } else {
        BootMode::Gradual
    }
}
/// Vendor retirement checks require the native reader export. The original
/// page loaded vendors first; their Rust replacements depend on news-reader.
/// No other relative order or selected component changes.
pub fn native_mount_order(mut ids: Vec<&str>) -> Vec<&str> {
    if let Some(reader) = ids.iter().position(|id| *id == "news-reader")
        && let Some(vendor) = ids
            .iter()
            .position(|id| matches!(*id, "vendor-markdown-it" | "vendor-purify"))
        && reader > vendor
    {
        let id = ids.remove(reader);
        ids.insert(vendor, id);
    }
    ids
}
/// All globals have mounted before attach. Native startup runs after every
/// listener/constructor attach; other relative ordering remains the page's.
pub fn phase_attach_order(
    r: &Report,
    has_attach: impl Fn(&str) -> bool,
    mode: BootMode,
) -> Vec<&str> {
    let mut ids = attach_order(r, has_attach);
    if mode == BootMode::Native
        && let Some(index) = ids.iter().position(|id| *id == "ui-general")
    {
        let startup = ids.remove(index);
        ids.push(startup);
    }
    ids
}

/// `boot` solo corre si es la primera vez en esta instancia y la página no
/// estaba ya lista (`window.__comandosReady === true`).
pub fn first_boot(booted: bool, page_ready: bool) -> bool {
    !booted && !page_ready
}

#[cfg(target_arch = "wasm32")]
pub use web::{boot, boot_with, dependency_failed, needs_content, needs_sound};

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{
        BootMode, COMPONENTS, Component, Phase, Report, boot_mode, find_in, first_boot, meta_ids,
        mount_all, native_mount_order, phase_attach_order,
    };
    use comandos_web_dom::bridge::{call_global, global_get, global_set, js_text};
    use js_sys::{Array, Function, JSON, Object, Promise, Reflect};
    use std::cell::Cell;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::wasm_bindgen;
    use wasm_bindgen::{JsCast, JsValue};

    thread_local! {
        static BOOTED: Cell<bool> = const { Cell::new(false) };
    }

    #[wasm_bindgen]
    pub fn needs_content() -> bool {
        let content = comandos_web_dom::dom::query(r#"meta[name="comandos-web"]"#)
            .and_then(|m| m.get_attribute("content"))
            .unwrap_or_default();
        meta_ids(&content).iter().any(|id| {
            matches!(
                *id,
                "news-reader"
                    | "ui-sounds"
                    | "vendor-markdown-it"
                    | "vendor-purify"
            )
        })
    }
    #[wasm_bindgen]
    pub fn needs_sound() -> bool {
        let content = comandos_web_dom::dom::query(r#"meta[name="comandos-web"]"#)
            .and_then(|m| m.get_attribute("content"))
            .unwrap_or_default();
        meta_ids(&content).contains(&"ui-sounds")
    }
    #[wasm_bindgen]
    pub fn dependency_failed(k: &str, error: JsValue) {
        let _ = BOOTED.try_with(|b| b.set(true));
        publish_report(
            k,
            &Report {
                mounted: Vec::new(),
                failed: vec![("native-sound".into(), describe(&error))],
            },
            "failed",
        );
    }

    /// Lee `<meta name="comandos-web">`, monta en ese orden, marca
    /// `window.__comandosReady = true`, avisa `POST /web/ready
    /// {"k", "mounted", "failed": [{"id", "error"}]}` y deja `attach` para
    /// cuando el DOM esté listo. Una segunda llamada no hace nada.
    #[wasm_bindgen]
    pub fn boot(k: &str) {
        let page_ready = global_get("__comandosReady") == JsValue::TRUE;
        // `try_with` y no `with`: este último trae la ruta de pánico de los
        // `thread_local` (y su formateador) al arranque vacío.
        let booted = BOOTED.try_with(|b| b.replace(true)).unwrap_or(true);
        if !first_boot(booted, page_ready) {
            return;
        }
        boot_with(k, COMPONENTS);
    }

    /// `boot` sobre una lista dada y sin la guarda de idempotencia (pruebas).
    pub fn boot_with(k: &str, components: &'static [Component]) {
        let content = comandos_web_dom::dom::query(r#"meta[name="comandos-web"]"#)
            .and_then(|m| m.get_attribute("content"))
            .unwrap_or_default();
        let mode = comandos_web_dom::dom::query(r#"meta[name="comandos-web-mode"]"#)
            .and_then(|m| m.get_attribute("content"))
            .map_or(BootMode::Gradual, |v| boot_mode(&v));
        if mode == BootMode::Native {
            let k = k.to_owned();
            let _ = global_set("__comandosBootState", &"waiting-dom".into());
            comandos_web_dom::dom::on_ready(move || boot_native(&k, &content, components));
            return;
        }
        let ids = meta_ids(&content);
        let report = mount_all(
            &ids,
            |id| find_in(components, id).map(|c| guarded(c.mount)),
            describe,
        );
        let _ = global_set("__comandosReady", &JsValue::TRUE);
        let _ = post_ready(k, &report);
        let attach: Vec<(String, Phase)> = phase_attach_order(
            &report,
            |id| find_in(components, id).is_some_and(|c| c.attach.is_some()),
            BootMode::Gradual,
        )
        .into_iter()
        .filter_map(|id| {
            find_in(components, id)
                .and_then(|c| c.attach)
                .map(|f| (String::from(id), f))
        })
        .collect();
        comandos_web_dom::dom::on_ready(move || attach_all(&attach));
    }

    fn publish_report(k: &str, report: &Report, status: &str) {
        if let Ok(bytes) = body(k, report)
            && let Some(text) = bytes.as_string()
            && let Ok(value) = JSON::parse(&text)
        {
            let _ = global_set("__comandosBootReport", &value);
        }
        let _ = global_set("__comandosBootState", &status.into());
    }
    fn boot_native(k: &str, content: &str, components: &'static [Component]) {
        let _ = global_set("__comandosBootState", &"mounting".into());
        let mut report = mount_all(
            &native_mount_order(meta_ids(content)),
            |id| find_in(components, id).map(|c| guarded(c.mount)),
            describe,
        );
        if !report.failed.is_empty() {
            publish_report(k, &report, "failed");
            return;
        }
        publish_report(k, &report, "attaching");
        let order = phase_attach_order(
            &report,
            |id| find_in(components, id).is_some_and(|c| c.attach.is_some()),
            BootMode::Native,
        )
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        for id in &order {
            // Never launch startup after a failed control attach.
            if id == "ui-general" && !report.failed.is_empty() {
                break;
            }
            if let Some(phase) = find_in(components, id).and_then(|c| c.attach)
                && let Err(error) = guarded(phase)
            {
                report.failed.push((id.clone(), describe(&error)));
            }
        }
        if !report.failed.is_empty() {
            publish_report(k, &report, "failed");
            return;
        }
        let startup = global_get("__comandosStartup");
        let needs_startup = order.iter().any(|id| id == "ui-general");
        if needs_startup && !Reflect::get(&startup, &"then".into()).is_ok_and(|v| v.is_function()) {
            report.failed.push((
                "ui-general".into(),
                "native startup promise unavailable".into(),
            ));
            publish_report(k, &report, "failed");
            return;
        }
        publish_report(k, &report, "starting");
        let k = k.to_owned();
        wasm_bindgen_futures::spawn_local(async move {
            if needs_startup
                && let Err(error) =
                    wasm_bindgen_futures::JsFuture::from(Promise::resolve(&startup)).await
            {
                report.failed.push(("ui-general".into(), describe(&error)));
                publish_report(&k, &report, "failed");
                return;
            }
            publish_report(&k, &report, "reporting");
            if let Err(error) = post_ready_native(&k, &report).await {
                report.failed.push(("@ready".into(), describe(&error)));
                publish_report(&k, &report, "failed");
                return;
            }
            let _ = global_set("__comandosAttached", &JsValue::TRUE);
            let _ = global_set("__comandosReady", &JsValue::TRUE);
            publish_report(&k, &report, "ready");
        });
    }
    async fn post_ready_native(k: &str, report: &Report) -> Result<(), JsValue> {
        let win: JsValue = web_sys::window().ok_or(JsValue::NULL)?.into();
        let options = Object::new();
        set(&options, "method", &"POST".into())?;
        let headers = Object::new();
        set(&headers, "Content-Type", &"application/json".into())?;
        set(&options, "headers", &headers)?;
        set(&options, "body", &body(k, report)?)?;
        let fetch: Function = Reflect::get(&win, &"fetch".into())?.dyn_into()?;
        let result = fetch.call2(&win, &"/web/ready".into(), &options)?;
        let response = wasm_bindgen_futures::JsFuture::from(Promise::resolve(&result)).await?;
        if Reflect::get(&response, &"ok".into())? != JsValue::TRUE {
            return Err(js_sys::Error::new("native readiness rejected by server").into());
        }
        Ok(())
    }

    fn attach_all(list: &[(String, Phase)]) {
        for (id, f) in list {
            if let Err(e) = guarded(*f) {
                // `window.ulog` ya existe en `DOMContentLoaded` (la región en JS
                // o la exportación de su port); llamarlo por el puente no mete
                // el registro nativo en el arranque.
                let args = [
                    "web-attach".into(),
                    id.as_str().into(),
                    describe(&e).into(),
                    JsValue::from_f64(0.0),
                ];
                let _ = call_global("ulog", &args);
            }
        }
        let _ = global_set("__comandosAttached", &JsValue::TRUE);
    }

    /// Texto de un fallo: `message` de un `Error`, o `String(e)`.
    fn describe(e: &JsValue) -> String {
        Reflect::get(e, &"message".into())
            .ok()
            .and_then(|m| m.as_string())
            .unwrap_or_else(|| js_text(e))
    }

    /// Llama a `f` desde JS con `try/catch`: un `Err`, una excepción de JS que
    /// atraviese el WASM o una trampa vuelven como `Err`.
    fn guarded(f: Phase) -> Result<(), JsValue> {
        let c = Closure::<dyn FnMut() -> Result<(), JsValue>>::new(f);
        let fun: &Function = c.as_ref().unchecked_ref();
        fun.call0(&JsValue::UNDEFINED).map(|_| ())
    }

    fn set(o: &Object, key: &str, v: &JsValue) -> Result<(), JsValue> {
        Reflect::set(o, &key.into(), v).map(|_| ())
    }

    /// Cuerpo de `/web/ready`, con las claves en el orden del plan.
    fn body(k: &str, r: &Report) -> Result<JsValue, JsValue> {
        let o = Object::new();
        set(&o, "k", &k.into())?;
        let mounted: Array = r
            .mounted
            .iter()
            .map(|s| JsValue::from(s.as_str()))
            .collect();
        set(&o, "mounted", &mounted)?;
        let failed = Array::new();
        for (id, error) in &r.failed {
            let f = Object::new();
            set(&f, "id", &id.as_str().into())?;
            set(&f, "error", &error.as_str().into())?;
            failed.push(&f);
        }
        set(&o, "failed", &failed)?;
        Ok(JSON::stringify(&o)?.into())
    }

    /// `fetch("/web/ready", …)` sin esperar respuesta: si no llega, la
    /// compuerta revierte sola a los 8 s (`?web=off`).
    fn post_ready(k: &str, r: &Report) -> Result<(), JsValue> {
        let win: JsValue = web_sys::window().ok_or(JsValue::NULL)?.into();
        let opt = Object::new();
        set(&opt, "method", &"POST".into())?;
        let headers = Object::new();
        set(&headers, "Content-Type", &"application/json".into())?;
        set(&opt, "headers", &headers)?;
        set(&opt, "body", &body(k, r)?)?;
        let fetch: Function = Reflect::get(&win, &"fetch".into())?.dyn_into()?;
        let p = Promise::resolve(&fetch.call2(&win, &"/web/ready".into(), &opt)?);
        // Rechazo atendido sin clausura: `p.catch(Boolean)` con el `Boolean`
        // del motor.
        let catch: Function = Reflect::get(&p, &"catch".into())?.dyn_into()?;
        let boolean = Reflect::get(&js_sys::global(), &"Boolean".into())?;
        catch.call1(&p, &boolean)?;
        Ok(())
    }
}
