use anyhow::Result;
use futures_util::StreamExt;
use gpui::Context;

use super::{Desktop, UndoNotice};
use crate::{
    backend::{Api, Config, Login, SessionExpired, User},
    model::{Event, Store},
};

impl Desktop {
    pub fn cancel_login(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.login_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.network_task = None;
            self.busy = false;
            cx.notify();
        }
    }

    pub fn connect(&mut self, cx: &mut Context<Self>) {
        self.store = None;
        self.error = None;
        match Config::from_env() {
            Ok(config) => {
                self.config = Some(config);
                self.restore_session(cx);
            }
            Err(_) => {
                self.error = Some(
                    "Set AUTH_BASE_URL, CALENDAR_API_URL and GOOGLE_DESKTOP_CLIENT_ID, then restart Calendar.",
                )
            }
        }
        cx.notify();
    }

    fn request<T: Send + 'static>(
        &mut self,
        job: impl FnOnce() -> Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let work = cx.background_executor().spawn(async move { job() });
        self.network_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |app, cx| {
                app.busy = false;
                app.login_cancel = None;
                match result {
                    Ok(value) => done(app, value, cx),
                    Err(error) => {
                        if error.downcast_ref::<SessionExpired>().is_some() {
                            app.realtime_task = None;
                            app.expire_session(cx);
                        } else {
                            app.error = Some("Couldn’t complete the request. Check your connection and try again.");
                        }
                    }
                }
                app.flush_saves(cx);
                if app.sync_pending && app.account.is_some() {
                    app.refresh_remote(cx);
                }
                cx.notify();
            }).ok();
        }));
        cx.notify();
    }

    fn clear_session(&mut self) {
        self.sync_pending = false;
        self.save_queue.clear();
        self.confirmed_events = None;
        self.account = None;
        self.store = None;
        self.editor = None;
        self.undo_notice = None;
        self.drag = None;
    }

    fn expire_session(&mut self, cx: &mut Context<Self>) {
        self.clear_session();
        if let Some(config) = &self.config {
            cx.delete_credentials(&config.credential_key()).detach();
        }
        self.error = Some("Your session expired. Sign in again.");
    }

    pub fn restore_session(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(config) = self.config.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        let credentials = cx.read_credentials(&config.credential_key());
        let timeout = cx
            .background_executor()
            .timer(std::time::Duration::from_secs(5));
        self.network_task = Some(cx.spawn(async move |this, cx| {
            futures_util::pin_mut!(credentials, timeout);
            let credentials = match futures_util::future::select(credentials, timeout).await {
                futures_util::future::Either::Left((credentials, _)) => credentials,
                futures_util::future::Either::Right(_) => {
                    this.update(cx, |app, cx| {
                        app.busy = false;
                        app.error = Some(
                            "Saved session access timed out. Unlock Keychain or sign in again.",
                        );
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            this.update(cx, |app, cx| {
                app.busy = false;
                match credentials {
                    Ok(Some((_, bytes))) => match String::from_utf8(bytes) {
                        Ok(token) => {
                            let api = Api { config, token };
                            app.load_account(api, cx);
                        }
                        Err(_) => app.error = Some("Saved session is invalid. Sign in again."),
                    },
                    Ok(None) => {}
                    Err(_) => {
                        app.error = Some(
                            "Couldn’t read the system credential store. Unlock it and try again.",
                        )
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn load_account(&mut self, api: Api, cx: &mut Context<Self>) {
        self.request(
            move || {
                let user = api.user()?;
                let store = Store::remote(api.clone())?;
                Ok((api, user, store))
            },
            |app, (api, user, store), cx| {
                app.account = Some((api, user));
                app.store = Some(store);
                app.start_realtime(cx);
            },
            cx,
        );
    }

    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(config) = self.config.clone() else {
            return;
        };
        let login = match Login::new(&config) {
            Ok(login) => login,
            Err(_) => {
                self.error = Some("Couldn’t open the local sign-in callback. Try again.");
                cx.notify();
                return;
            }
        };
        cx.open_url(login.url.as_str());
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.login_cancel = Some(cancel.clone());
        self.request(
            move || login.finish(config, cancel),
            |app, (api, user), cx| {
                app.persist_session(api, user, cx);
            },
            cx,
        );
    }

    fn persist_session(&mut self, api: Api, user: User, cx: &mut Context<Self>) {
        self.busy = true;
        let saved =
            cx.write_credentials(&api.config.credential_key(), &user.id, api.token.as_bytes());
        self.network_task = Some(cx.spawn(async move |this, cx| {
            let result = saved.await;
            this.update(cx, |app, cx| {
                app.busy = false;
                if result.is_ok() { app.load_account(api, cx); }
                else { app.error = Some("Couldn’t save your session securely. Unlock the credential store and sign in again."); }
                cx.notify();
            }).ok();
        }));
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some((api, _)) = &self.account else {
            return;
        };
        let api = api.clone();
        self.request(move || api.logout(), |app, (), cx| {
            app.realtime_task = None;
            app.clear_session();
            if let Some(config) = &app.config {
                let deletion = cx.delete_credentials(&config.credential_key());
                app.busy = true;
                app.network_task = Some(cx.spawn(async move |this, cx| {
                    let result = deletion.await;
                    this.update(cx, |app, cx| {
                        app.busy = false;
                        if result.is_err() { app.error = Some("Session revoked, but the saved credential could not be removed."); }
                        cx.notify();
                    }).ok();
                }));
            }
        }, cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.account.is_some() {
            self.refresh_remote(cx);
        } else {
            self.restore_session(cx);
        }
    }

    fn start_realtime(&mut self, cx: &mut Context<Self>) {
        let Some((api, _)) = &self.account else {
            return;
        };
        let api = api.clone();
        let (sender, mut receiver) = futures_channel::mpsc::unbounded();
        std::thread::spawn(move || {
            let mut retry = 1;
            while !sender.is_closed() {
                let result = api.watch(|| {
                    retry = 1;
                    sender.unbounded_send(false).is_ok()
                });
                if result
                    .as_ref()
                    .err()
                    .is_some_and(|error| error.is::<SessionExpired>())
                {
                    let _ = sender.unbounded_send(true);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(retry));
                retry = (retry * 2).min(30);
            }
        });
        self.realtime_task = Some(cx.spawn(async move |this, cx| {
            while let Some(expired) = receiver.next().await {
                if this
                    .update(cx, |app, cx| {
                        if expired {
                            app.network_task = None;
                            app.busy = false;
                            app.expire_session(cx);
                        } else {
                            app.refresh_remote(cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn refresh_remote(&mut self, cx: &mut Context<Self>) {
        self.sync_pending = true;
        if self.busy {
            return;
        }
        let Some((api, _)) = &self.account else {
            return;
        };
        let api = api.clone();
        self.sync_pending = false;
        self.request(
            move || api.load(),
            |app, loaded, _| {
                if let Some(store) = &mut app.store {
                    store.events = loaded.events;
                    store.hidden_events = loaded.hidden_events;
                    app.rebase_saves();
                }
            },
            cx,
        );
    }

    pub fn save(&mut self, event: Event, cx: &mut Context<Self>) {
        let Some(store) = &mut self.store else {
            return;
        };
        if self.confirmed_events.is_none() {
            self.confirmed_events = Some(store.events.clone());
        }
        if store.preview_save(event.clone()).is_err() {
            if self.save_queue.is_empty() {
                self.confirmed_events = None;
            }
            self.error = Some("Couldn’t save your changes. Check the event details.");
            cx.notify();
            return;
        }
        self.save_queue.push_back((event, self.editor.take()));
        self.error = None;
        self.flush_saves(cx);
        cx.notify();
    }

    fn publish_saves(&mut self) {
        if let Some(events) = &self.confirmed_events
            && let Some(store) = &mut self.store
        {
            store.events = events.clone();
            for (event, _) in &self.save_queue {
                if store.preview_save(event.clone()).is_err() {
                    self.error = Some("Couldn’t display your pending changes.");
                }
            }
        }
    }

    fn rebase_saves(&mut self) {
        if self.confirmed_events.is_some() {
            self.confirmed_events = self.store.as_ref().map(|store| store.events.clone());
            self.publish_saves();
        }
    }

    fn flush_saves(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.save_queue.is_empty() {
            return;
        }
        let Some(mut store) = self.store.clone() else {
            return;
        };
        if let Some(events) = &self.confirmed_events {
            store.events = events.clone();
        }
        let Some((event, _)) = self.save_queue.front() else {
            return;
        };
        let event = event.clone();
        self.request(
            move || {
                let result = store.save(event);
                Ok((store, result))
            },
            move |app, (store, result), cx| {
                let saved = app.save_queue.pop_front();
                match result {
                    Ok(()) => app.confirmed_events = Some(store.events),
                    Err(error) if error.is::<SessionExpired>() => {
                        app.realtime_task = None;
                        app.expire_session(cx);
                        return;
                    }
                    Err(_) => {
                        app.error = Some(
                            "Couldn’t save your changes. Check your connection and try again.",
                        );
                        if app.editor.is_none() {
                            app.editor = saved
                                .filter(|(failed, _)| {
                                    !app.save_queue
                                        .iter()
                                        .any(|(event, _)| event.id == failed.id)
                                })
                                .and_then(|(_, editor)| editor);
                        }
                    }
                }
                app.publish_saves();
                if app.save_queue.is_empty() {
                    app.confirmed_events = None;
                }
            },
            cx,
        );
    }

    pub fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let id = editor.read(cx).event_id().to_owned();
        let Some(mut store) = self.store.clone() else {
            return;
        };
        self.request(
            move || {
                store.delete(&id)?;
                Ok(store)
            },
            move |app, store, _| {
                app.store = Some(store);
                app.rebase_saves();
                if app.editor.as_ref() == Some(&editor) {
                    app.editor = None;
                }
                app.undo_notice = Some(UndoNotice::Deleted);
            },
            cx,
        );
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(mut store) = self.store.clone() else {
            return;
        };
        self.request(
            move || match store.undo() {
                Ok(()) => Ok(Some(store)),
                Err(error) if error.is::<SessionExpired>() => Err(error),
                Err(_) => Ok(None),
            },
            |app, store, _| {
                if let Some(store) = store {
                    app.store = Some(store);
                    app.rebase_saves();
                    app.undo_notice = None;
                } else {
                    app.undo_notice = Some(UndoNotice::RestoreFailed);
                }
            },
            cx,
        );
    }
}
