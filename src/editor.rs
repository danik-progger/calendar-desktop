use chrono::{Datelike, Duration, Months, NaiveDate};
use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::{
    icons::{self, Icon},
    model::{Calendar, Event, clamp_range, clock, monday, parse_time},
    style::{
        ACCENT, ERROR_COLOR, EVENT_SHADOW, HOVER, MODAL_OVERLAY, OUTSIDE_MONTH, PLACEHOLDER,
        SURFACE, TEXT_ON_SOLID, TEXT_SECONDARY, TEXT_STRONG, action_button, bare_control,
    },
};

pub(super) enum EditorAction {
    Save,
    Delete,
    Cancel,
}

impl EventEmitter<EditorAction> for Editor {}

#[derive(Clone, Copy, PartialEq)]
enum Picker {
    Date,
    Start,
    End,
    Calendar,
}

const DROPDOWN_ROW_HEIGHT: f32 = 36.;

pub struct Editor {
    event: Event,
    picker: Option<Picker>,
    existing: bool,
    month: NaiveDate,
    time_scroll: ScrollHandle,
    title: Entity<InputState>,
    notes: Entity<InputState>,
    start: Entity<InputState>,
    end: Entity<InputState>,
    _subscriptions: [Subscription; 4],
}

impl Editor {
    pub fn new(event: Event, existing: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Event title")
                .default_value(event.title.clone())
        });
        let notes = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(3)
                .placeholder("Add description...")
                .default_value(event.notes.clone())
        });
        let start = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Start time")
                .default_value(clock(event.start))
        });
        let end = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("End time")
                .default_value(clock(event.end))
        });
        let subscriptions =
            [(&start, Picker::Start), (&end, Picker::End)].map(|(field, picker)| {
                cx.subscribe(field, move |app, _, event, cx| {
                    if matches!(event, InputEvent::Focus) {
                        app.open_picker(picker, cx);
                    }
                })
            });
        let [start_subscription, end_subscription] = subscriptions;
        let [title_subscription, notes_subscription] = [&title, &notes].map(|field| {
            cx.subscribe(field, |app, _, event, cx| {
                if matches!(event, InputEvent::Focus) && app.dismiss_picker() {
                    cx.notify();
                }
            })
        });
        let subscriptions = [
            start_subscription,
            end_subscription,
            title_subscription,
            notes_subscription,
        ];
        title.update(cx, |state, cx| state.focus(window, cx));
        Self {
            month: event.date.with_day(1).unwrap_or(event.date),
            event,
            existing,
            title,
            notes,
            start,
            end,
            picker: None,
            time_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn render(
        &self,
        message: Option<&str>,
        compact: bool,
        keyboard_focus_visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let inset = px(if compact { 22. } else { 28. });
        let time_list_height = px(226.).min(window.viewport_size().height / 2.) - px(8.);
        let panel = div()
            .relative()
            .w(px(420.))
            .max_w_full()
            .max_h_full()
            .rounded(px(8.))
            .bg(rgb(SURFACE))
            .shadow(vec![BoxShadow {
                color: rgba(EVENT_SHADOW).into(),
                offset: point(px(0.), px(24.)),
                blur_radius: px(32.),
                spread_radius: px(0.),
            }])
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("editor-body")
                    .max_h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .pt(px(28.))
                            .text_color(rgb(TEXT_SECONDARY))
                            .child(
                                div()
                                    .px(px(if compact { 24. } else { 28. }))
                                    .pb(px(16.))
                                    .child(
                                        editor_input(&self.title, 20., 26., 26.)
                                            .text_color(rgb(TEXT_STRONG))
                                            .font_weight(FontWeight::BOLD),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(if compact { 9. } else { 10. }))
                                    .px(inset)
                                    .h(px(32.))
                                    .child(
                                        div()
                                            .relative()
                                            .w(px(140.))
                                            .child(
                                                icons::animate(
                                                    bare_control(
                                                        "date",
                                                        "",
                                                        keyboard_focus_visible,
                                                        cx.listener(|app, _, _, cx| {
                                                            app.toggle_picker(Picker::Date, cx)
                                                        }),
                                                    ),
                                                    "date-icon",
                                                    Icon::CalendarDays,
                                                    16.,
                                                    PLACEHOLDER,
                                                    window,
                                                    cx,
                                                )
                                                .justify_start()
                                                .h(px(28.))
                                                .px_0()
                                                .gap(px(12.))
                                                .text_color(rgb(TEXT_SECONDARY))
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(
                                                    self.event.date.format("%m.%d.%Y").to_string(),
                                                ),
                                            )
                                            .when(self.picker == Some(Picker::Date), |el| {
                                                el.child(
                                                    deferred(self.date_picker(
                                                        keyboard_focus_visible,
                                                        window,
                                                        cx,
                                                    ))
                                                    .with_priority(2),
                                                )
                                            }),
                                    )
                                    .when(!self.event.all_day, |row| {
                                        row.child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(2.))
                                                .child(self.time_field(
                                                    Picker::Start,
                                                    &self.start,
                                                    time_list_height,
                                                    keyboard_focus_visible,
                                                    cx,
                                                ))
                                                .child(
                                                    div()
                                                        .text_size(px(12.))
                                                        .text_color(rgb(PLACEHOLDER))
                                                        .child("–"),
                                                )
                                                .child(self.time_field(
                                                    Picker::End,
                                                    &self.end,
                                                    time_list_height,
                                                    keyboard_focus_visible,
                                                    cx,
                                                )),
                                        )
                                    }),
                            )
                            .child(
                                icons::with_animation(
                                    bare_control(
                                        "all-day-toggle",
                                        "",
                                        keyboard_focus_visible,
                                        cx.listener(|app, _, _, cx| {
                                            app.event.all_day = !app.event.all_day;
                                            app.picker = None;
                                            cx.notify();
                                        }),
                                    ),
                                    "all-day-icon",
                                    window,
                                    cx,
                                    |start| checkbox(self.event.all_day, start),
                                )
                                .0
                                .ml(inset)
                                .w(px(85.))
                                .h(px(32.))
                                .px_0()
                                .gap(px(if compact { 13. } else { 14. }))
                                .text_color(rgb(TEXT_SECONDARY))
                                .font_weight(FontWeight::MEDIUM)
                                .justify_start()
                                .child("All day"),
                            )
                            .child(
                                div()
                                    .relative()
                                    .mx(inset)
                                    .mt(px(4.))
                                    .h(px(32.))
                                    .child(
                                        icons::animate(
                                            bare_control(
                                                "calendar-picker",
                                                "",
                                                keyboard_focus_visible,
                                                cx.listener(|app, _, _, cx| {
                                                    app.toggle_picker(Picker::Calendar, cx)
                                                }),
                                            )
                                            .child(swatch(self.event.calendar).mr(px(14.)))
                                            .child(
                                                div().mr(px(4.)).child(self.event.calendar.label()),
                                            ),
                                            "calendar-picker-icon",
                                            Icon::ChevronDown,
                                            13.,
                                            PLACEHOLDER,
                                            window,
                                            cx,
                                        )
                                        .justify_start()
                                        .px_0()
                                        .gap(px(0.))
                                        .text_color(rgb(TEXT_SECONDARY))
                                        .font_weight(FontWeight::MEDIUM),
                                    )
                                    .when(self.picker == Some(Picker::Calendar), |el| {
                                        el.child(
                                            deferred(
                                                self.calendar_picker(keyboard_focus_visible, cx),
                                            )
                                            .with_priority(2),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .px(inset)
                                    .pt(px(18.))
                                    .pb(px(12.))
                                    .text_size(px(12.))
                                    .child(editor_input(&self.notes, 12., 17.4, 62.)),
                            )
                            .children(message.map(|text| {
                                div()
                                    .px(inset)
                                    .pb_2()
                                    .text_size(px(12.))
                                    .text_color(rgb(ERROR_COLOR))
                                    .child(text.to_owned())
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .py(px(10.))
                            .px(px(16.))
                            .when(self.existing, |el| {
                                el.child(
                                    action_button(
                                        "delete",
                                        "Delete",
                                        keyboard_focus_visible,
                                        cx.listener(|_, _, _, cx| cx.emit(EditorAction::Delete)),
                                    )
                                    .hover(|el| el.bg(rgb(HOVER))),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                action_button(
                                    "cancel",
                                    "Cancel",
                                    keyboard_focus_visible,
                                    cx.listener(|_, _, _, cx| cx.emit(EditorAction::Cancel)),
                                )
                                .hover(|el| el.bg(rgb(HOVER))),
                            )
                            .child(
                                action_button(
                                    "save",
                                    "Save",
                                    keyboard_focus_visible,
                                    cx.listener(|_, _, _, cx| cx.emit(EditorAction::Save)),
                                )
                                .bg(rgb(TEXT_STRONG))
                                .text_color(rgb(TEXT_ON_SOLID)),
                            ),
                    ),
            );
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p(px(16.))
            .child(
                div()
                    .id("editor-overlay")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .bg(rgba(MODAL_OVERLAY))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.emit(EditorAction::Cancel);
                        }),
                    ),
            )
            .child(panel.occlude())
    }

    fn time_field(
        &self,
        picker: Picker,
        field: &Entity<InputState>,
        list_height: Pixels,
        keyboard_focus_visible: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut input = editor_input(field, 12., 16., 26.).pl(px(3.));
        input
            .text_style()
            .get_or_insert_with(Default::default)
            .font_features = Some(FontFeatures(vec![("tnum".into(), 1)].into()));
        div()
            .relative()
            // Leave room for the input component's 10 px caret margin after HH:MM.
            .w(px(44.))
            .h(px(32.))
            .flex()
            .flex_shrink_0()
            .items_center()
            .child(input)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |app, _, _, cx| {
                    app.open_picker(picker, cx);
                }),
            )
            .when(self.picker == Some(picker), |el| {
                el.child(
                    deferred(self.time_picker(picker, list_height, keyboard_focus_visible, cx))
                        .with_priority(2),
                )
            })
    }

    fn time_picker(
        &self,
        picker: Picker,
        list_height: Pixels,
        keyboard_focus_visible: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let field = if picker == Picker::Start {
            &self.start
        } else {
            &self.end
        };
        let initial = parse_time(&field.read(cx).value());
        dropdown_panel(104.)
            .debug_selector(|| "time-picker".into())
            .child(
                div()
                    .id("time-options")
                    .max_h(list_height)
                    .overflow_y_scroll()
                    .track_scroll(&self.time_scroll)
                    .children((0..48).map(|index| {
                        let minute = index * 30;
                        bare_control(
                            ("time", index as usize),
                            clock(minute),
                            keyboard_focus_visible,
                            cx.listener(move |app, _, window, cx| {
                                let field = if picker == Picker::Start {
                                    &app.start
                                } else {
                                    &app.end
                                };
                                field.update(cx, |input, cx| {
                                    input.set_value(clock(minute), window, cx)
                                });
                                app.picker = None;
                                cx.notify();
                            }),
                        )
                        .map(|option| dropdown_option(option, Some(minute) == initial))
                    })),
            )
    }

    fn date_picker(
        &self,
        keyboard_focus_visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let first = monday(self.month);
        picker_panel(252.)
            .p(px(10.))
            .rounded(px(8.))
            .shadow(vec![BoxShadow {
                offset: point(px(0.), px(8.)),
                blur_radius: px(12.),
                ..dropdown_shadow()
            }])
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(2.))
                    .pb(px(6.))
                    .text_color(rgb(TEXT_STRONG))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.month.format("%B %Y").to_string())
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .child(
                                icons::animate(
                                    bare_control(
                                        "prev-month",
                                        "",
                                        keyboard_focus_visible,
                                        cx.listener(|app, _, _, cx| app.shift_month(false, cx)),
                                    ),
                                    "prev-month-icon",
                                    Icon::ChevronLeft,
                                    17.,
                                    TEXT_SECONDARY,
                                    window,
                                    cx,
                                )
                                .size(px(26.))
                                .px(px(0.))
                                .text_size(px(20.))
                                .text_color(rgb(TEXT_SECONDARY))
                                .rounded(px(5.)),
                            )
                            .child(
                                icons::animate(
                                    bare_control(
                                        "next-month",
                                        "",
                                        keyboard_focus_visible,
                                        cx.listener(|app, _, _, cx| app.shift_month(true, cx)),
                                    ),
                                    "next-month-icon",
                                    Icon::ChevronRight,
                                    17.,
                                    TEXT_SECONDARY,
                                    window,
                                    cx,
                                )
                                .size(px(26.))
                                .px(px(0.))
                                .text_size(px(20.))
                                .text_color(rgb(TEXT_SECONDARY))
                                .rounded(px(5.)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .children(["M", "T", "W", "T", "F", "S", "S"].map(|day| {
                        div()
                            .flex_1()
                            .flex()
                            .justify_center()
                            .items_center()
                            .h(px(29.))
                            .text_color(rgb(PLACEHOLDER))
                            .child(day)
                    })),
            )
            .children((0..6).map(|week| {
                div().flex().gap(px(2.)).children((0..7).map(|day| {
                    let date = first + Duration::days(week * 7 + day);
                    bare_control(
                        ("date-option", (week * 7 + day) as usize),
                        date.day().to_string(),
                        keyboard_focus_visible,
                        cx.listener(move |app, _, _, cx| {
                            app.event.date = date;
                            app.picker = None;
                            cx.notify();
                        }),
                    )
                    .flex_1()
                    .px_0()
                    .h(px(29.))
                    .rounded(px(5.))
                    .text_color(rgb(if date.month() == self.month.month() {
                        TEXT_SECONDARY
                    } else {
                        OUTSIDE_MONTH
                    }))
                    .when(date == self.event.date, |option| {
                        option
                            .bg(rgb(TEXT_STRONG))
                            .text_color(rgb(TEXT_ON_SOLID))
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(date != self.event.date, |option| {
                        option.hover(|style| style.bg(rgb(HOVER)))
                    })
                }))
            }))
    }

    fn calendar_picker(&self, keyboard_focus_visible: bool, cx: &mut Context<Self>) -> Div {
        dropdown_panel(156.).children(Calendar::ALL.map(|calendar| {
            bare_control(
                calendar.label(),
                "",
                keyboard_focus_visible,
                cx.listener(move |app, _, _, cx| {
                    app.event.calendar = calendar;
                    app.picker = None;
                    cx.notify();
                }),
            )
            .child(swatch(calendar).size(px(14.)).rounded(px(5.)))
            .child(calendar.label())
            .map(|option| dropdown_option(option, calendar == self.event.calendar))
        }))
    }

    pub fn event_id(&self) -> &str {
        &self.event.id
    }

    pub fn submit_focused(&self, window: &Window, cx: &App) -> bool {
        self.title.focus_handle(cx).is_focused(window)
    }

    pub fn finish_time_entry(&mut self, window: &mut Window, cx: &mut App) -> bool {
        let (field, fallback) = if self.start.focus_handle(cx).is_focused(window) {
            (&self.start, self.event.start)
        } else if self.end.focus_handle(cx).is_focused(window) {
            (&self.end, self.event.end)
        } else {
            return false;
        };
        let minute = parse_time(&field.read(cx).value()).unwrap_or(fallback);
        field.update(cx, |input, cx| input.set_value(clock(minute), window, cx));
        self.picker = None;
        true
    }

    pub(super) fn draft(&self, cx: &App) -> Result<Event, &'static str> {
        let mut event = self.event.clone();
        event.title = self.title.read(cx).value().trim().to_owned();
        if event.title.is_empty() {
            return Err("Add a title to save this event.");
        }
        event.notes = self.notes.read(cx).value().to_string();
        if !event.all_day {
            let start =
                parse_time(&self.start.read(cx).value()).ok_or("Enter a start time as HH:MM.")?;
            let end =
                parse_time(&self.end.read(cx).value()).ok_or("Enter an end time as HH:MM.")?;
            if (start, end) != (event.start, event.end) {
                (event.start, event.end) = clamp_range(start, end);
            }
        }
        Ok(event)
    }
}

