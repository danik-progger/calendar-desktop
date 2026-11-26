use std::cmp::Reverse;

use super::CardFrame;
use crate::model::Event;

pub(super) fn layout_timed_events(
    mut events: Vec<&Event>,
    x: f32,
    width: f32,
    hour: f32,
) -> Vec<(&Event, CardFrame)> {
    events.sort_unstable_by_key(|event| (event.start, Reverse(event.end), &event.id));
    let mut cards = Vec::with_capacity(events.len());
    let mut group = Vec::new();
    let mut column_ends = Vec::new();
    let mut group_end = 0_f32;
    let top = |event: &Event| event.start as f32 / 60. * hour;
    let mut events = events.into_iter().peekable();

    while let Some(event) = events.next() {
        let y = top(event);
        let height = card_height(event, hour);
        let bottom = (event.end as f32 / 60. * hour).max(y + height);
        let column = column_ends
            .iter()
            .position(|end| *end <= y)
            .unwrap_or(column_ends.len());
        if column == column_ends.len() {
            column_ends.push(bottom);
        } else {
            column_ends[column] = bottom;
        }
        group_end = group_end.max(bottom);
        group.push((event, column, y, height));
        if events.peek().is_none_or(|next| top(next) >= group_end) {
            let step = width / column_ends.len() as f32;
            for (event, column, y, height) in group.drain(..) {
                let frame = CardFrame {
                    x: x + column as f32 * step,
                    y,
                    width: step - 4_f32.min(step / 2.),
                    height,
                    opacity: 1.,
                };
                cards.push((event, frame));
            }
            column_ends.clear();
        }
    }
    cards
}

pub(super) fn card_height(event: &Event, hour: f32) -> f32 {
    let duration = (event.end - event.start) as f32;
    if event.all_day {
        24.
    } else {
        (duration / 60. * hour).max(if duration >= 30. { 38. } else { 24. }) - 3.
    }
}

#[cfg(test)]
#[test]
fn timed_layout_preserves_overlap_and_preview_geometry() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
    let event = |id: &str, start, end| Event {
        id: id.into(),
        end,
        ..Event::new(date, Some(start))
    };
    let long = event("long", 540, 600);
    let short = event("short", 540, 545);
    let overlapping = event("overlapping", 545, 555);
    let adjacent = event("adjacent", 600, 615);
    let frames = |events| {
        layout_timed_events(events, 52., 120., 60.)
            .into_iter()
            .map(|(event, frame)| {
                let bounds = [frame.x, frame.y, frame.width, frame.height];
                (event.id.as_str(), bounds)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        frames(vec![&short, &adjacent, &overlapping, &long]),
        vec![
            ("long", [52., 540., 36., 57.]),
            ("short", [92., 540., 36., 21.]),
            ("overlapping", [132., 545., 36., 21.]),
            ("adjacent", [52., 600., 116., 21.]),
        ]
    );
    let preview = event("short", 600, 605);
    assert_eq!(
        frames(vec![&long, &overlapping, &adjacent, &preview]).last(),
        Some(&("short", [112., 600., 56., 21.]))
    );
}
