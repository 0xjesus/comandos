//! Ficha viva on the desktop (design/ficha-viva-spec.md §2 and §4): K5 tiles from
//! `/session-feed`, the hover preview with 1–9 answers, the hover link with the
//! web feed and the «seen» marks.
use super::*;
use crate::feed::{self, Feed, TileLook};
use webkit2gtk::WebViewExt;

#[derive(Default)]
pub(super) struct Owned {
    pub(super) feed: RefCell<Option<Feed>>,
    preview: RefCell<Option<ui::feed_preview::FeedPreview>>,
    /// Session whose card is on screen.
    preview_key: RefCell<Option<String>>,
    /// Tile under the pointer.
    hover_key: RefCell<Option<String>>,
    show_timer: RefCell<Option<glib::SourceId>>,
    hide_timer: RefCell<Option<glib::SourceId>>,
    ring_tick: RefCell<Option<glib::SourceId>>,
    /// Last value handed to `ComandosFeed.hover`, to call it only on changes.
    web_hover: RefCell<String>,
    /// Tile lit by the web feed (`k5-hl`).
    highlight: RefCell<String>,
    /// Open tab at the last check, to detect switches.
    current: RefCell<Option<String>>,
    /// (session, ts) already marked from a poll; avoids repeating in-flight posts.
    seen_sent: RefCell<Option<(String, f64)>>,
}

fn cancel(cell: &RefCell<Option<glib::SourceId>>) {
    if let Some(id) = cell.borrow_mut().take() {
        id.remove();
    }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |d| d.as_secs_f64())
}

