//! «Ficha viva» glue (design/ficha-viva-spec.md §2–4): polls `GET /session-feed`, paints the
//! K5 marks on the remote top-bar tiles (`#tabbar .apptab`), the «Sesiones» feed of the
//! left sidebar, the B4 preview card (hover; long press on touch) and posts
//! `POST /session-seen` for the session open in the remote.
//!
//! Model and HTML live in `comandos_web_view::session_feed` (host-tested). Here: state,
//! timers and events.
//!
//! - Remote (`WEBTERM`): tiles and feed are both in the page; hovering one highlights the
//!   other. A row opens the session like a tab and selects its pane in the frame.
//! - Desktop app (`inApp()`): the top bar is native GTK. A row asks the app to open the
//!   session (`{session, win, label}`, then `/terminal-panes` select, like the notices),
//!   row hover is sent as `{"feedHover": "<session>"}` (`""` on leave) and the app
//!   highlights a row through `window.ComandosFeed.hover(session)`. The app posts «seen».
//! - Absent route (404 until the server ships it): nothing new shows and polling backs off.
use crate::components::web_support::*;
use comandos_web_dom::port::*;
use comandos_web_view::session_feed as view;
use serde_json::json;
use std::cell::RefCell;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;

const POLL_MS: f64 = 2500.0;
/// The route answered but is not the feed (old server): look again later, quietly.
const ABSENT_MS: f64 = 60_000.0;
const OFFLINE_MS: f64 = 10_000.0;
const PEEK_DELAY_MS: f64 = 350.0;
const PEEK_GRACE_MS: f64 = 150.0;
const LONG_PRESS_MS: f64 = 450.0;
/// `comandos-notifyd` `choose`: the digit, then `Enter` 250 ms later.
const CHOOSE_DELAY_MS: f64 = 250.0;
const SEEN_THROTTLE_MS: f64 = 2000.0;
const CHIP_KEY: &str = "comandos.feed.chip";

struct Peek {
    session: String,
    touch: bool,
    el: JsValue,
}

#[derive(Default)]
struct Feed {
    data: Option<view::Feed>,
    chip: view::Chip,
    busy: bool,
    until: f64,
    hover: String,
    peek: Option<Peek>,
    peek_timer: Option<JsValue>,
    hide_timer: Option<JsValue>,
    press_timer: Option<JsValue>,
    press: Option<(f64, f64)>,
    swallow_click: bool,
    seen: (String, f64),
    started: bool,
    key_listener: Option<JsValue>,
    frames: Vec<JsValue>,
}

thread_local! {
    static FEED: RefCell<Feed> = RefCell::new(Feed::default());
}

/// Runs `f` on the state unless it is already borrowed (a re-entrant event): then skips.
fn with<R: Default>(f: impl FnOnce(&mut Feed) -> R) -> R {
    FEED.with(|c| {
        c.try_borrow_mut()
            .map(|mut s| f(&mut s))
            .unwrap_or_default()
    })
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}
fn text(v: &JsValue) -> String {
    if truthy(v) {
        utf16_string(v)
    } else {
        String::new()
    }
}
fn has_class(el: &JsValue, k: &str) -> bool {
    call(&get(el, "classList"), "contains", &[k.into()]).is_ok_and(|v| truthy(&v))
}
fn closest(el: &JsValue, selector: &str) -> JsValue {
    if !truthy(el) {
        return JsValue::NULL;
    }
    call(el, "closest", &[selector.into()]).unwrap_or(JsValue::NULL)
}
fn in_app() -> bool {
    invoke(&global("inApp"), &[]).is_ok_and(|v| truthy(&v))
}
fn remote() -> bool {
    truthy(&global("WEBTERM"))
}
fn visible() -> bool {
    get(&doc(), "visibilityState").as_string().as_deref() != Some("hidden")
}
fn day_start() -> f64 {
    let d = js_sys::Date::new_0();
    d.set_hours(0);
    d.set_minutes(0);
    d.set_seconds(0);
    d.set_milliseconds(0);
    d.get_time() / 1000.0
}
fn sleep(ms: f64) -> JsValue {
    js_sys::Promise::new(&mut |resolve, _| {
        later(resolve.into(), ms);
    })
    .into()
}
fn post(message: serde_json::Value) {
    let handler = get(&get(&global("webkit"), "messageHandlers"), "centro");
    if truthy(&handler) {
        let _ = call(
            &handler,
            "postMessage",
            &[utf16_value(&message.to_string())],
        );
    }
}
fn entry(session: &str) -> Option<view::Entry> {
    with(|s| {
        s.data
            .as_ref()
            .and_then(|d| d.sessions.iter().find(|e| e.session == session).cloned())
    })
}
fn feed_now() -> f64 {
    with(|s| s.data.as_ref().map(|d| d.now).unwrap_or_default())
}

