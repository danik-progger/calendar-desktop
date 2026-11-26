mod board;
mod connection;
mod launch;
#[cfg(test)]
mod window_tests;

use std::time::Duration as Wait;

use chrono::{Duration, Local, NaiveDate};
use gpui::{prelude::*, *};

use self::board::{Drag, EventMotion};
pub(super) use self::launch::run;
use crate::{
    backend,
    editor::{Editor, EditorAction},
    model::{Event, Store, monday},
    style::{ERROR_COLOR, HOVER, SURFACE, TEXT_STRONG, action_button},
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Day,
    Three,
    Week,
}

impl Mode {
    fn initial(width: Pixels) -> Self {
        if width < px(960.) {
            Self::Day
        } else if width < px(1440.) {
            Self::Three
        } else {
            Self::Week
        }
    }

    fn count(self) -> usize {
        match self {
            Self::Day => 1,
            Self::Three => 3,
            Self::Week => 7,
        }
    }
}

enum UndoNotice {
    Deleted,
    RestoreFailed,
}

struct Desktop {
    config: Option<backend::Config>,
    account: Option<(backend::Api, backend::User)>,
    account_menu: Option<Entity<gpui_component::menu::PopupMenu>>,
    keyboard_focus_visible: bool,
    _keyboard_focus_subscription: Subscription,
    busy: bool,
    network_task: Option<Task<()>>,
    realtime_task: Option<Task<()>>,
    sync_pending: bool,
    save_queue: std::collections::VecDeque<(Event, Option<Entity<Editor>>)>,
    confirmed_events: Option<Vec<Event>>,
    login_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    store: Option<Store>,
    error: Option<&'static str>,
    undo_notice: Option<UndoNotice>,
    now: chrono::DateTime<Local>,
    mode: Mode,
    anchor: NaiveDate,
    editor: Option<Entity<Editor>>,
    scroll: ScrollHandle,
    needs_initial_scroll: bool,
    drag: Option<Drag>,
    event_motion: EventMotion,
    wheel: (f32, Option<std::time::Instant>),
}

