use std::{collections::HashMap, time::Instant};

use chrono::{Datelike, Duration, NaiveDate, Timelike};
use gpui::{prelude::*, *};
use gpui_component::menu::{PopupMenu, PopupMenuItem};

mod layout;
use layout::{card_height, layout_timed_events};

use super::{Desktop, Mode, UndoNotice};
use crate::{
    icons::{self, Icon},
    model::{Event, event_time, snap},
    style::{
        ACCENT, ERROR_COLOR, EVENT_SHADOW, HOUR_LABEL_COLOR, HOVER, LINE, SURFACE, TEXT_SECONDARY,
        TEXT_STRONG, action_button, bare_control, control, focus_outline,
    },
};

const RAIL: f32 = 52.;
const HEADER: f32 = 42.;

#[derive(Clone, PartialEq)]
struct PlacedEvent {
    event: Event,
    frame: CardFrame,
}

#[derive(Clone, Copy, PartialEq)]
struct CardFrame {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    opacity: f32,
}

struct CardTransition {
    from: CardFrame,
    previous: Option<Event>,
    started: Instant,
}

enum DragCommit {
    Move,
    Resize(Event),
}

impl CardTransition {
    fn progress(&self, now: Instant) -> f32 {
        (now.duration_since(self.started).as_secs_f32() / 0.24).min(1.)
    }
}

#[derive(Default)]
pub(crate) struct EventMotion {
    cards: Vec<PlacedEvent>,
    transitions: HashMap<String, CardTransition>,
    exits: Vec<PlacedEvent>,
    days: Vec<NaiveDate>,
    drag_commit: Option<DragCommit>,
}

impl PlacedEvent {
    fn animated_frame(&self, transition: Option<&CardTransition>, now: Instant) -> CardFrame {
        let Some(transition) = transition else {
            return self.frame;
        };
        let progress = transition.progress(now);
        let position = ease_in_out(progress);
        let fade = ease_out_quint()((progress * 4. / 3.).min(1.));
        let from = transition.from;
        CardFrame {
            x: from.x + (self.frame.x - from.x) * position,
            y: from.y + (self.frame.y - from.y) * position,
            width: from.width + (self.frame.width - from.width) * position,
            height: from.height + (self.frame.height - from.height) * position,
            opacity: from.opacity + (1. - from.opacity) * fade,
        }
    }
}

impl EventMotion {
    fn update(&mut self, cards: Vec<PlacedEvent>, days: &[NaiveDate], top: f32, now: Instant) {
        let drag_commit = self.drag_commit.take();
        self.transitions
            .retain(|_, motion| motion.progress(now) < 1.);
        self.exits
            .retain(|card| self.transitions.contains_key(&card.event.id));
        if self.days != days
            || matches!(drag_commit, Some(DragCommit::Move))
            || icons::reduced_motion()
        {
            self.days = days.to_vec();
            self.transitions.clear();
            self.exits.clear();
            self.cards = cards;
            return;
        }
        if self.cards == cards {
            return;
        }
        for next in &cards {
            let id = &next.event.id;
            let old = self.cards.iter().find(|card| &card.event.id == id);
            if old == Some(next) {
                continue;
            }
            let (from, previous) = if matches!(&drag_commit, Some(DragCommit::Resize(event)) if event == &next.event)
            {
                (
                    CardFrame {
                        opacity: 0.88,
                        ..next.frame
                    },
                    None,
                )
            } else if let Some(old) = old {
                let mut from = old.animated_frame(self.transitions.get(id), now);
                if old.event.all_day != next.event.all_day {
                    from.y += if old.event.all_day { -top } else { top };
                }
                (from, Some(old.event.clone()))
            } else {
                let opacity = self.transitions.get(id).map_or(0., |motion| {
                    motion.from.opacity * (1. - ease_out_quint()(motion.progress(now)))
                });
                (
                    CardFrame {
                        opacity,
                        ..next.frame
                    },
                    None,
                )
            };
            self.transitions.insert(
                id.clone(),
                CardTransition {
                    from,
                    previous,
                    started: now,
                },
            );
        }
        self.exits
            .retain(|old| !cards.iter().any(|card| card.event.id == old.event.id));
        for old in self
            .cards
            .iter()
            .filter(|old| !cards.iter().any(|card| card.event.id == old.event.id))
        {
            let from = old.animated_frame(self.transitions.get(&old.event.id), now);
            self.transitions.insert(
                old.event.id.clone(),
                CardTransition {
                    from,
                    previous: None,
                    started: now,
                },
            );
            self.exits.push(old.clone());
        }
        self.cards = cards;
    }
}