/// Publishes `window.ComandosFeed` and starts polling once the page is a real document.
pub fn install() -> Result<(), JsValue> {
    let api = object();
    method(&api, "hover", |a| {
        set_hover(&text(&a.get(0)), false);
        Ok(JsValue::UNDEFINED)
    })?;
    set(&js_sys::global(), "ComandosFeed", &api)?;
    // Only a browser document with fetch polls (not the node contract tests).
    if !global("fetch").is_function() || get(&doc(), "visibilityState").as_string().is_none() {
        return Ok(());
    }
    if get(&doc(), "readyState").as_string().as_deref() == Some("loading") {
        listen(&doc(), "DOMContentLoaded", function(|_| start()));
    } else {
        later(function(|_| start()), 0.0);
    }
    Ok(())
}

fn start() -> Result<JsValue, JsValue> {
    if with(|s| std::mem::replace(&mut s.started, true)) {
        return Ok(JsValue::UNDEFINED);
    }
    if let Ok(saved) = call(
        &get(&js_sys::global(), "localStorage"),
        "getItem",
        &[CHIP_KEY.into()],
    ) {
        let chip = view::Chip::parse(&text(&saved));
        with(|s| s.chip = chip);
    }
    call(
        &js_sys::global(),
        "setInterval",
        &[function(|_| tick()), POLL_MS.into()],
    )?;
    listen(
        &doc(),
        "visibilitychange",
        function(|_| {
            if visible() {
                with(|s| s.until = 0.0);
                tick()?;
            } else {
                hide_peek();
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
    let keys = function(|a| on_key(&a.get(0)));
    with(|s| s.key_listener = Some(keys.clone()));
    call(
        &js_sys::global(),
        "addEventListener",
        &["keydown".into(), keys, true.into()],
    )?;
    call(
        &doc(),
        "addEventListener",
        &[
            "pointerdown".into(),
            function(|a| {
                let target = get(&a.get(0), "target");
                let outside = with(|s| {
                    s.peek.as_ref().is_some_and(|p| {
                        p.touch
                            && !call(&p.el, "contains", std::slice::from_ref(&target))
                                .is_ok_and(|v| truthy(&v))
                    })
                });
                if outside {
                    hide_peek();
                }
                Ok(JsValue::UNDEFINED)
            }),
            true.into(),
        ],
    )?;
    tick()
}

fn tick() -> Result<JsValue, JsValue> {
    bind_bar();
    bind_feed();
    if !visible() || has_class(&get(&doc(), "body"), "only-panel") {
        return Ok(JsValue::UNDEFINED);
    }
    let go = with(|s| {
        if s.busy || now_ms() < s.until {
            false
        } else {
            s.busy = true;
            true
        }
    });
    if !go {
        return Ok(JsValue::UNDEFINED);
    }
    spawn_local(async {
        let result = request("GET", "/session-feed", JsValue::UNDEFINED).await;
        let status = result
            .as_ref()
            .map(|r| number(&get(r, "status")))
            .unwrap_or(0.0);
        let parsed = match &result {
            Ok(r) if status == 200.0 => view::parse(&to_utf16_json(&get(r, "body"))),
            _ => None,
        };
        // No route on this server (404 until it ships) or a page that is not the feed.
        // Anything else (offline, 401, 5xx) is transient: keep what is shown.
        let absent = [200.0, 404.0, 405.0, 501.0].contains(&status);
        let changed = with(|s| {
            s.busy = false;
            match parsed {
                Some(feed) => {
                    s.data = Some(feed);
                    s.until = 0.0;
                    true
                }
                None => {
                    s.until = now_ms() + if absent { ABSENT_MS } else { OFFLINE_MS };
                    absent && s.data.take().is_some()
                }
            }
        });
        if changed {
            seen_visible();
            paint();
        }
    });
    Ok(JsValue::UNDEFINED)
}

/// Everything the feed draws, after new data or a local change.
fn paint() {
    let on = with(|s| s.data.is_some());
    let root = get(&doc(), "documentElement");
    if on {
        attr(&root, "data-feed", "1");
    } else {
        let _ = call(&root, "removeAttribute", &["data-feed".into()]);
    }
    for tile in all(&doc(), "#tabbar .apptab") {
        decorate(&tile, &get(&tile, "_session"));
    }
    paint_feed();
    paint_tab();
    refresh_peek();
}

/// K5 classes and mark of one top-bar tile; `make_tab` calls it after each render too.
pub fn decorate(tile: &JsValue, session: &JsValue) {
    let session = text(session);
    let (on, cls, hl) = with(|s| {
        let e = s
            .data
            .as_ref()
            .and_then(|d| d.sessions.iter().find(|e| e.session == session));
        (
            s.data.is_some(),
            e.map(|e| view::mark_class(view::mark(e))).unwrap_or(""),
            !s.hover.is_empty() && s.hover == session,
        )
    });
    for c in view::TILE_CLASSES {
        classes(tile, c, c == cls);
    }
    classes(tile, "fs-hl", hl);
    let mark = query(tile, ".fs-mk");
    if !on {
        if truthy(&mark) {
            let _ = call(&mark, "remove", &[]);
        }
        return;
    }
    if !truthy(&mark)
        && let Ok(el) = call(&doc(), "createElement", &["i".into()])
    {
        set_class(&el, "fs-mk");
        attr(&el, "aria-hidden", "true");
        let close = query(tile, ".x");
        let _ = if truthy(&close) {
            call(tile, "insertBefore", &[el, close])
        } else {
            call(tile, "appendChild", &[el])
        };
    }
}
fn set_class(el: &JsValue, name: &str) {
    let _ = set(el, "className", &name.into());
}

fn active_session() -> String {
    if in_app() {
        text(&get(&global("ACTIVE_TAB"), "session"))
    } else {
        text(&global("activeTerm"))
    }
}

fn paint_feed() {
    let el = query(&doc(), "#command-sidebar .cs-feed");
    if !truthy(&el) {
        return;
    }
    let active = active_session();
    let html = with(|s| {
        s.data.as_ref().map(|d| {
            view::feed_html(
                d,
                &view::View {
                    chip: s.chip,
                    day_start: day_start(),
                    en: english(),
                    hl: &s.hover,
                    active: &active,
                },
            )
        })
    })
    .unwrap_or_default();
    if text(&get(&el, "__fsHtml")) != html {
        let scroll = query(&el, ".fs-scroll");
        let top = get(&scroll, "scrollTop");
        let _ = set(&el, "innerHTML", &utf16_value(&html));
        let _ = set(&el, "__fsHtml", &utf16_value(&html));
        let scroll = query(&el, ".fs-scroll");
        if truthy(&scroll) && truthy(&top) {
            let _ = set(&scroll, "scrollTop", &top);
        }
    }
    bind_feed();
}

fn paint_tab() {
    let button = query(&doc(), "#command-sidebar [data-sidebar-view=\"sessions\"]");
    if !truthy(&button) {
        return;
    }
    let (on, waiting) = with(|s| {
        (
            s.data.is_some(),
            s.data
                .as_ref()
                .map(|d| view::counts(&d.sessions).waiting)
                .unwrap_or(0),
        )
    });
    let _ = set(&button, "hidden", &(!on).into());
    let label = query(&button, ".lb");
    if truthy(&label) {
        let _ = set(&label, "textContent", &t("Sesiones", "Sessions").into());
    }
    let badge = query(&button, ".fs-n");
    if truthy(&badge) {
        let _ = set(&badge, "textContent", &waiting.to_string().into());
        let _ = set(&badge, "hidden", &(waiting == 0).into());
        attr(
            &badge,
            "aria-label",
            &t(
                &format!("Sesiones que te esperan: {waiting}"),
                &format!("Sessions waiting for you: {waiting}"),
            ),
        );
    }
}

/// Linked hover: a feed row and its top-bar tile light up together.
fn set_hover(session: &str, from_row: bool) {
    let changed = with(|s| {
        if s.hover == session {
            false
        } else {
            s.hover = session.to_string();
            true
        }
    });
    if !changed {
        return;
    }
    for row in all(&doc(), "#command-sidebar .cs-feed .fs-row") {
        let on = !session.is_empty() && text(&get(&get(&row, "dataset"), "session")) == session;
        classes(&row, "hl", on);
    }
    for tile in all(&doc(), "#tabbar .apptab") {
        classes(
            &tile,
            "fs-hl",
            !session.is_empty() && text(&get(&tile, "_session")) == session,
        );
    }
    if from_row && in_app() {
        post(json!({ "feedHover": session }));
    }
}

fn row_session(el: &JsValue) -> String {
    let row = closest(el, ".cs-feed .fs-row");
    text(&get(&get(&row, "dataset"), "session"))
}

fn bind_feed() {
    let el = query(&doc(), "#command-sidebar .cs-feed");
    if !truthy(&el) || truthy(&get(&el, "__fsBound")) {
        return;
    }
    let _ = set(&el, "__fsBound", &true.into());
    listen(
        &el,
        "click",
        function(|a| {
            let e = a.get(0);
            let target = get(&e, "target");
            let chip = closest(&target, "[data-chip]");
            if truthy(&chip) {
                let chip = view::Chip::parse(&text(&get(&get(&chip, "dataset"), "chip")));
                with(|s| s.chip = chip);
                let _ = call(
                    &get(&js_sys::global(), "localStorage"),
                    "setItem",
                    &[CHIP_KEY.into(), chip.key().into()],
                );
                paint_feed();
                return Ok(JsValue::UNDEFINED);
            }
            let session = row_session(&target);
            if !session.is_empty() {
                open(&session);
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
    listen(
        &el,
        "keydown",
        function(|a| {
            let e = a.get(0);
            let key = text(&get(&e, "key"));
            let target = get(&e, "target");
            if (key == "Enter" || key == " ") && has_class(&target, "fs-row") {
                stop(&e);
                open(&row_session(&target));
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
    listen(
        &el,
        "mouseover",
        function(|a| {
            let e = a.get(0);
            let to = row_session(&get(&e, "target"));
            if to != row_session(&get(&e, "relatedTarget")) && !to.is_empty() {
                set_hover(&to, true);
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
    listen(
        &el,
        "mouseout",
        function(|a| {
            let e = a.get(0);
            let from = row_session(&get(&e, "target"));
            let to = row_session(&get(&e, "relatedTarget"));
            if from != to && to.is_empty() {
                set_hover("", true);
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
}

fn tile_of(el: &JsValue) -> JsValue {
    closest(el, "#tabbar .apptab")
}

fn bind_bar() {
    let bar = id("tabbar");
    if !truthy(&bar) || truthy(&get(&bar, "__fsBound")) {
        return;
    }
    let _ = set(&bar, "__fsBound", &true.into());
    listen(
        &bar,
        "pointerover",
        function(|a| {
            let e = a.get(0);
            if text(&get(&e, "pointerType")) != "mouse" {
                return Ok(JsValue::UNDEFINED);
            }
            let tile = tile_of(&get(&e, "target"));
            if !truthy(&tile) || tile_of(&get(&e, "relatedTarget")) == tile {
                return Ok(JsValue::UNDEFINED);
            }
            let session = text(&get(&tile, "_session"));
            set_hover(&session, false);
            schedule_peek(tile, session);
            Ok(JsValue::UNDEFINED)
        }),
    );
    listen(
        &bar,
        "pointerout",
        function(|a| {
            let e = a.get(0);
            if text(&get(&e, "pointerType")) != "mouse" {
                return Ok(JsValue::UNDEFINED);
            }
            let tile = tile_of(&get(&e, "target"));
            if !truthy(&tile) || tile_of(&get(&e, "relatedTarget")) == tile {
                return Ok(JsValue::UNDEFINED);
            }
            set_hover("", false);
            cancel_timer(|s| &mut s.peek_timer);
            let open = with(|s| s.peek.as_ref().is_some_and(|p| !p.touch));
            if open {
                let timer = later(
                    function(|_| {
                        with(|s| s.hide_timer = None);
                        hide_peek();
                        Ok(JsValue::UNDEFINED)
                    }),
                    PEEK_GRACE_MS,
                );
                cancel_timer(|s| &mut s.hide_timer);
                with(|s| s.hide_timer = Some(timer));
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
    listen(
        &bar,
        "pointerdown",
        function(|a| {
            let e = a.get(0);
            if text(&get(&e, "pointerType")) == "mouse" {
                return Ok(JsValue::UNDEFINED);
            }
            let tile = tile_of(&get(&e, "target"));
            cancel_timer(|s| &mut s.press_timer);
            if !truthy(&tile) || !with(|s| s.data.is_some()) {
                return Ok(JsValue::UNDEFINED);
            }
            let session = text(&get(&tile, "_session"));
            let at = (number(&get(&e, "clientX")), number(&get(&e, "clientY")));
            let timer = later(
                function(move |_| {
                    with(|s| {
                        s.press_timer = None;
                        s.press = None;
                        s.swallow_click = true;
                    });
                    show_peek(&tile, &session, true);
                    Ok(JsValue::UNDEFINED)
                }),
                LONG_PRESS_MS,
            );
            with(|s| {
                s.press_timer = Some(timer);
                s.press = Some(at);
                s.swallow_click = false;
            });
            Ok(JsValue::UNDEFINED)
        }),
    );
    listen(
        &bar,
        "pointermove",
        function(|a| {
            let e = a.get(0);
            let (x, y) = (number(&get(&e, "clientX")), number(&get(&e, "clientY")));
            let moved = with(|s| {
                s.press
                    .is_some_and(|(px, py)| (x - px).abs() > 10.0 || (y - py).abs() > 10.0)
            });
            if moved {
                cancel_timer(|s| &mut s.press_timer);
                with(|s| s.press = None);
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
    for event in ["pointerup", "pointercancel"] {
        listen(
            &bar,
            event,
            function(|_| {
                cancel_timer(|s| &mut s.press_timer);
                with(|s| s.press = None);
                Ok(JsValue::UNDEFINED)
            }),
        );
    }
    // The click that ends a long press must not open the session under it.
    let _ = call(
        &bar,
        "addEventListener",
        &[
            "click".into(),
            function(|a| {
                if with(|s| std::mem::take(&mut s.swallow_click)) {
                    stop(&a.get(0));
                }
                Ok(JsValue::UNDEFINED)
            }),
            true.into(),
        ],
    );
    listen(
        &bar,
        "contextmenu",
        function(|a| {
            let e = a.get(0);
            if with(|s| s.press.is_some() || s.swallow_click || s.peek.is_some()) {
                let _ = call(&e, "preventDefault", &[]);
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
}

fn cancel_timer(pick: impl FnOnce(&mut Feed) -> &mut Option<JsValue>) {
    if let Some(timer) = with(|s| pick(s).take()) {
        cancel(timer);
    }
}

fn schedule_peek(tile: JsValue, session: String) {
    cancel_timer(|s| &mut s.peek_timer);
    let same = with(|s| {
        s.peek
            .as_ref()
            .is_some_and(|p| !p.touch && p.session == session)
    });
    if same {
        // Back on the tile whose card is still up within the grace period.
        cancel_timer(|s| &mut s.hide_timer);
        return;
    }
    if !with(|s| s.data.is_some()) {
        return;
    }
    let timer = later(
        function(move |_| {
            with(|s| s.peek_timer = None);
            show_peek(&tile, &session, false);
            Ok(JsValue::UNDEFINED)
        }),
        PEEK_DELAY_MS,
    );
    with(|s| s.peek_timer = Some(timer));
}

fn show_peek(tile: &JsValue, session: &str, touch: bool) {
    let Some(e) = entry(session) else {
        hide_peek();
        return;
    };
    let el = with(|s| s.peek.as_ref().map(|p| p.el.clone())).unwrap_or_else(|| {
        let el = call(&doc(), "createElement", &["div".into()]).unwrap_or(JsValue::NULL);
        if truthy(&el) {
            let _ = call(
                &get(&doc(), "body"),
                "appendChild",
                std::slice::from_ref(&el),
            );
            bind_peek(&el);
        }
        el
    });
    if !truthy(&el) {
        return;
    }
    set_class(&el, if touch { "fs-peek touch" } else { "fs-peek" });
    attr(&el, "role", if touch { "dialog" } else { "tooltip" });
    let html = view::card_html(&e, feed_now(), english(), touch);
    let _ = set(&el, "innerHTML", &utf16_value(&html));
    let _ = set(&el, "__fsHtml", &utf16_value(&html));
    place(&el, tile, touch);
    cancel_timer(|s| &mut s.hide_timer);
    let previous = with(|s| {
        s.peek.replace(Peek {
            session: session.to_string(),
            touch,
            el: el.clone(),
        })
    });
    if previous.is_none() && !touch {
        watch_frames();
    }
}

fn place(el: &JsValue, tile: &JsValue, touch: bool) {
    let width = number(&get(&js_sys::global(), "innerWidth"));
    let put = |k: &str, v: String| style(el, k, &v);
    if touch {
        let bar = id("app-navigation");
        let anchor = if truthy(&bar) { bar } else { tile.clone() };
        let rect = call(&anchor, "getBoundingClientRect", &[]).unwrap_or(JsValue::NULL);
        put("top", format!("{}px", number(&get(&rect, "bottom")) + 6.0));
        put("left", "8px".into());
        put("width", format!("{}px", (width - 16.0).max(0.0)));
        return;
    }
    let rect = call(tile, "getBoundingClientRect", &[]).unwrap_or(JsValue::NULL);
    let w = 320f64.min(width - 16.0).max(0.0);
    let mut left = number(&get(&rect, "left"));
    if left + w > width - 8.0 {
        left = number(&get(&rect, "right")) - w;
    }
    put("top", format!("{}px", number(&get(&rect, "bottom")) + 6.0));
    put("left", format!("{}px", left.max(8.0)));
    put("width", format!("{w}px"));
}

fn bind_peek(el: &JsValue) {
    listen(
        el,
        "click",
        function(|a| {
            let e = a.get(0);
            let Some((session, touch)) =
                with(|s| s.peek.as_ref().map(|p| (p.session.clone(), p.touch)))
            else {
                return Ok(JsValue::UNDEFINED);
            };
            if !touch {
                return Ok(JsValue::UNDEFINED);
            }
            stop(&e);
            let option = closest(&get(&e, "target"), ".fs-opt[data-key]");
            hide_peek();
            if truthy(&option) {
                choose(&session, &text(&get(&get(&option, "dataset"), "key")));
            } else {
                open(&session);
            }
            Ok(JsValue::UNDEFINED)
        }),
    );
}

fn refresh_peek() {
    let Some((session, touch, el)) = with(|s| {
        s.peek
            .as_ref()
            .map(|p| (p.session.clone(), p.touch, p.el.clone()))
    }) else {
        return;
    };
    let Some(e) = entry(&session) else {
        hide_peek();
        return;
    };
    let html = view::card_html(&e, feed_now(), english(), touch);
    if text(&get(&el, "__fsHtml")) != html {
        let _ = set(&el, "innerHTML", &utf16_value(&html));
        let _ = set(&el, "__fsHtml", &utf16_value(&html));
    }
}

fn hide_peek() {
    cancel_timer(|s| &mut s.peek_timer);
    cancel_timer(|s| &mut s.hide_timer);
    let Some(peek) = with(|s| s.peek.take()) else {
        return;
    };
    let _ = call(&peek.el, "remove", &[]);
    unwatch_frames();
}

/// Terminal frames are same-origin documents; while the card is shown, keys typed with a
/// terminal focused must reach the card too.
fn watch_frames() {
    let Some(listener) = with(|s| s.key_listener.clone()) else {
        return;
    };
    let mut watched = Vec::new();
    for frame in all(&doc(), "iframe") {
        let win = get(&frame, "contentWindow");
        if truthy(&win)
            && call(
                &win,
                "addEventListener",
                &["keydown".into(), listener.clone(), true.into()],
            )
            .is_ok()
        {
            watched.push(win);
        }
    }
    with(|s| s.frames = watched);
}
fn unwatch_frames() {
    let (listener, frames) = with(|s| (s.key_listener.clone(), std::mem::take(&mut s.frames)));
    let Some(listener) = listener else {
        return;
    };
    for win in frames {
        let _ = call(
            &win,
            "removeEventListener",
            &["keydown".into(), listener.clone(), true.into()],
        );
    }
}

/// With the card shown over a tile that waits on a menu, 1–9 answer it. No other key.
fn on_key(e: &JsValue) -> Result<JsValue, JsValue> {
    let Some(session) = with(|s| {
        s.peek
            .as_ref()
            .filter(|p| !p.touch)
            .map(|p| p.session.clone())
    }) else {
        return Ok(JsValue::UNDEFINED);
    };
    if ["ctrlKey", "metaKey", "altKey", "isComposing"]
        .iter()
        .any(|k| truthy(&get(e, k)))
    {
        return Ok(JsValue::UNDEFINED);
    }
    let key = text(&get(e, "key"));
    let Some(entry) = entry(&session) else {
        return Ok(JsValue::UNDEFINED);
    };
    if !view::digits(&entry).contains(&key.as_str()) {
        return Ok(JsValue::UNDEFINED);
    }
    stop(e);
    let _ = call(e, "stopImmediatePropagation", &[]);
    hide_peek();
    choose(&session, &key);
    Ok(JsValue::UNDEFINED)
}

/// Same as a notification's answer: `POST /key` with the digit, `Enter` 250 ms later.
fn choose(session: &str, digit: &str) {
    let Some(e) = entry(session) else {
        return;
    };
    let session = e.session.clone();
    let pane = e.pane.clone();
    let digit = digit.to_string();
    spawn_local(async move {
        let send = |key: &str| {
            from_utf16_json(&json!({"session": session, "pane": pane, "key": key}))
                .unwrap_or(JsValue::NULL)
        };
        let ok =
            |r: &Result<JsValue, JsValue>| matches!(r, Ok(r) if number(&get(r, "status")) < 300.0);
        let first = request("POST", "/key", send(&digit)).await;
        let sent = ok(&first) && {
            let _ = wait(Ok(sleep(CHOOSE_DELAY_MS))).await;
            ok(&request("POST", "/key", send("Enter")).await)
        };
        if !sent {
            toast(&t("No se pudo enviar la tecla", "Could not send the key"));
        }
    });
}

fn valid_pane(pane: &str) -> bool {
    pane.strip_prefix('%')
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// Opens a session from the feed or a preview card, and selects its pane.
fn open(session: &str) {
    if session.is_empty() {
        return;
    }
    let e = entry(session).unwrap_or_default();
    let label = if e.label.is_empty() {
        session.to_string()
    } else {
        e.label.clone()
    };
    let pane = if valid_pane(&e.pane) {
        e.pane.clone()
    } else {
        String::new()
    };
    let sess = utf16_value(session);
    if in_app() {
        // The desktop app opens the tab (`BridgeMsg::OpenSession`) and the server selects
        // the pane, exactly like opening a notice's session.
        let _ = invoke(
            &global("openInApp"),
            &[sess.clone(), "claude".into(), utf16_value(&label)],
        );
        if !pane.is_empty() {
            spawn_local(async move {
                let api = |body: serde_json::Value| {
                    invoke(
                        &global("api"),
                        &[
                            "/terminal-panes".into(),
                            from_utf16_json(&body).unwrap_or(JsValue::NULL),
                        ],
                    )
                };
                let session = utf16_string(&sess);
                let Ok(list) = wait(api(json!({"session": session, "action": "list"}))).await
                else {
                    return;
                };
                let panes = get(&list, "panes");
                let hit = if js_sys::Array::is_array(&panes) {
                    js_sys::Array::from(&panes)
                        .iter()
                        .find(|p| text(&get(p, "id")) == pane)
                } else {
                    None
                };
                if let Some(hit) = hit {
                    let body = json!({"session": session, "action": "select", "pane": pane,
                        "identity": to_utf16_json(&get(&hit, "identity"))});
                    let _ = wait(api(body)).await;
                }
            });
        }
        return;
    }
    if remote() {
        let open = call(&global("openTerms"), "has", std::slice::from_ref(&sess))
            .is_ok_and(|v| truthy(&v));
        let _ = if open {
            invoke(
                &global("showView"),
                &[utf16_value(&format!("term:{session}")), true.into()],
            )
        } else {
            invoke(&global("openTerm"), &[sess.clone(), utf16_value(&label)])
        };
        if !pane.is_empty() {
            let _ = invoke(&global("selectPaneInFrame"), &[sess, utf16_value(&pane)]);
        }
        return;
    }
    let body = from_utf16_json(&json!({ "session": session })).unwrap_or(JsValue::NULL);
    if let Ok(p) = invoke(&global("api"), &["/focus".into(), body]) {
        spawn_local(async move {
            let _ = wait(Ok(p)).await;
        });
    }
}

/// The session the remote shows right now (none while the panel covers the terminal).
fn visible_session() -> String {
    if in_app() || !remote() || !visible() || !has_class(&id("term-area"), "on") {
        return String::new();
    }
    text(&global("activeTerm"))
}

/// `show_view` calls it: opening a session's view in the remote marks it seen.
pub fn viewed() {
    let session = visible_session();
    if !session.is_empty() {
        post_seen(&session, false);
    }
}

fn seen_visible() {
    let session = visible_session();
    if !session.is_empty() {
        post_seen(&session, true);
    }
}

fn post_seen(session: &str, only_if_new: bool) {
    let go = with(|s| {
        let Some(d) = s.data.as_mut() else {
            return false;
        };
        let now = d.now;
        let Some(e) = d.sessions.iter_mut().find(|e| e.session == session) else {
            return false;
        };
        if only_if_new && !view::needs_seen(e) {
            return false;
        }
        let at = now_ms();
        if s.seen.0 == session && at - s.seen.1 < SEEN_THROTTLE_MS {
            return false;
        }
        view::mark_seen(e, now);
        s.seen = (session.to_string(), at);
        true
    });
    if !go {
        return;
    }
    let body = from_utf16_json(&json!({ "session": session })).unwrap_or(JsValue::NULL);
    spawn_local(async move {
        let _ = request("POST", "/session-seen", body).await;
    });
    paint();
}
