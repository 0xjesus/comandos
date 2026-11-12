//! Orden GET del Python y residuo estático/desconocido de 2f-3/T7.
mod static_files;
use super::{Answer, Entry, Fault, Key, Native, NativeRoute, Verb, reply, retired, target};
use crate::{HandlerError, Request};
use http::{Method, StatusCode};
use serde_json::json;
pub use static_files::serve as static_response;
use std::sync::Arc;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidueRoute {
    GetFallback,
    HeadFallback,
    PostUnknown,
    DeleteUnknown,
}
const fn entry(verb: Verb, route: ResidueRoute) -> Entry {
    Entry {
        verb,
        key: Key::Prefix("/"),
        route: NativeRoute::Residue(route),
    }
}
pub const ROUTES: &[Entry] = &[
    entry(Verb::Get, ResidueRoute::GetFallback),
    entry(Verb::Head, ResidueRoute::HeadFallback),
    entry(Verb::Post, ResidueRoute::PostUnknown),
    entry(Verb::Delete, ResidueRoute::DeleteUnknown),
];

/// Orden de `_do_GET`, antes de agrupar por dominio. La retirada deliberada
/// de `/proxy` conserva también consultas; `/events/v2` precede a `/events`.
const GET_ORDER: &[Key] = &[
    Key::Prefix("/operator"),
    Key::Path("/session-config-history"),
    Key::Prefix("/pane-extensions"),
    Key::Prefix("/webterm-token"),
    Key::Prefix("/state"),
    Key::Prefix("/accounts"),
    Key::Prefix("/providers"),
    Key::Prefix("/optimization/plans"),
    Key::Prefix("/opencode/models"),
    Key::Prefix("/model-tiers"),
    Key::Prefix("/tab-models"),
    Key::Prefix("/active-tab"),
    Key::Prefix("/dedication"),
    Key::Prefix("/analytics/week"),
    Key::Prefix("/pomodoro/report"),
    Key::Raw("/pomodoro"),
    Key::Raw("/sovereignty"),
    Key::Prefix("/model/status"),
    Key::Path("/proxy"),
    Key::Prefix("/ui-log/summary"),
    Key::Prefix("/session-profiles"),
    Key::Prefix("/extension-usage"),
    Key::Prefix("/session-brain"),
    Key::Prefix("/usage/guard"),
    Key::Prefix("/usage/changes"),
    Key::Prefix("/notifs/count"),
    Key::Prefix("/news/latest"),
    Key::Path("/push/key"),
    Key::Path("/news/editions"),
    Key::Prefix("/news/media/"),
    Key::Path("/news/source"),
    Key::Path("/news/chat"),
    Key::Path("/news/notes"),
    Key::Path("/news/saved"),
    Key::Path("/news/edition"),
    Key::Prefix("/models/latest"),
    Key::Prefix("/fs/dirs"),
    Key::Prefix("/usage/provider-compare"),
    Key::Prefix("/usage/experiments"),
    Key::Prefix("/usage/analytics"),
    Key::Prefix("/usage/interactions"),
    Key::Prefix("/usage/state"),
    Key::Prefix("/project-profiles"),
    Key::Prefix("/ssh"),
    Key::Prefix("/conf"),
    Key::Prefix("/prefs"),
    Key::ExactOrQuery("/tmux-mouse"),
    Key::Path("/workspace"),
    Key::Path("/notices/watch"),
    Key::Path("/notices"),
    Key::Path("/notices/prefs"),
    Key::Path("/workspace/close-group"),
    Key::Path("/workspace/client"),
    Key::Prefix("/tabs"),
    Key::Prefix("/tab-history"),
    Key::Prefix("/remote-state"),
    Key::Prefix("/remote-qr.png"),
    Key::Path("/work-marks"),
    Key::Path("/events/v2"),
    Key::Prefix("/events"),
    Key::Prefix("/commands/catalog"),
    Key::Raw("/snippets"),
    Key::Raw("/chains"),
];

pub(super) fn canonical_get_target(target: &str) -> Option<&'static str> {
    GET_ORDER.iter().find_map(|key| {
        if !key.matches(target) {
            return None;
        }
        Some(match key {
            Key::Path(path) | Key::Raw(path) | Key::Prefix(path) | Key::ExactOrQuery(path) => *path,
        })
    })
}

