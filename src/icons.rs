use std::{sync::LazyLock, time::Instant};

use gpui::{prelude::*, *};
use lyon::tessellation::{LineCap, LineJoin};
use serde::Deserialize;

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub(crate) enum Icon {
    CalendarDays,
    Check,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    LogOut,
    Mail,
    Plus,
    Square,
    UserRound,
}

impl Icon {
    fn name(self) -> &'static str {
        match self {
            Self::CalendarDays => "CalendarDays",
            Self::Check => "Check",
            Self::ChevronDown => "ChevronDown",
            Self::ChevronLeft => "ChevronLeft",
            Self::ChevronRight => "ChevronRight",
            Self::LogOut => "LogOut",
            Self::Mail => "Mail",
            Self::Plus => "Plus",
            Self::Square => "Square",
            Self::UserRound => "UserRound",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    frame_rate: f32,
    icons: std::collections::HashMap<String, Vec<Layer>>,
}

#[derive(Deserialize)]
struct Layer {
    paths: Vec<Vec<Vec<f32>>>,
    frames: Vec<[f32; 6]>,
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/icons/calendar-icons.json"))
        .expect("valid bundled AnimateIcons data")
});
#[used]
static ICON_LICENSE: &str = include_str!("../assets/icons/LICENSE.txt");

struct Playback {
    start: Option<Instant>,
    focus: FocusHandle,
    _subscriptions: [Subscription; 2],
}

pub(crate) fn animate(
    button: Stateful<Div>,
    id: &'static str,
    icon: Icon,
    size: f32,
    color: impl Into<Option<u32>>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    with_animation(button, id, window, cx, |start| {
        draw(icon, size, color, start)
    })
    .0
}

pub(crate) fn with_animation(
    button: Stateful<Div>,
    id: &'static str,
    window: &mut Window,
    cx: &mut App,
    icon: impl FnOnce(Option<Instant>) -> Div,
) -> (Stateful<Div>, FocusHandle) {
    let state: Entity<Playback> = window.use_keyed_state(id, cx, |window, cx| {
        let focus = cx.focus_handle();
        let weak: WeakEntity<Playback> = cx.weak_entity();
        let began = window.on_focus_in(&focus, cx, {
            let weak = weak.clone();
            move |window, cx| {
                let _ = weak.update(cx, |playback, _| {
                    playback.start = Some(Instant::now());
                    window.refresh();
                });
            }
        });
        let ended = window.on_focus_out(&focus, cx, move |_, window, cx| {
            let _ = weak.update(cx, |playback, _| {
                playback.start = None;
                window.refresh();
            });
        });
        Playback {
            start: None,
            focus,
            _subscriptions: [began, ended],
        }
    });
    let focus = state.read(cx).focus.clone();
    let button = button
        .track_focus(&focus)
        .on_hover({
            let state = state.clone();
            move |active, window, cx| {
                state.update(cx, |playback, _| {
                    playback.start = active.then(Instant::now);
                });
                window.refresh();
            }
        })
        .child(icon(state.read(cx).start));
    (button, focus)
}

pub(crate) fn draw(
    icon: Icon,
    size: f32,
    color: impl Into<Option<u32>>,
    start: Option<Instant>,
) -> Div {
    let color = color.into();
    let layers = &CATALOG.icons[icon.name()];
    div().flex_shrink_0().size(px(size)).child(
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let color =
                    color.map_or_else(|| window.text_style().color, |color| rgb(color).into());
                let start = start.filter(|_| !reduced_motion());
                let frame = start
                    .map(|start| (start.elapsed().as_secs_f32() * CATALOG.frame_rate) as usize)
                    .unwrap_or(0);
                let factor = size / 24.;
                for layer in layers {
                    let [a, b, c, d, tx, ty] = layer
                        .frames
                        .get(frame)
                        .or_else(|| layer.frames.last())
                        .copied()
                        .unwrap_or([1., 0., 0., 1., 0., 0.]);
                    let at = |x, y| {
                        bounds.origin
                            + point(
                                px((a * x + c * y + tx) * factor),
                                px((b * x + d * y + ty) * factor),
                            )
                    };
                    let stroke = StrokeOptions::default()
                        .with_line_width(2. * factor * (a.hypot(b) * c.hypot(d)).sqrt())
                        .with_line_cap(LineCap::Round)
                        .with_line_join(LineJoin::Round);
                    let mut path =
                        PathBuilder::stroke(px(2. * factor)).with_style(PathStyle::Stroke(stroke));
                    for commands in &layer.paths {
                        for command in commands {
                            match command.as_slice() {
                                [] => path.close(),
                                &[x, y, 0.] => path.move_to(at(x, y)),
                                &[x, y, 1.] => path.line_to(at(x, y)),
                                &[x1, y1, x2, y2, x, y] => {
                                    path.cubic_bezier_to(at(x, y), at(x1, y1), at(x2, y2))
                                }
                                _ => unreachable!("invalid bundled icon path"),
                            }
                        }
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                }
                if start.is_some() && layers.iter().any(|layer| frame + 1 < layer.frames.len()) {
                    window.request_animation_frame();
                }
            },
        )
        .size_full(),
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn reduced_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn reduced_motion() -> bool {
    false
}