fn dropdown_option(option: Stateful<Div>, selected: bool) -> Stateful<Div> {
    option
        .w_full()
        .justify_start()
        .gap(px(8.))
        .h(px(DROPDOWN_ROW_HEIGHT))
        .rounded(px(6.))
        .text_size(px(13.))
        .text_color(rgb(TEXT_STRONG))
        .when(selected, |option| option.bg(rgb(HOVER)))
        .hover(|style| style.bg(rgb(HOVER)))
}

impl Editor {
    pub fn dismiss_picker(&mut self) -> bool {
        self.picker.take().is_some()
    }

    fn toggle_picker(&mut self, picker: Picker, cx: &mut Context<Self>) {
        if self.picker == Some(picker) {
            self.picker = None;
            cx.notify();
        } else {
            self.open_picker(picker, cx);
        }
    }

    fn open_picker(&mut self, picker: Picker, cx: &mut Context<Self>) {
        if self.picker == Some(picker) {
            return;
        }
        self.picker = Some(picker);
        if matches!(picker, Picker::Start | Picker::End) {
            let field = if picker == Picker::Start {
                &self.start
            } else {
                &self.end
            };
            let minute = parse_time(&field.read(cx).value()).unwrap_or(0);
            self.time_scroll.set_offset(point(
                px(0.),
                px(-DROPDOWN_ROW_HEIGHT * (minute / 30) as f32),
            ));
        }
        if picker == Picker::Date {
            self.month = self.event.date.with_day(1).unwrap_or(self.event.date);
        }
        cx.notify();
    }

