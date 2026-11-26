use std::{
    sync::mpsc::{Receiver, channel},
    time::Duration as Wait,
};

use chrono::Duration;
use gpui::{
    AppContext, Bounds, KeyUpEvent, Keystroke, Modifiers, MouseButton, TestAppContext,
    VisualTestContext, WindowBounds, WindowOptions, point, px, size,
};

use gpui_component::Root;

use super::{Desktop, UndoNotice};
use crate::{
    backend,
    model::{Event, Store},
};

fn calendar_server(
    events: &[Event],
    write_statuses: &[&'static str],
) -> (backend::Api, Receiver<Vec<String>>) {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let rows = backend::events_response(events);
    let statuses = write_statuses.to_vec();
    let (send_requests, requests_received) = channel();
    std::thread::spawn(move || {
        let mut requests = Vec::new();
        for status in std::iter::once("200 OK").chain(statuses) {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let method = first.split_whitespace().next().unwrap();
            requests.push(method.to_owned());
            let mut length = 0;
            for line in reader.by_ref().lines() {
                let line = line.unwrap();
                if line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body = if method == "GET" && status == "200 OK" {
                rows.as_str()
            } else {
                ""
            };
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        send_requests.send(requests).ok();
    });
    let api = backend::Api {
        config: backend::Config {
            auth: origin.parse().unwrap(),
            calendar: origin.parse().unwrap(),
            google_id: String::new(),
            google_secret: None,
        },
        token: "test".into(),
    };
    (api, requests_received)
}

#[test]
fn invalid_previews_leave_events_unchanged() {
    let mut event = Event::new(
        chrono::NaiveDate::from_ymd_opt(2026, 11, 23).unwrap(),
        Some(600),
    );
    event.title = "Planning".into();
    let (api, server) = calendar_server(&[event.clone()], &[]);
    let mut store = Store::remote(api).unwrap();
    let invalid: [fn(&mut Event); 10] = [
        |event| event.id.clear(),
        |event| event.title = "  ".into(),
        |event| event.duration = 0,
        |event| event.duration = 1441,
        |event| event.start = -1,
        |event| event.start = i32::MAX,
        |event| event.end = 1441,
        |event| event.end = i32::MIN,
        |event| event.duration = i32::MIN,
        |event| event.end = event.start,
    ];
    for invalidate in invalid {
        let mut edited = event.clone();
        invalidate(&mut edited);
        assert!(store.preview_save(edited).is_err());
        assert_eq!(store.events.as_slice(), std::slice::from_ref(&event));
    }
    assert_eq!(server.recv_timeout(Wait::from_secs(5)).unwrap(), ["GET"]);
}

#[test]
fn short_events_allow_saving_a_new_event_and_editing_the_short_event() {
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
    let mut short = Event::new(day, Some(600));
    short.title = "Check-in".into();
    short.end = 602;
    short.duration = 2;
    let (api, server) = calendar_server(&[short.clone()], &["204 No Content"; 2]);
    let mut store = Store::remote(api).unwrap();
    let mut lunch = Event::new(day, Some(720));
    lunch.title = "Lunch".into();
    short.title = "Edited check-in".into();
    for event in [lunch.clone(), short.clone()] {
        let mut preview = store.clone();
        preview.preview_save(event.clone()).unwrap();
        store.save(event).unwrap();
        assert_eq!(store.events, preview.events);
    }
    assert_eq!(store.events, [short, lunch]);
    assert_eq!(
        server.recv_timeout(Wait::from_secs(5)).unwrap(),
        ["GET", "POST", "PUT"]
    );
}

#[test]
fn undo_keeps_an_event_already_restored_by_another_client() {
    let mut event = Event::new(
        chrono::NaiveDate::from_ymd_opt(2026, 11, 23).unwrap(),
        Some(600),
    );
    event.title = "Planning".into();
    let (api, server) = calendar_server(&[event.clone()], &["204 No Content"]);
    let mut store = Store::remote(api).unwrap();
    store.delete(&event.id).unwrap();
    assert!(store.events.is_empty());

    event.title = "Updated after restore".into();
    store.events.push(event.clone());
    store.undo().unwrap();
    assert_eq!(store.events, [event]);
    assert_eq!(
        server.recv_timeout(Wait::from_secs(5)).unwrap(),
        ["GET", "DELETE"]
    );
}

#[gpui::test]
fn remote_saves_are_optimistic_and_queued(cx: &mut TestAppContext) {
    let (api, server) = calendar_server(
        &[],
        &[
            "500 Internal Server Error",
            "204 No Content",
            "500 Internal Server Error",
            "401 Unauthorized",
        ],
    );
    let store = Store::remote(api.clone()).unwrap();
    cx.update(gpui_component::init);
    let mut calendar = None;
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(|cx| {
                let mut app = Desktop::new(window, cx);
                app.account = Some((
                    api,
                    backend::User {
                        id: "test".into(),
                        email: "test@example.test".into(),
                    },
                ));
                app.store = Some(store);
                app
            });
            calendar = Some(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .unwrap()
    });
    let calendar = calendar.unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        calendar.update(cx, |app, cx| {
            let mut event = Event::new(app.anchor, Some(600));
            event.title = "First".into();
            app.show_editor(event.clone(), false, window, cx);
            app.save_editor(cx);
            assert!(app.editor.is_none());
            assert_eq!(app.store.as_ref().unwrap().events, vec![event.clone()]);
            assert!(app.busy);
            event.title = "Second".into();
            app.show_editor(event.clone(), false, window, cx);
            app.save_editor(cx);
            assert_eq!(app.store.as_ref().unwrap().events, vec![event]);
            assert_eq!(app.save_queue.len(), 2);
        });
        window.draw(cx).clear();
    });
    assert!(visual.debug_bounds("account-button").is_some());
    visual.run_until_parked();
    cx.read(|cx| {
        let app = calendar.read(cx);
        assert!(!app.busy);
        assert!(app.editor.is_none());
        assert!(app.save_queue.is_empty());
        assert_eq!(app.store.as_ref().unwrap().events[0].title, "Second");
    });
    visual.update(|window, cx| {
        calendar.update(cx, |app, cx| {
            let mut event = app.store.as_ref().unwrap().events[0].clone();
            event.title = "Failed draft".into();
            app.show_editor(event, true, window, cx);
            app.save_editor(cx);
            assert!(app.editor.is_none());
            assert_eq!(app.store.as_ref().unwrap().events[0].title, "Failed draft");
        });
    });
    visual.run_until_parked();
    cx.update(|cx| {
        calendar.update(cx, |app, cx| {
            assert_eq!(app.store.as_ref().unwrap().events[0].title, "Second");
            let editor = app.editor.as_ref().unwrap().read(cx);
            assert_eq!(editor.draft(cx).unwrap().title, "Failed draft");
            assert!(app.error.is_some());
            app.save_editor(cx);
            let mut event = app.store.as_ref().unwrap().events[0].clone();
            event.title = "Waiting behind expired write".into();
            app.save(event, cx);
            app.undo_notice = Some(UndoNotice::Deleted);
            assert_eq!(app.save_queue.len(), 2);
        })
    });
    visual.run_until_parked();
    cx.read(|cx| {
        let app = calendar.read(cx);
        assert!(!app.busy);
        assert!(app.account.is_none());
        assert!(app.store.is_none());
        assert!(app.editor.is_none());
        assert!(app.undo_notice.is_none());
        assert!(app.confirmed_events.is_none());
        assert!(app.save_queue.is_empty());
        assert_eq!(app.error, Some("Your session expired. Sign in again."));
    });
    assert_eq!(
        server.recv_timeout(Wait::from_secs(5)).unwrap(),
        ["GET", "POST", "POST", "PUT", "PUT"]
    );
}