pub(super) fn ordered_get_route(target: &str) -> Option<NativeRoute> {
    let canonical = canonical_get_target(target)?;
    if canonical == "/operator" {
        return Some(NativeRoute::Retired);
    }
    // Solo busca la tabla canónica, sin volver a aplicar GET_ORDER. El
    // despachador recibe la ruta original y conserva el corte de su dueño.
    super::route_in_tables(&Method::GET, canonical)
}

// Comparaciones literales de Handler.do_POST, conservadas como gate de cobertura.
#[cfg(test)]
const CANONICAL_POST: &[&str] = &[
    "/pane-extensions",
    "/pane-extensions/apply",
    "/pane-extensions/template",
    "/pane-extensions/cancel",
    "/pane-extensions/recover",
    "/workspace/sort",
    "/workspace",
    "/notify-popup",
    "/presence",
    "/notices/read",
    "/notices/sound",
    "/notices/prefs",
    "/workspace/close-group",
    "/work-marks",
    "/events/v2",
    "/workspace/client",
    "/session/recover",
    "/terminal-history",
    "/terminal-panes",
    "/open-path",
    "/project-profile",
    "/optimization/default",
    "/proxy",
    "/ui-log",
    "/pomodoro",
    "/conf-set",
    "/session-profiles",
    "/session-profile-apply",
    "/skill-toggle",
    "/mcp-toggle",
    "/pause",
    "/event",
    "/test",
    "/remote-on",
    "/remote-off",
    "/remote-webterm-on",
    "/remote-webterm-off",
    "/ssh-add",
    "/ssh-del",
    "/ssh-update",
    "/prefs-set",
    "/usage/experiment",
    "/usage/rating",
    "/usage/capture",
    "/usage/refresh",
    "/push/subscription",
    "/push/test",
    "/news/saved",
    "/news/notes",
    "/news/chat",
    "/news/chat/note",
    "/news/translate",
    "/news/refresh",
    "/open-url",
    "/models/refresh",
    "/fs/mkdir",
    "/usage/quota",
    "/usage/subscription",
    "/usage/settings",
    "/ssh-connect",
    "/ssh-new-tab",
    "/ssh-key-setup",
    "/chains",
    "/chains/delete",
    "/snippets",
    "/snippets/update",
    "/snippets/delete",
    "/session-new",
    "/open-with-account",
    "/terminal/quick",
    "/tab-new",
    "/account/add",
    "/account/remove",
    "/account/rename",
    "/app/command",
    "/tmux-mouse",
    "/tmux-scroll",
    "/tab-register",
    "/tab-metadata",
    "/tab-metadata-remove",
    "/tab-close",
    "/recover-tab",
    "/session/configure",
    "/account/switch",
    "/harness/switch",
    "/model/switch-cancel",
    "/model/switch",
    "/pane/type",
    "/send",
    "/paste",
    "/key",
    "/export",
    "/kill",
    "/focus",
    "/ensure",
    "/new",
    "/shell",
    "/up",
];
pub async fn answer(native: &Arc<Native>, route: ResidueRoute, request: &Request) -> Answer {
    if matches!(request.method, Method::GET | Method::POST)
        && request.target.starts_with("/operator")
    {
        return retired::answer(&request.method, &request.target);
    }
    match route {
        ResidueRoute::GetFallback | ResidueRoute::HeadFallback => {
            let root = native.options().dash_dir.as_deref().ok_or(Fault::Decline)?;
            static_response(root, request).await
        }
        ResidueRoute::PostUnknown => {
            let data = request
                .data
                .as_ref()
                .ok_or(Fault::Error(HandlerError::Failure))?;
            match target::post_target(native, &request.target, data).await {
                Ok(_) => reply(StatusCode::NOT_FOUND, &json!({"error":"Ruta desconocida"})),
                Err(answer) => answer,
            }
        }
        ResidueRoute::DeleteUnknown => {
            super::delete_body(request)?;
            reply(StatusCode::NOT_FOUND, &json!({"error":"No encontrado"}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_merged_dynamic_branch_has_a_domain_owner() {
        for key in GET_ORDER {
            let path = match key {
                Key::Path(p) | Key::Raw(p) | Key::Prefix(p) | Key::ExactOrQuery(p) => *p,
            };
            assert!(
                !matches!(
                    super::super::route(&Method::GET, path),
                    None | Some(NativeRoute::Residue(_))
                ),
                "GET {path}"
            );
        }
        for path in CANONICAL_POST {
            assert!(
                !matches!(
                    super::super::route(&Method::POST, path),
                    None | Some(NativeRoute::Residue(_))
                ),
                "POST {path}"
            );
        }
    }
}