    fn shift_month(&mut self, forward: bool, cx: &mut Context<Self>) {
        let next = if forward {
            self.month.checked_add_months(Months::new(1))
        } else {
            self.month.checked_sub_months(Months::new(1))
        };
        if let Some(next) = next {
            self.month = next;
        }
        cx.notify();
    }
}

fn picker_panel(width: f32) -> Div {
    div()
        .absolute()
        .top_full()
        .left_0()
        .mt(px(5.))
        .w(px(width))
        .bg(rgb(SURFACE))
        .text_size(px(12.))
        .text_color(rgb(TEXT_SECONDARY))
        .occlude()
}

fn dropdown_panel(width: f32) -> Div {
    picker_panel(width)
        .p(px(4.))
        .rounded(px(12.))
        .shadow(vec![dropdown_shadow()])
}

fn dropdown_shadow() -> BoxShadow {
    BoxShadow {
        color: rgba(EVENT_SHADOW).into(),
        offset: point(px(0.), px(4.)),
        blur_radius: px(6.),
        spread_radius: px(0.),
    }
}

fn swatch(calendar: Calendar) -> Div {
    div()
        .size(px(16.))
        .rounded(px(6.))
        .bg(rgb(calendar.color()))
}

fn editor_input(
    field: &Entity<InputState>,
    font_size: f32,
    line_height: f32,
    height: f32,
) -> Input {
    Input::new(field)
        .appearance(false)
        .p(px(0.))
        .h(px(height))
        .text_size(px(font_size))
        .line_height(px(line_height))
        .text_color(rgb(TEXT_SECONDARY))
        .font_weight(FontWeight::MEDIUM)
}

fn checkbox(checked: bool, start: Option<std::time::Instant>) -> Div {
    div()
        .size(px(16.))
        .flex_shrink_0()
        .rounded(px(6.))
        .when(checked, |el| el.bg(rgb(ACCENT)))
        .flex()
        .items_center()
        .justify_center()
        .child(if checked {
            icons::draw(Icon::Check, 12., TEXT_ON_SOLID, start)
        } else {
            icons::draw(Icon::Square, 16., TEXT_SECONDARY, start)
        })
}