pub struct Drag {
    original: Event,
    origin: Point<Pixels>,
    grab: f32,
    resize: bool,
    active: bool,
    preview: Option<Event>,
}

#[cfg(test)]
impl Drag {
    pub(crate) fn is_resize(&self) -> bool {
        self.resize
    }
}
impl Desktop {
    pub fn calendar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let viewport = window.viewport_size();
        let compact = viewport.width < px(720.);
        let days: Vec<_> = (0..self.mode.count())
            .map(|index| self.anchor + Duration::days(index as i64))
            .collect();
        let col = ((f32::from(viewport.width) - RAIL) / days.len() as f32).max(150.);
        let board_width = viewport.width.max(px(RAIL + 150. * days.len() as f32));
        let scrolls_horizontally = board_width - viewport.width > px(0.5);
        let all_day = self
            .store
            .as_ref()
            .map(|store| {
                days.iter()
                    .map(|date| {
                        store
                            .events
                            .iter()
                            .filter(|e| e.date == *date && e.all_day)
                            .count()
                    })
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let lane = (all_day as f32 * 26. + 5.).max(34.);
        let hidden_events = self.store.as_ref().map_or(0, |store| store.hidden_events);
        let notice_height = if hidden_events > 0 { 28. } else { 0. };
        let hour =
            ((f32::from(viewport.height) - 56. - notice_height - HEADER - lane) / 8.).max(1.);
        let now = Instant::now();
        let mut cards = Vec::new();
        let mut timed_preview = None;
        if let Some(store) = &self.store {
            let preview = self
                .drag
                .as_ref()
                .filter(|drag| !drag.resize)
                .and_then(|drag| drag.preview.as_ref())
                .filter(|event| !event.all_day);
            for (column, day) in days.iter().enumerate() {
                let x = RAIL + column as f32 * col;
                let on_day = store.events.iter().filter(|event| event.date == *day);
                for (row, event) in on_day.clone().filter(|event| event.all_day).enumerate() {
                    cards.push(PlacedEvent {
                        event: event.clone(),
                        frame: CardFrame {
                            x,
                            y: 3. + row as f32 * 26.,
                            width: col - 4.,
                            height: card_height(event, hour),
                            opacity: 1.,
                        },
                    });
                }
                let timed = on_day.filter(|event| !event.all_day);
                cards.extend(
                    layout_timed_events(timed.clone().collect(), x, col, hour)
                        .into_iter()
                        .map(|(event, frame)| PlacedEvent {
                            event: event.clone(),
                            frame,
                        }),
                );
                if let Some(preview) = preview.filter(|event| event.date == *day) {
                    let events = timed
                        .filter(|event| event.id != preview.id)
                        .chain([preview])
                        .collect();
                    timed_preview = layout_timed_events(events, x, col, hour)
                        .into_iter()
                        .find(|(event, _)| event.id == preview.id);
                }
            }
        }
        self.event_motion
            .update(cards, &days, lane + f32::from(self.scroll.offset().y), now);
        if !self.event_motion.transitions.is_empty() {
            window.request_animation_frame();
        }

        let mut base = div()
            .id("calendar-content")
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .child(self.drag_listener(col, lane, hour, cx))
            .child(self.toolbar(compact, window, cx).flex_shrink_0())
            .when(hidden_events > 0, |el| {
                let message = if hidden_events == 1 {
                    "1 event couldn’t be displayed.".to_owned()
                } else {
                    format!("{hidden_events} events couldn’t be displayed.")
                };
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(notice_height))
                        .flex_shrink_0()
                        .px(px(20.))
                        .text_color(rgb(ERROR_COLOR))
                        .child(message),
                )
            });
        if self.store.is_some() {
            let today = self.now.date_naive();
            let new_event_day = days
                .iter()
                .copied()
                .find(|&day| day == today)
                .unwrap_or(self.anchor);
            let days_header = div()
                .flex()
                .w_full()
                .h(px(HEADER))
                .flex_shrink_0()
                .border_b_1()
                .border_color(rgb(LINE))
                .child(
                    icons::animate(
                        bare_control(
                            "add-event",
                            "",
                            self.keyboard_focus_visible,
                            cx.listener(move |app, _, window, cx| {
                                app.show_editor(
                                    Event::new(new_event_day, Some(9 * 60)),
                                    false,
                                    window,
                                    cx,
                                );
                            }),
                        ),
                        "add-event-icon",
                        Icon::Plus,
                        14.,
                        None,
                        window,
                        cx,
                    )
                    .w(px(RAIL))
                    .h_full()
                    .flex_shrink_0()
                    .text_size(px(16.))
                    .text_color(rgb(TEXT_SECONDARY))
                    .rounded(px(5.))
                    .hover(|el| el.bg(rgb(HOVER))),
                )
                .children(days.iter().map(|day| {
                    let today = *day == self.now.date_naive();
                    div()
                        .w(px(col))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(TEXT_SECONDARY))
                                .child(day.format("%a").to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(if today { ACCENT } else { TEXT_STRONG }))
                                .child(day.day().to_string()),
                        )
                }));
            let all_day = div()
                .relative()
                .flex()
                .h(px(lane))
                .w_full()
                .flex_shrink_0()
                .border_b_1()
                .border_color(rgb(LINE))
                .bg(rgb(HOVER))
                .child(div().w(px(RAIL)).flex_shrink_0())
                .children(days.iter().enumerate().map(|(index, &day)| {
                    let preview = self
                        .drag
                        .as_ref()
                        .and_then(|d| d.preview.as_ref())
                        .filter(|e| e.all_day && e.date == day);
                    div()
                        .id(("all-day", index))
                        .relative()
                        .w(px(col))
                        .flex_shrink_0()
                        .h_full()
                        .border_r_1()
                        .border_color(rgb(LINE))
                        .child(
                            div()
                                .id("create")
                                .absolute()
                                .inset_0()
                                .cursor_pointer()
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    if app.drag.is_none() {
                                        app.show_editor(Event::new(day, None), false, window, cx);
                                    }
                                })),
                        )
                        .children(preview.map(|event| {
                            card_skin(event, col - 4., card_height(event, hour), None)
                                .absolute()
                                .top(px(3.))
                                .left_0()
                                .right(px(3.))
                        }))
                }))
                .child(self.event_layer(true, hour, now, cx));
            let current_time = days.iter().position(|&day| day == today).map(|index| {
                let minute = self.now.hour() * 60 + self.now.minute();
                div()
                    .absolute()
                    .top(px(minute as f32 / 60. * hour))
                    .left(px(RAIL + index as f32 * col))
                    .w(px(col - 1.))
                    .h(px(2.))
                    .bg(rgb(ACCENT))
                    .child(
                        div()
                            .absolute()
                            .left(px(-4.))
                            .top(px(-2.5))
                            .size(px(7.))
                            .rounded_full()
                            .bg(rgb(ACCENT)),
                    )
            });
            let timeline =
                div()
                    .id("timeline")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .relative()
                            .flex()
                            .h(px(hour * 24.))
                            .child(div().w(px(RAIL)).flex_shrink_0().relative().children(
                                (1..24).map(|slot| {
                                    div()
                                        .absolute()
                                        .left_0()
                                        .w_full()
                                        .text_center()
                                        .top(px(hour * slot as f32 - 6.))
                                        .text_size(px(10.))
                                        .text_color(rgb(HOUR_LABEL_COLOR))
                                        .child(slot.to_string())
                                }),
                            ))
                            .children(days.iter().map(|day| self.column(*day, col, hour, cx)))
                            .child(self.event_layer(false, hour, now, cx))
                            .children(timed_preview.map(|(event, frame)| {
                                card_skin(event, frame.width, frame.height, None)
                                    .absolute()
                                    .left(px(frame.x))
                                    .w(px(frame.width))
                                    .top(px(frame.y))
                                    .shadow_md()
                                    .opacity(0.88)
                            }))
                            .children(current_time),
                    );
            base = base.child(
                div()
                    .id("board")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .on_scroll_wheel(cx.listener(move |app, event: &ScrollWheelEvent, _, cx| {
                        let delta = event.delta.pixel_delta(px(20.));
                        let dx = if delta.x.abs() > delta.y.abs() {
                            delta.x
                        } else if event.modifiers.shift {
                            delta.y
                        } else {
                            px(0.)
                        };
                        if dx == px(0.) || app.drag.is_some() {
                            return;
                        }
                        if scrolls_horizontally && !event.modifiers.shift {
                            return;
                        }
                        cx.stop_propagation();
                        let now = Instant::now();
                        if app
                            .wheel
                            .1
                            .is_none_or(|previous| now.duration_since(previous).as_millis() > 140)
                        {
                            app.wheel.0 = 0.;
                        }
                        app.wheel.0 += f32::from(dx);
                        app.wheel.1 = Some(now);
                        if app.wheel.0.abs() >= 80. {
                            app.shift(if app.wheel.0 < 0. { 1 } else { -1 });
                            app.wheel.0 = 0.;
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .w(board_width)
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(days_header)
                            .child(all_day)
                            .child(timeline),
                    ),
            );
            if self.needs_initial_scroll {
                self.needs_initial_scroll = false;
                let scroll = self.scroll.clone();
                window.on_next_frame(move |window, _| {
                    scroll.set_offset(point(px(0.), px(-9. * hour)));
                    window.refresh();
                });
            }
        } else {
            base = base.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .text_color(rgb(TEXT_SECONDARY))
                    .child(
                        div()
                            .when(self.error.is_some(), |el| el.text_color(rgb(ERROR_COLOR)))
                            .child(self.error.unwrap_or("Loading your calendar…")),
                    )
                    .when(self.error.is_some(), |el| {
                        el.child(
                            action_button(
                                "reload-events",
                                "Try again",
                                self.keyboard_focus_visible,
                                cx.listener(|app, _, _, cx| {
                                    app.refresh(cx);
                                    cx.notify();
                                }),
                            )
                            .rounded(px(8.))
                            .px(px(14.))
                            .bg(rgb(HOVER)),
                        )
                    }),
            );
        }
        if let Some(notice) = &self.undo_notice {
            let (message, color) = match notice {
                UndoNotice::Deleted => ("Event deleted", TEXT_STRONG),
                UndoNotice::RestoreFailed => ("Couldn’t restore. Try again.", ERROR_COLOR),
            };
            base = base.child(
                div()
                    .absolute()
                    .bottom(px(20.))
                    .right(px(20.))
                    .pl(px(16.))
                    .pr(px(12.))
                    .py(px(10.))
                    .rounded(px(8.))
                    .shadow(vec![BoxShadow {
                        color: rgba(EVENT_SHADOW).into(),
                        offset: point(px(0.), px(8.)),
                        blur_radius: px(12.),
                        spread_radius: px(0.),
                    }])
                    .bg(rgb(SURFACE))
                    .occlude()
                    .flex()
                    .items_center()
                    .gap(px(24.))
                    .line_height(px(18.))
                    .text_color(rgb(TEXT_STRONG))
                    .font_weight(FontWeight::MEDIUM)
                    .child(
                        div()
                            .id("undo-message")
                            .debug_selector(|| message.into())
                            .text_color(rgb(color))
                            .child(message),
                    )
                    .child(
                        action_button(
                            "undo",
                            "Undo",
                            self.keyboard_focus_visible,
                            cx.listener(|app, _, _, cx| app.undo(cx)),
                        )
                        .hover(|el| el.bg(rgb(HOVER))),
                    ),
            );
        } else if let Some(message) = self.error
            && self.editor.is_none()
            && self.store.is_some()
        {
            base = base.child(
                div()
                    .absolute()
                    .bottom(px(16.))
                    .right(px(16.))
                    .p_3()
                    .rounded(px(8.))
                    .shadow_md()
                    .bg(rgb(SURFACE))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_color(rgb(ERROR_COLOR)).child(message))
                    .child(control(
                        "dismiss",
                        "OK",
                        self.keyboard_focus_visible,
                        cx.listener(|app, _, _, cx| {
                            app.error = None;
                            cx.notify();
                        }),
                    )),
            );
        }
        if let Some(editor) = &self.editor {
            base = base.child(editor.update(cx, |editor, cx| {
                editor.render(
                    self.error,
                    viewport.width <= px(560.),
                    self.keyboard_focus_visible,
                    window,
                    cx,
                )
            }));
        }
        base.into_any_element()
    }

    fn toolbar(&self, compact: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let step = if self.mode == Mode::Week { 7 } else { 1 };
        let modes = [(Mode::Three, "3-day"), (Mode::Week, "Week")];
        div()
            .h(px(56.))
            .pl(px(20.))
            .pr(px(12.))
            .w_full()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .text_size(px(14.))
                    .child(self.anchor.format("%b %Y").to_string()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        icons::animate(
                            control(
                                "prev",
                                "",
                                self.keyboard_focus_visible,
                                cx.listener(move |app, _, _, cx| {
                                    app.shift(-step);
                                    cx.notify();
                                }),
                            ),
                            "prev-icon",
                            Icon::ChevronLeft,
                            17.,
                            None,
                            window,
                            cx,
                        )
                        .w(px(24.))
                        .h(px(24.))
                        .px(px(0.))
                        .rounded(px(5.))
                        .text_size(px(17.))
                        .text_color(rgb(TEXT_SECONDARY)),
                    )
                    .child(
                        control(
                            "today",
                            "Today",
                            self.keyboard_focus_visible,
                            cx.listener(|app, _, _, cx| {
                                app.today();
                                cx.notify();
                            }),
                        )
                        .h(px(24.))
                        .px(px(6.))
                        .rounded(px(5.))
                        .text_color(rgb(TEXT_SECONDARY)),
                    )
                    .child(
                        icons::animate(
                            control(
                                "next",
                                "",
                                self.keyboard_focus_visible,
                                cx.listener(move |app, _, _, cx| {
                                    app.shift(step);
                                    cx.notify();
                                }),
                            ),
                            "next-icon",
                            Icon::ChevronRight,
                            17.,
                            None,
                            window,
                            cx,
                        )
                        .w(px(24.))
                        .h(px(24.))
                        .px(px(0.))
                        .rounded(px(5.))
                        .text_size(px(17.))
                        .text_color(rgb(TEXT_SECONDARY)),
                    ),
            )
            .child(div().flex_1())
            .when(!compact, |toolbar| {
                toolbar.child(
                    div()
                        .flex()
                        .mr(px(8.))
                        .p(px(2.))
                        .rounded(px(7.))
                        .bg(rgb(HOVER))
                        .children(modes.iter().map(|&(mode, name)| {
                            bare_control(
                                name,
                                name,
                                self.keyboard_focus_visible,
                                cx.listener(move |app, _, _, cx| {
                                    app.set_mode(mode);
                                    cx.notify();
                                }),
                            )
                            .min_w(px(52.))
                            .h(px(26.))
                            .text_color(rgb(TEXT_SECONDARY))
                            .when(self.mode == mode, |el| {
                                el.bg(rgb(SURFACE)).text_color(rgb(TEXT_STRONG))
                            })
                        })),
                )
            })
            .child(self.account_control(window, cx))
    }

    fn account_control(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let button = div()
            .id("account")
            .debug_selector(|| "account-button".into())
            .tab_index(0)
            .size(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(HOVER))
            .text_color(rgb(if self.account_menu.is_some() {
                TEXT_STRONG
            } else {
                TEXT_SECONDARY
            }))
            .hover(|el| el.text_color(rgb(TEXT_STRONG)))
            .when(self.keyboard_focus_visible, |el| el.focus(focus_outline));
        let (button, focus) = icons::with_animation(button, "account-icon", window, cx, |start| {
            icons::draw(Icon::UserRound, 17., None, start)
        });
        let button = button
            .when(self.account_menu.is_none(), |button| {
                button.on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let focus = focus.clone();
                        move |app, _, window, cx| {
                            window.prevent_default();
                            app.open_account_menu(&focus, window, cx);
                        }
                    }),
                )
            })
            .on_click(cx.listener(move |app, event, window, cx| {
                if matches!(event, ClickEvent::Keyboard(_)) {
                    app.open_account_menu(&focus, window, cx);
                }
            }));

        div().relative().flex_shrink_0().child(button).when_some(
            self.account_menu.clone(),
            |el, menu| {
                el.child(
                    deferred(
                        div()
                            .absolute()
                            .right_0()
                            .top(px(42.))
                            .occlude()
                            .child(menu),
                    )
                    .with_priority(2),
                )
            },
        )
    }

    fn open_account_menu(
        &mut self,
        focus: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, user)) = &self.account else {
            return;
        };
        let email = SharedString::from(user.email.clone());
        let sign_out = cx.listener(|app, _, _, cx| app.sign_out(cx));
        let menu = PopupMenu::build(window, cx, |menu, _, _| {
            menu.action_context(focus.clone())
                .min_w(px(200.))
                .max_w(px(300.))
                .item(
                    PopupMenuItem::element(move |_, _| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_h(px(36.))
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .text_color(rgb(TEXT_STRONG))
                            .child(icons::draw(Icon::Mail, 16., TEXT_STRONG, None))
                            .child(div().truncate().child(email.clone()))
                    })
                    .disabled(true),
                )
                .item(
                    PopupMenuItem::element(|_, _| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_h(px(36.))
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .child(icons::draw(Icon::LogOut, 16., TEXT_STRONG, None))
                            .child("Log out")
                    })
                    .on_click(sign_out),
                )
        });
        cx.subscribe(&menu, |app, _, _: &DismissEvent, cx| {
            app.account_menu = None;
            cx.notify();
        })
        .detach();
        self.account_menu = Some(menu.clone());
        menu.focus_handle(cx).focus(window);
        cx.notify();
    }

    fn event_layer(&self, all_day: bool, hour: f32, now: Instant, cx: &mut Context<Self>) -> Div {
        let motion = &self.event_motion;
        div()
            .absolute()
            .inset_0()
            .children(
                motion
                    .cards
                    .iter()
                    .filter(|card| card.event.all_day == all_day)
                    .map(|card| {
                        let transition = motion.transitions.get(&card.event.id);
                        let frame = card.animated_frame(transition, now);
                        let change = transition.and_then(|motion| {
                            motion.previous.as_ref().map(|old| {
                                (
                                    old,
                                    ease_out_quint()((motion.progress(now) * 4. / 3.).min(1.)),
                                )
                            })
                        });
                        self.card(&card.event, hour, frame, change, cx)
                            .absolute()
                            .left(px(frame.x))
                            .top(px(frame.y))
                            .w(px(frame.width))
                            .when(frame.opacity < 1., |el| el.opacity(frame.opacity))
                    }),
            )
            .children(
                motion
                    .exits
                    .iter()
                    .filter(|card| card.event.all_day == all_day)
                    .filter_map(|card| {
                        let exit = motion.transitions.get(&card.event.id)?;
                        let frame = exit.from;
                        let opacity = frame.opacity
                            * (1. - ease_out_quint()((exit.progress(now) * 4. / 3.).min(1.)));
                        Some(
                            card_skin(&card.event, frame.width, frame.height, None)
                                .absolute()
                                .left(px(frame.x))
                                .top(px(frame.y))
                                .w(px(frame.width))
                                .opacity(opacity),
                        )
                    }),
            )
    }

    fn column(&self, day: NaiveDate, width: f32, hour: f32, cx: &mut Context<Self>) -> Div {
        div()
            .relative()
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(rgb(LINE))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .children((0..24).map(|slot| {
                        div()
                            .id(SharedString::from(format!("{day}-{slot}")))
                            .w_full()
                            .h(px(hour))
                            .flex_shrink_0()
                            .border_b_1()
                            .border_color(rgb(LINE))
                            .cursor_pointer()
                            .on_click(cx.listener(move |app, _, window, cx| {
                                if app.drag.is_none() {
                                    app.show_editor(
                                        Event::new(day, Some(slot * 60)),
                                        false,
                                        window,
                                        cx,
                                    );
                                }
                            }))
                    })),
            )
    }

    fn card(
        &self,
        event: &Event,
        hour: f32,
        frame: CardFrame,
        change: Option<(&Event, f32)>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let picked = self
            .drag
            .as_ref()
            .filter(|d| d.active && d.original.id == event.id);
        let drawn = picked
            .filter(|d| d.resize)
            .and_then(|d| d.preview.as_ref())
            .unwrap_or(event);
        let move_event = event.clone();
        let resize_event = event.clone();
        let open_event = event.clone();
        let height = if drawn != event {
            card_height(drawn, hour)
        } else {
            frame.height
        };
        card_skin(drawn, frame.width, height, change)
            .id(SharedString::from(event.id.clone()))
            .block_mouse_except_scroll()
            .relative()
            .tab_index(0)
            .when(self.keyboard_focus_visible, |el| el.focus(focus_outline))
            .cursor(CursorStyle::OpenHand)
            .active(|el| el.cursor(CursorStyle::ClosedHand))
            .when(picked.is_some_and(|d| !d.resize), |el| el.opacity(0.24))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |app, e: &MouseDownEvent, _, _| {
                    let timeline_top = app.scroll.bounds().top() + app.scroll.offset().y;
                    let grab = if move_event.all_day {
                        0.
                    } else {
                        f32::from(e.position.y - timeline_top).max(0.) - frame.y
                    };
                    app.drag = Some(Drag {
                        original: move_event.clone(),
                        origin: e.position,
                        grab: grab.max(0.),
                        resize: false,
                        active: false,
                        preview: None,
                    });
                }),
            )
            .on_click(cx.listener(move |app, click: &ClickEvent, window, cx| {
                if let ClickEvent::Mouse(mouse) = click {
                    let delta = mouse.up.position - mouse.down.position;
                    if delta.x.abs().max(delta.y.abs()) >= px(4.) {
                        return;
                    }
                }
                if app.drag.take().is_none_or(|d| !d.active) {
                    cx.stop_propagation();
                    app.show_editor(open_event.clone(), true, window, cx);
                }
            }))
            .on_key_down(cx.listener({
                let event = event.clone();
                move |app, key: &KeyDownEvent, _, cx| match key.keystroke.key.as_str() {
                    "up" | "down" if !event.all_day => {
                        app.save(
                            event.resized(
                                event.end + if key.keystroke.key == "up" { -5 } else { 5 },
                            ),
                            cx,
                        );
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .when(!event.all_day, |el| {
                el.child(
                    div()
                        .id("resize")
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(6.))
                        .cursor(CursorStyle::ResizeUpDown)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |app, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                app.drag = Some(Drag {
                                    original: resize_event.clone(),
                                    origin: event.position,
                                    grab: 0.,
                                    resize: true,
                                    active: false,
                                    preview: None,
                                });
                            }),
                        )
                        .on_click(|_, _, cx| cx.stop_propagation()),
                )
            })
    }

    // A drag can leave its card or cross an overlapping card before release.
    fn drag_listener(
        &self,
        column: f32,
        lane: f32,
        hour: f32,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let calendar = cx.weak_entity();
        canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                let view = calendar.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    view.update(cx, |app, cx| {
                        if app.drag.is_none() {
                            return;
                        }
                        if event.dragging() {
                            app.drag_to(event.position, column, lane, hour);
                        } else {
                            app.drag = None;
                        }
                        cx.notify();
                    })
                    .ok();
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                        return;
                    }
                    let handled = calendar
                        .update(cx, |app, cx| {
                            let Some(drag) = app.drag.take().filter(|drag| drag.active) else {
                                return false;
                            };
                            if let Some(next) = drag.preview
                                && next != drag.original
                            {
                                app.event_motion.drag_commit = Some(if drag.resize {
                                    DragCommit::Resize(next.clone())
                                } else {
                                    DragCommit::Move
                                });
                                app.save(next, cx);
                            }
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if handled {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .size(px(0.))
    }

    fn drag_to(&mut self, position: Point<Pixels>, column: f32, lane: f32, hour: f32) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        let delta = position - drag.origin;
        if !drag.active && delta.x.abs().max(delta.y.abs()) < px(4.) {
            return;
        }
        drag.active = true;
        let board = self.scroll.bounds();
        if drag.resize {
            let minute = drag.original.end as f32 + f32::from(delta.y) / hour * 60.;
            drag.preview = Some(drag.original.resized(snap(minute)));
            return;
        }
        let x = f32::from(position.x - board.left()) - RAIL;
        let index = (x / column).floor() as i64;
        let y = f32::from(position.y - board.top());
        if index < 0
            || index >= self.mode.count() as i64
            || y < -lane
            || position.y > board.bottom()
        {
            drag.preview = None;
            return;
        }
        let date = self.anchor + Duration::days(index);
        let minute = (y >= 0.)
            .then(|| snap((y - f32::from(self.scroll.offset().y) - drag.grab) / hour * 60.));
        drag.preview = Some(drag.original.moved(date, minute));
    }
}