#[gpui::test]
fn pointer_input_hides_keyboard_focus_on_sign_in_screen(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let mut desktop = None;
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(|cx| Desktop::new(window, cx));
            desktop = Some(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .unwrap()
    });
    let desktop = desktop.unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| window.draw(cx).clear());

    visual.simulate_keystrokes("tab");
    assert!(cx.read(|cx| desktop.read(cx).keyboard_focus_visible));
    visual.simulate_click(point(px(20.), px(100.)), Modifiers::default());
    assert!(!cx.read(|cx| desktop.read(cx).keyboard_focus_visible));
}

#[gpui::test]
fn account_menu_supports_pointer_and_keyboard(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let mut calendar = None;
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(|cx| {
                let mut app = Desktop::new(window, cx);
                app.account = Some((
                    backend::Api {
                        config: backend::Config {
                            auth: "https://auth.example.test".parse().unwrap(),
                            calendar: "https://calendar.example.test".parse().unwrap(),
                            google_id: String::new(),
                            google_secret: None,
                        },
                        token: String::new(),
                    },
                    backend::User {
                        id: "test-user".into(),
                        email: "test@example.test".into(),
                    },
                ));
                app
            });
            calendar = Some(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .unwrap()
    });
    let calendar = calendar.unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| window.draw(cx).clear());
    let button = visual.debug_bounds("account-button").unwrap().center();
    let state = |cx: &TestAppContext| {
        cx.read(|cx| {
            let calendar = calendar.read(cx);
            (
                calendar.account_menu.is_some(),
                calendar.keyboard_focus_visible,
            )
        })
    };

    visual.simulate_click(button, Modifiers::default());
    assert_eq!(state(cx), (true, false));
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_click(button, Modifiers::default());
    assert_eq!(state(cx), (false, false));
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_click(button, Modifiers::default());
    assert_eq!(state(cx), (true, false));
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_keystrokes("escape");
    assert_eq!(state(cx), (false, true));
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("enter").unwrap(),
    });
    assert_eq!(state(cx), (true, true));
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_click(button, Modifiers::default());
    assert_eq!(state(cx), (false, false));
}