impl App {
    pub(super) fn install_feed(self: &Rc<Self>) {
        *self.feed.preview.borrow_mut() = Some(ui::feed_preview::FeedPreview::new(
            &self.window,
            self.english,
        ));
        // Window level, before the focused terminal or web view sees the key.
        let weak = Rc::downgrade(self);
        let signal = self.window.connect_key_press_event(move |_, event| {
            if weak.upgrade().is_some_and(|app| app.feed_answer_key(event)) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.protocol_signals
            .borrow_mut()
            .push((self.window.clone().upcast(), signal));
        let weak = Rc::downgrade(self);
        let signal = self.window.connect_is_active_notify(move |window| {
            let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                return;
            };
            if !window.is_active() {
                app.feed_hide_preview();
            }
            app.paint_feed_tiles();
            app.feed_seen_on_poll();
        });
        self.protocol_signals
            .borrow_mut()
            .push((self.window.clone().upcast(), signal));
    }

    pub(super) fn shutdown_feed(&self) {
        cancel(&self.feed.show_timer);
        cancel(&self.feed.hide_timer);
        cancel(&self.feed.ring_tick);
        if let Some(preview) = self.feed.preview.borrow_mut().take() {
            preview.close();
        }
    }

    /// Pointer and click hooks of one session tile.
    pub(super) fn attach_feed_hover(self: &Rc<Self>, key: &str, tab: &ui::tab_label::TabLabel) {
        let weak = Rc::downgrade(self);
        let k = key.to_string();
        tab.item.connect_enter_notify_event(move |_, _| {
            if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                app.feed_pointer_enter(&k);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let k = key.to_string();
        tab.item.connect_leave_notify_event(move |_, event| {
            if event.detail() != gdk::NotifyType::Inferior
                && let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire))
            {
                app.feed_pointer_leave(&k);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        tab.item.connect_button_press_event(move |_, _| {
            if let Some(app) = weak.upgrade() {
                cancel(&app.feed.show_timer);
                app.feed_hide_preview();
            }
            glib::Propagation::Proceed
        });
        tab.set_look(feed::tile_look(
            self.feed.feed.borrow().as_ref(),
            key,
            false,
        ));
    }

    fn feed_pointer_enter(self: &Rc<Self>, key: &str) {
        cancel(&self.feed.hide_timer);
        let same = self.feed.hover_key.borrow().as_deref() == Some(key);
        *self.feed.hover_key.borrow_mut() = Some(key.to_string());
        self.feed_web_hover(key);
        if same && (self.feed_preview_visible() || self.feed.show_timer.borrow().is_some()) {
            return;
        }
        cancel(&self.feed.show_timer);
        if self.drag_layer.active() {
            return;
        }
        if self.feed_preview_visible() {
            // Sliding along the bar keeps the card open on the next tile.
            self.feed_show_preview(key);
            return;
        }
        let weak = Rc::downgrade(self);
        let k = key.to_string();
        *self.feed.show_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(feed::PREVIEW_DELAY_MS),
            move || {
                if let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) {
                    app.feed.show_timer.borrow_mut().take();
                    if app.feed.hover_key.borrow().as_deref() == Some(k.as_str()) {
                        app.feed_show_preview(&k);
                    }
                }
            },
        ));
    }

    fn feed_pointer_leave(self: &Rc<Self>, key: &str) {
        if self.feed.hover_key.borrow().as_deref() != Some(key) {
            return;
        }
        self.feed.hover_key.borrow_mut().take();
        cancel(&self.feed.show_timer);
        self.feed_web_hover("");
        cancel(&self.feed.hide_timer);
        if !self.feed_preview_visible() {
            return;
        }
        let weak = Rc::downgrade(self);
        *self.feed.hide_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(feed::PREVIEW_GRACE_MS),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.feed.hide_timer.borrow_mut().take();
                    if app.feed.hover_key.borrow().is_none() {
                        app.feed_hide_preview();
                    }
                }
            },
        ));
    }

    fn feed_preview_visible(&self) -> bool {
        self.feed
            .preview
            .borrow()
            .as_ref()
            .is_some_and(|p| p.is_visible())
    }

    fn feed_show_preview(&self, key: &str) {
        if !self.window.is_active() || self.drag_layer.active() {
            return;
        }
        let tab = self.labels.borrow().get(key).cloned();
        let feed = self.feed.feed.borrow();
        let (Some(tab), Some(entry)) = (tab, feed.as_ref().and_then(|f| f.sessions.get(key)))
        else {
            drop(feed);
            self.feed_hide_preview();
            return;
        };
        if !tab.item.is_mapped() {
            return;
        }
        let name = tab.text.text().to_string();
        let name = if name.is_empty() {
            entry.label.clone()
        } else {
            name
        };
        if let Some(preview) = self.feed.preview.borrow().as_ref() {
            preview.render(&name, entry, unix_now(), self.english);
            preview.show_under(tab.item.upcast_ref(), &self.window);
            *self.feed.preview_key.borrow_mut() = Some(key.to_string());
        }
    }

    pub(super) fn feed_hide_preview(&self) {
        cancel(&self.feed.hide_timer);
        self.feed.preview_key.borrow_mut().take();
        if let Some(preview) = self.feed.preview.borrow().as_ref() {
            preview.hide();
        }
    }

    /// `window.ComandosFeed.hover(session)` in the dashboard, without revealing it.
    fn feed_web_hover(&self, session: &str) {
        if *self.feed.web_hover.borrow() == session {
            return;
        }
        *self.feed.web_hover.borrow_mut() = session.to_string();
        self.webview.evaluate_javascript(
            &feed::hover_script(session),
            None,
            None,
            Some(&self.protocol_cancellable),
            |_| {},
        );
    }

    /// Bridge `{"feedHover": "<session>" | ""}` from the web feed.
    pub(super) fn feed_highlight(&self, session: &str) {
        *self.feed.highlight.borrow_mut() = session.to_string();
        for (key, tab) in self.labels.borrow().iter() {
            tab.set_highlight(!session.is_empty() && key == session);
        }
    }

    pub(super) fn apply_feed(self: &Rc<Self>, value: &Value) {
        let parsed = feed::parse_feed(value);
        *self.feed.feed.borrow_mut() = parsed;
        self.paint_feed_tiles();
        let shown = self.feed.preview_key.borrow().clone();
        if let Some(key) = shown
            && self.feed_preview_visible()
        {
            self.feed_show_preview(&key);
        }
        self.feed_seen_on_poll();
        self.focus_refresh();
    }

    fn feed_current_seen(&self, key: &str) -> bool {
        self.window.is_active() && self.feed.current.borrow().as_deref() == Some(key)
    }

    pub(super) fn paint_feed_tiles(self: &Rc<Self>) {
        let feed = self.feed.feed.borrow();
        let animate = ui::header::animations_enabled();
        let mut spinning = false;
        for (key, tab) in self.labels.borrow().iter() {
            let look: TileLook = feed::tile_look(feed.as_ref(), key, self.feed_current_seen(key));
            tab.set_look(look);
            if look.marker == feed::Marker::Ring {
                spinning = true;
                if !animate {
                    tab.spin(0.);
                }
            }
        }
        drop(feed);
        if spinning && animate {
            self.feed_start_ring();
        } else {
            cancel(&self.feed.ring_tick);
        }
    }

    /// 25 fps redraw of the 9 px rings; runs only while some tile works.
    fn feed_start_ring(self: &Rc<Self>) {
        if self.feed.ring_tick.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let start = glib::monotonic_time();
        *self.feed.ring_tick.borrow_mut() = Some(glib::timeout_add_local(
            Duration::from_millis(40),
            move || {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return glib::ControlFlow::Break;
                };
                let animate = ui::header::animations_enabled();
                let seconds = (glib::monotonic_time() - start) as f64 / 1_000_000.;
                let angle = feed::ring_angle(seconds, animate);
                let mut any = false;
                if app.presence_visible.get() || !animate {
                    for tab in app.labels.borrow().values() {
                        any |= tab.spin(angle);
                    }
                } else {
                    any = app
                        .labels
                        .borrow()
                        .values()
                        .any(|t| t.look().marker == feed::Marker::Ring);
                }
                if any && animate {
                    glib::ControlFlow::Continue
                } else {
                    app.feed.ring_tick.borrow_mut().take();
                    glib::ControlFlow::Break
                }
            },
        ));
    }

    /// Called by the 80 ms UI tick with the open tab: a switch hides the card
    /// and marks the newly opened session as seen.
    pub(super) fn feed_note_current(self: &Rc<Self>, key: Option<&str>) {
        let previous = self.feed.current.borrow().clone();
        if previous.as_deref() == key {
            return;
        }
        *self.feed.current.borrow_mut() = key.map(str::to_string);
        self.feed_hide_preview();
        if let Some(session) =
            feed::seen_on_switch(previous.as_deref(), key, self.window.is_active())
        {
            self.post_session_seen(&session);
        }
        self.paint_feed_tiles();
        self.focus_refresh();
    }

    fn feed_seen_on_poll(&self) {
        let current = self.feed.current.borrow().clone();
        let feed = self.feed.feed.borrow();
        let Some(feed) = feed.as_ref() else {
            return;
        };
        let Some(session) = feed::seen_on_poll(feed, current.as_deref(), self.window.is_active())
        else {
            return;
        };
        let ts = feed.sessions.get(&session).map_or(0., |e| e.ts);
        let mark = (session.clone(), ts);
        if self.feed.seen_sent.borrow().as_ref() == Some(&mark) {
            return;
        }
        *self.feed.seen_sent.borrow_mut() = Some(mark);
        self.post_session_seen(&session);
    }

    /// `POST /session-seen`; an older server without the route is ignored.
    fn post_session_seen(&self, session: &str) {
        if !self.writable() || self.cfg.dash_url().is_none() {
            return;
        }
        let dash = self.dash.clone();
        let body = json!({ "session": session });
        self.jobs.spawn(
            move || dash.post("/session-seen", &body, Duration::from_secs(3)),
            |_| {},
        );
    }

    /// A digit of the visible menu answers that session's pane: the digit,
    /// then Enter 250 ms later (same as the notifications' `choose`).
    fn feed_answer_key(self: &Rc<Self>, event: &gdk::EventKey) -> bool {
        if self.closed.load(Ordering::Acquire) || !self.feed_preview_visible() {
            return false;
        }
        let modifiers = gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::MOD1_MASK
            | gdk::ModifierType::SUPER_MASK
            | gdk::ModifierType::META_MASK;
        if event.state().intersects(modifiers) {
            return false;
        }
        let Some(digit) = event.keyval().to_unicode() else {
            return false;
        };
        let Some(session) = self.feed.preview_key.borrow().clone() else {
            return false;
        };
        let entry = self
            .feed
            .feed
            .borrow()
            .as_ref()
            .and_then(|f| f.sessions.get(&session).cloned());
        let Some(digit) = feed::answer_digit(entry.as_ref(), digit) else {
            return false;
        };
        let Some(entry) = entry else {
            return false;
        };
        if !self.writable() {
            return false;
        }
        cancel(&self.feed.show_timer);
        self.feed_hide_preview();
        let [first, enter] = feed::answer_payloads(&entry, &digit);
        let dash = self.dash.clone();
        let weak = Rc::downgrade(self);
        self.jobs.spawn(
            move || dash.post("/key", &first, Duration::from_secs(3)),
            move |result| {
                let Some(app) = weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire)) else {
                    return;
                };
                match result {
                    Ok((status, _)) if (200..300).contains(&status) => {
                        let weak = Rc::downgrade(&app);
                        glib::timeout_add_local_once(
                            Duration::from_millis(feed::CHOOSE_ENTER_MS),
                            move || {
                                if let Some(app) =
                                    weak.upgrade().filter(|a| !a.closed.load(Ordering::Acquire))
                                {
                                    let dash = app.dash.clone();
                                    app.jobs.spawn(
                                        move || dash.post("/key", &enter, Duration::from_secs(3)),
                                        |_| {},
                                    );
                                }
                            },
                        );
                    }
                    Ok((status, _)) => app.status.set_text(&format!("Responder: HTTP {status}")),
                    Err(error) => app.status.set_text(&format!("Responder: {error:?}")),
                }
            },
        );
        true
    }
}