fn card_skin(event: &Event, width: f32, height: f32, change: Option<(&Event, f32)>) -> Div {
    let timed = shows_time(event);
    let color = event.calendar.color();
    let fill = |event: &Event| {
        mix(
            event.calendar.color(),
            SURFACE,
            if event.all_day { 0.28 } else { 0.24 },
        )
    };
    let background = change.map_or_else(
        || fill(event),
        |(old, progress)| mix(fill(event), fill(old), progress),
    );
    div()
        .h(px(height))
        .flex_shrink_0()
        .overflow_hidden()
        .rounded(px(8.))
        .px(px((if event.all_day { 10_f32 } else { 8. }).min(width / 4.)))
        .py(px(if timed { 4. } else { 2. }))
        .bg(rgb(background))
        .child(
            div()
                .relative()
                .child(card_copy(event, change, color))
                .children(change.filter(|(old, _)| copy_changed(old, event)).map(
                    |(old, progress)| {
                        card_copy(old, None, old.calendar.color())
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .opacity(1. - progress)
                    },
                )),
        )
}

fn shows_time(event: &Event) -> bool {
    !event.all_day && event.end - event.start >= 30
}

fn copy_changed(old: &Event, event: &Event) -> bool {
    old.title != event.title
        || shows_time(old) != shows_time(event)
        || (shows_time(event) && (old.start, old.end) != (event.start, event.end))
}

fn card_copy(event: &Event, change: Option<(&Event, f32)>, color: u32) -> Div {
    let text_color = change.map_or(color, |(old, progress)| {
        mix(color, old.calendar.color(), progress)
    });
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .when_some(
            change.filter(|(old, _)| copy_changed(old, event)),
            |el, (_, progress)| el.opacity(progress),
        )
        .child(
            div()
                .line_clamp(3)
                .text_size(px(13.))
                .line_height(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(mix(text_color, TEXT_STRONG, 0.82)))
                .child(event.title.clone()),
        )
        .when(shows_time(event), |el| {
            el.child(
                div()
                    .line_clamp(1)
                    .text_size(px(10.))
                    .line_height(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(text_color))
                    .child(format!(
                        "{} – {}",
                        event_time(event.start),
                        event_time(event.end)
                    )),
            )
        })
}

fn mix(from: u32, to: u32, weight: f32) -> u32 {
    [16, 8, 0].into_iter().fold(0, |rgb, shift| {
        let a = ((from >> shift) & 255) as f32;
        let b = ((to >> shift) & 255) as f32;
        rgb | (((a * weight + b * (1. - weight)).round() as u32) << shift)
    })
}