#[gpui::test]
fn clicks_move_resize_and_save(cx: &mut TestAppContext) {
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
    let mut timed = Event::new(day, Some(570));
    timed.title = "Deep work".into();
    timed.end = 675;
    let event_id = timed.id.clone();
    let mut all_day = Event::new(day, None);
    all_day.title = "Birthday".into();
    let (api, server) = calendar_server(
        &[timed, all_day],
        &[
            "200 OK",
            "204 No Content",
            "204 No Content",
            "204 No Content",
            "204 No Content",
            "500 Internal Server Error",
            "204 No Content",
        ],
    );
    let store = Store::remote(api.clone()).unwrap();
    cx.update(gpui_component::init);
    let (calendar, window) = cx.update(|cx| {
        let mut calendar = None;
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1280.), px(820.)),
                    ))),
                    ..WindowOptions::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| {
                        let mut app = Desktop::new(window, cx);
                        app.anchor = day;
                        app.account = Some((
                            api,
                            backend::User {
                                id: "test".into(),
                                email: "test@example.test".into(),
                            },
                        ));
                        app.store = Some(store);
                        app
                    });
                    calendar = Some(view.clone());
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .unwrap();
        (calendar.unwrap(), window)
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        window.draw(cx).clear();
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        window.draw(cx).clear();
    });
    let scroll = cx.read(|cx| calendar.read(cx).scroll.clone());
    let bounds = scroll.bounds();
    let hour = f32::from(bounds.size.height) / 8.;
    assert!(hour > 30.);
    scroll.set_offset(point(px(0.), px(-9. * hour)));
    visual.update(|window, cx| {
        window.draw(cx).clear();
    });
    let start = point(bounds.left() + px(160.), bounds.top() + px(hour * 0.75));
    visual.simulate_keystrokes("tab");
    assert!(cx.read(|cx| calendar.read(cx).keyboard_focus_visible));
    visual.simulate_click(start, Modifiers::default());
    assert!(!cx.read(|cx| calendar.read(cx).keyboard_focus_visible));
    assert_eq!(
        cx.read(|cx| calendar
            .read(cx)
            .editor
            .as_ref()
            .map(|draft| draft.read(cx).event_id().to_owned())),
        Some(event_id.clone())
    );
    visual.update(|window, cx| window.draw(cx).clear());
    let modal = bounds.left() + px(430.);
    let midpoint_y = bounds.bottom() - px(410.);
    visual.simulate_click(
        point(modal + px(200.), midpoint_y - px(73.)),
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(visual.debug_bounds("time-picker").is_some());
    visual.simulate_click(
        point(modal + px(196.), midpoint_y - px(30.)),
        Modifiers::default(),
    );
    visual.update(|window, cx| window.draw(cx).clear());
    let editor = cx.read(|cx| calendar.read(cx).editor.as_ref().unwrap().downgrade());
    visual.simulate_click(
        point(modal + px(307.), midpoint_y + px(134.)),
        Modifiers::default(),
    );
    assert!(cx.read(|cx| calendar.read(cx).editor.is_none()));
    assert!(editor.upgrade().is_none(), "dismissed editor is released");
    visual.update(|window, cx| window.draw(cx).clear());
    let dest = start + point(px(409.), px(hour * 2.));
    visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(dest, MouseButton::Left, Modifiers::default());
    cx.update(|cx| calendar.update(cx, |app, cx| app.refresh(cx)));
    visual.run_until_parked();
    visual.simulate_mouse_up(dest, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    let (start_minute, end) = cx.read(|cx| {
        let app = calendar.read(cx);
        let event = app
            .store
            .as_ref()
            .unwrap()
            .events
            .iter()
            .find(|e| e.id == event_id)
            .unwrap();
        assert_eq!(event.date, app.anchor + Duration::days(1));
        (event.start, event.end)
    });
    assert_eq!((start_minute, end), (690, 795));
    let resize = point(dest.x, bounds.top() + px(hour * 4.25 - 8.));
    let extended = resize + point(px(0.), px(hour));
    visual.simulate_mouse_down(resize, MouseButton::Left, Modifiers::default());
    assert!(
        cx.read(|cx| calendar
            .read(cx)
            .drag
            .as_ref()
            .is_some_and(|drag| drag.is_resize())),
        "resize handle received mouse down"
    );
    visual.simulate_mouse_move(extended, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(extended, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        cx.read(|cx| calendar
            .read(cx)
            .store
            .as_ref()
            .unwrap()
            .events
            .iter()
            .find(|e| e.id == event_id)
            .unwrap()
            .end),
        855
    );
    let sample = point(bounds.left() + px(280.), bounds.top() + px(hour * 0.2));
    visual.simulate_click(
        point(bounds.left() + px(26.), px(77.)),
        Modifiers::default(),
    );
    visual.simulate_keystrokes("enter");
    assert_eq!(
        cx.read(|cx| calendar.read(cx).error),
        Some("Add a title to save this event.")
    );
    visual.simulate_input("Desktop parity");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(cx.read(|cx| {
        calendar
            .read(cx)
            .store
            .as_ref()
            .unwrap()
            .events
            .iter()
            .any(|event| {
                event.title == "Desktop parity"
                    && event.date == calendar.read(cx).anchor
                    && event.start == 540
            })
    }));
    let before_delete = cx.read(|cx| calendar.read(cx).store.as_ref().unwrap().events.clone());
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_click(sample, Modifiers::default());
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_click(
        point(modal + px(44.), midpoint_y + px(134.)),
        Modifiers::default(),
    );
    visual.run_until_parked();
    assert!(cx.read(|cx| matches!(calendar.read(cx).undo_notice, Some(UndoNotice::Deleted))));
    visual.update(|window, cx| window.draw(cx).clear());
    let undo = point(bounds.right() - px(55.), bounds.bottom() - px(45.));
    visual.simulate_click(undo, Modifiers::default());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    assert!(
        visual
            .debug_bounds("Couldn’t restore. Try again.")
            .is_some()
    );
    assert!(cx.read(
        |cx| calendar.read(cx).store.as_ref().unwrap().events.len() == before_delete.len() - 1
    ));
    visual.simulate_click(undo, Modifiers::default());
    visual.run_until_parked();
    cx.read(|cx| {
        let app = calendar.read(cx);
        assert!(app.undo_notice.is_none());
        assert_eq!(app.store.as_ref().unwrap().events, before_delete);
    });
    assert_eq!(
        server.recv_timeout(Wait::from_secs(5)).unwrap(),
        ["GET", "GET", "PUT", "PUT", "POST", "DELETE", "POST", "POST"]
    );
}