impl Drop for Desktop {
    fn drop(&mut self) {
        if let Some(cancel) = &self.login_cancel {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl Desktop {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let now = Local::now();
        let mode = Mode::initial(window.viewport_size().width);
        let window_handle = window.window_handle();
        let desktop = cx.weak_entity();
        let keyboard_focus_subscription = cx.intercept_keystrokes(move |_, window, cx| {
            if window.window_handle() == window_handle {
                _ = desktop.update(cx, |app, cx| app.set_keyboard_focus_visible(true, cx));
            }
        });
        let mut app = Self {
            config: None,
            account: None,
            account_menu: None,
            keyboard_focus_visible: false,
            _keyboard_focus_subscription: keyboard_focus_subscription,
            busy: false,
            network_task: None,
            realtime_task: None,
            sync_pending: false,
            save_queue: std::collections::VecDeque::new(),
            confirmed_events: None,
            login_cancel: None,
            store: None,
            error: None,
            undo_notice: None,
            mode,
            anchor: now.date_naive(),
            now,
            editor: None,
            scroll: ScrollHandle::new(),
            needs_initial_scroll: true,
            drag: None,
            event_motion: EventMotion::default(),
            wheel: (0., None),
        };
        cx.observe_window_bounds(window, |app, window, cx| {
            let width = window.viewport_size().width;
            if width < px(720.) {
                app.set_mode(Mode::Day);
            } else if app.mode == Mode::Day && width >= px(1440.) {
                app.set_mode(Mode::Week);
            }
            cx.notify();
        })
        .detach();
        app.today();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |app, cx| {
            loop {
                executor.timer(Wait::from_secs(30)).await;
                if app
                    .update(cx, |app, cx| {
                        app.now = Local::now();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        app
    }

    fn set_keyboard_focus_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.keyboard_focus_visible != visible {
            self.keyboard_focus_visible = visible;
            cx.notify();
        }
    }

    fn today(&mut self) {
        self.anchor = Local::now().date_naive();
        if self.mode == Mode::Week {
            self.anchor = monday(self.anchor);
        }
    }

    fn shift(&mut self, count: i64) {
        self.anchor += Duration::days(count);
    }

    fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Week && self.mode != Mode::Week {
            self.anchor = monday(self.anchor);
        }
        self.mode = mode;
    }

    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else { return };
        match editor.read(cx).draft(cx) {
            Ok(event) => self.save(event, cx),
            Err(message) => self.error = Some(message),
        }
        cx.notify();
    }

    fn show_editor(
        &mut self,
        event: Event,
        existing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        let editor = cx.new(|cx| Editor::new(event, existing, window, cx));
        cx.subscribe(&editor, |app, _, action, cx| match action {
            EditorAction::Save => app.save_editor(cx),
            EditorAction::Delete => app.delete(cx),
            EditorAction::Cancel => {
                app.editor = None;
                app.error = None;
                cx.notify();
            }
        })
        .detach();
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        self.editor = Some(editor);
        cx.notify();
    }
}

impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desktop = cx.weak_entity();
        let pointer_focus = canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        _ = desktop.update(cx, |app, cx| app.set_keyboard_focus_visible(false, cx));
                    }
                });
            },
        )
        .absolute()
        .size(px(0.));
        if self.account.is_none() {
            self.event_motion = EventMotion::default();
            self.account_menu = None;
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.))
                .bg(rgb(SURFACE))
                .text_color(rgb(TEXT_STRONG))
                .font_family("Mulish")
                .text_size(px(13.))
                .line_height(px(18.))
                .child(pointer_focus)
                .child(
                    div()
                        .w_full()
                        .max_w(px(360.))
                        .p(px(28.))
                        .flex()
                        .flex_col()
                        .text_center()
                        .child(
                            div()
                                .text_size(px(20.))
                                .line_height(px(26.))
                                .font_weight(FontWeight::BOLD)
                                .child("Sign in to Calendar"),
                        )
                        .when(self.config.is_some(), |el| {
                            let button = if self.busy {
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .h(px(42.))
                                    .rounded(px(6.))
                                    .bg(rgb(HOVER))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .opacity(0.5)
                                    .child(if self.login_cancel.is_some() {
                                        "Connecting…"
                                    } else {
                                        "Checking your session…"
                                    })
                                    .into_any_element()
                            } else {
                                action_button(
                                    "google-sign-in",
                                    "Continue with Google",
                                    self.keyboard_focus_visible,
                                    cx.listener(|app, _, _, cx| app.sign_in(cx)),
                                )
                                .h(px(42.))
                                .bg(rgb(HOVER))
                                .text_color(rgb(TEXT_STRONG))
                                .hover(|el| el.bg(rgb(0xeeeeee)))
                                .into_any_element()
                            };
                            el.child(div().mt(px(20.)).child(button))
                        })
                        .children(self.error.map(|message| {
                            div().mt(px(8.)).text_color(rgb(ERROR_COLOR)).child(message)
                        }))
                        .when(self.login_cancel.is_some(), |el| {
                            el.child(
                                action_button(
                                    "cancel-login",
                                    "Cancel sign-in",
                                    self.keyboard_focus_visible,
                                    cx.listener(|app, _, _, cx| app.cancel_login(cx)),
                                )
                                .mt(px(8.))
                                .hover(|el| el.bg(rgb(HOVER))),
                            )
                        })
                        .when(
                            !self.busy && self.config.is_some() && self.error.is_some(),
                            |el| {
                                el.child(
                                    action_button(
                                        "restore-session",
                                        "Retry saved session",
                                        self.keyboard_focus_visible,
                                        cx.listener(|app, _, _, cx| app.restore_session(cx)),
                                    )
                                    .mt(px(8.))
                                    .hover(|el| el.bg(rgb(HOVER))),
                                )
                            },
                        ),
                )
                .into_any_element();
        }
        let width = window.viewport_size().width;
        if width < px(720.) {
            self.mode = Mode::Day;
        }
        div()
            .id("calendar")
            .relative()
            .size_full()
            .bg(rgb(SURFACE))
            .text_color(rgb(TEXT_STRONG))
            .font_family("Mulish")
            .text_size(px(13.))
            .on_key_down(cx.listener(|app, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let mods = event.keystroke.modifiers;
                match key {
                    "enter"
                        if app
                            .editor
                            .as_ref()
                            .is_some_and(|editor| editor.read(cx).submit_focused(window, cx)) =>
                    {
                        cx.stop_propagation();
                        window.prevent_default();
                        app.save_editor(cx);
                    }
                    "enter"
                        if app.editor.as_mut().is_some_and(|editor| {
                            editor.update(cx, |editor, cx| editor.finish_time_entry(window, cx))
                        }) =>
                    {
                        cx.stop_propagation();
                        window.prevent_default();
                        cx.notify();
                    }
                    "escape" => {
                        let dismissed_picker = app.editor.as_ref().is_some_and(|editor| {
                            editor.update(cx, |editor, _| editor.dismiss_picker())
                        });
                        if !dismissed_picker {
                            if app.editor.take().is_some() {
                                app.error = None;
                            }
                            app.drag = None;
                        }
                        cx.notify();
                    }
                    "z" if app.editor.is_none()
                        && (mods.platform || mods.control)
                        && !mods.alt
                        && !mods.shift =>
                    {
                        app.undo(cx)
                    }
                    _ => {}
                }
            }))
            .child(pointer_focus)
            .child(self.calendar(window, cx))
            .into_any_element()
    }
}
