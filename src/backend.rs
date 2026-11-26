use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    time::{Duration as Wait, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Local, TimeZone, Timelike, Utc};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, Response},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{Calendar, Event};

#[derive(Clone)]
pub struct Config {
    pub auth: Url,
    pub calendar: Url,
    pub google_id: String,
    pub google_secret: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        fn configured(key: &str, release: Option<&str>) -> Result<String> {
            std::env::var(key)
                .ok()
                .or_else(|| release.map(str::to_owned))
                .with_context(|| format!("Set {key}"))
        }

        fn origin(key: &str, release: Option<&str>) -> Result<Url> {
            let value = Url::parse(&configured(key, release)?)?;
            let local = matches!(value.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
            ensure!(
                (value.scheme() == "https" || (local && value.scheme() == "http"))
                    && value.username().is_empty()
                    && value.password().is_none()
                    && value.path() == "/"
                    && value.query().is_none()
                    && value.fragment().is_none(),
                "{key} must be an HTTPS origin (loopback HTTP allowed)"
            );
            Ok(value)
        }
        let google_id = configured(
            "GOOGLE_DESKTOP_CLIENT_ID",
            option_env!("CALENDAR_RELEASE_GOOGLE_DESKTOP_CLIENT_ID"),
        )?;
        ensure!(!google_id.trim().is_empty(), "Set GOOGLE_DESKTOP_CLIENT_ID");
        Ok(Self {
            auth: origin(
                "AUTH_BASE_URL",
                option_env!("CALENDAR_RELEASE_AUTH_BASE_URL"),
            )?,
            calendar: origin(
                "CALENDAR_API_URL",
                option_env!("CALENDAR_RELEASE_CALENDAR_API_URL"),
            )?,
            google_id,
            google_secret: std::env::var("GOOGLE_DESKTOP_CLIENT_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }

    pub fn credential_key(&self) -> String {
        format!(
            "calendar-desktop:{}:{}",
            self.auth.origin().ascii_serialization(),
            self.calendar.origin().ascii_serialization()
        )
    }
}

#[derive(Clone)]
pub struct Api {
    pub config: Config,
    pub token: String,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Api { credentials: [redacted] }")
    }
}

#[derive(Debug)]
pub struct SessionExpired;
impl std::fmt::Display for SessionExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sign in again")
    }
}
impl std::error::Error for SessionExpired {}

#[derive(Debug, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
}

#[derive(Default)]
pub struct LoadedEvents {
    pub events: Vec<Event>,
    pub hidden_events: usize,
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Wait::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
fn checked(response: Response) -> Result<Response> {
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err(SessionExpired.into());
    }
    ensure!(
        response.status().is_success(),
        "Service returned HTTP {}",
        response.status()
    );
    Ok(response)
}
impl Api {
    pub fn watch(&self, mut changed: impl FnMut() -> bool) -> Result<()> {
        let response = checked(
            Client::builder()
                .timeout(Wait::from_secs(75))
                .redirect(reqwest::redirect::Policy::none())
                .build()?
                .get(self.config.calendar.join("/api/v1/events/stream")?)
                .bearer_auth(&self.token)
                .header("Accept", "text/event-stream")
                .send()?,
        )?;
        ensure!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/event-stream")),
            "Invalid event stream"
        );
        for line in BufReader::new(response).lines() {
            if line? == "data: {}" && !changed() {
                break;
            }
        }
        Ok(())
    }

    pub fn user(&self) -> Result<User> {
        #[derive(Deserialize)]
        struct Session {
            user: User,
        }
        let session: Option<Session> = checked(
            client()?
                .get(
                    self.config
                        .auth
                        .join("/api/auth/get-session?disableCookieCache=true")?,
                )
                .bearer_auth(&self.token)
                .send()?,
        )?
        .json()?;
        Ok(session.ok_or(SessionExpired)?.user)
    }
    pub fn logout(&self) -> Result<()> {
        checked(
            client()?
                .post(self.config.auth.join("/api/auth/sign-out")?)
                .bearer_auth(&self.token)
                .json(&serde_json::json!({}))
                .send()?,
        )?;
        Ok(())
    }
    pub fn load(&self) -> Result<LoadedEvents> {
        const PAGE_SIZE: usize = 1000;
        let mut loaded = LoadedEvents::default();
        let mut offset = 0;
        let client = client()?;
        loop {
            let rows: Vec<ApiEvent> = checked(
                client
                    .get(self.config.calendar.join("/api/v1/events")?)
                    .query(&[("limit", PAGE_SIZE), ("offset", offset)])
                    .bearer_auth(&self.token)
                    .send()?,
            )?
            .json()?;
            let count = rows.len();
            for row in rows {
                match row.into_event() {
                    Ok(event) => loaded.events.push(event),
                    Err(_) => loaded.hidden_events += 1,
                }
            }
            if count < PAGE_SIZE {
                return Ok(loaded);
            }
            offset += count;
        }
    }
    pub fn save(&self, event: &Event, existing: bool) -> Result<()> {
        let data = ApiEvent::from_event(event)?;
        let id = uuid::Uuid::parse_str(&event.id)?;
        let client = client()?;
        let request = if existing {
            client.put(self.config.calendar.join(&format!("/api/v1/events/{id}"))?)
        } else {
            client.post(self.config.calendar.join("/api/v1/events")?)
        };
        checked(request.bearer_auth(&self.token).json(&data).send()?)?;
        Ok(())
    }
    pub fn delete(&self, id: &str) -> Result<()> {
        let id = uuid::Uuid::parse_str(id)?;
        let response = client()?
            .delete(self.config.calendar.join(&format!("/api/v1/events/{id}"))?)
            .bearer_auth(&self.token)
            .send()?;
        if response.status() != StatusCode::NOT_FOUND {
            checked(response)?;
        }
        Ok(())
    }
}

pub struct Login {
    listener: TcpListener,
    state: String,
    verifier: String,
    nonce: String,
    redirect: String,
    pub url: Url,
}
impl Login {
    pub fn new(config: &Config) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let redirect = format!(
            "http://127.0.0.1:{}/callback",
            listener.local_addr()?.port()
        );
        let verifier = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let state = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let nonce = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let mut url = Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", config.google_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", "openid email profile"),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            (
                "code_challenge",
                URL_SAFE_NO_PAD
                    .encode(Sha256::digest(verifier.as_bytes()))
                    .as_str(),
            ),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ]);
        Ok(Self {
            listener,
            state,
            verifier,
            nonce,
            redirect,
            url,
        })
    }
    fn callback(&self, target: &str) -> Result<Option<String>> {
        ensure!(
            target.starts_with("/callback?") && !target.starts_with("//"),
            "Unexpected callback target"
        );
        let url = Url::parse(&self.redirect)?.join(target)?;
        ensure!(url.path() == "/callback", "Unexpected callback path");
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
        ensure!(
            pairs.get("state").map(|s| s.as_ref()) == Some(self.state.as_str()),
            "Invalid OAuth state"
        );
        if pairs.contains_key("error") {
            return Ok(None);
        }
        Ok(Some(
            pairs
                .get("code")
                .context("Missing authorization code")?
                .to_string(),
        ))
    }
    pub fn finish(
        self,
        config: Config,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(Api, User)> {
        let deadline = Instant::now() + Wait::from_secs(180);
        let code = loop {
            ensure!(
                !cancel.load(std::sync::atomic::Ordering::Relaxed),
                "Sign-in cancelled"
            );
            ensure!(Instant::now() < deadline, "Google sign-in timed out");
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Wait::from_secs(2)))?;
                    stream.set_write_timeout(Some(Wait::from_secs(2)))?;
                    let mut first = String::new();
                    if BufReader::new((&stream).take(8192))
                        .read_line(&mut first)
                        .is_err()
                    {
                        continue;
                    }
                    let mut request = first.split_whitespace();
                    let method = request.next();
                    let target = if method == Some("GET") {
                        request.next().unwrap_or("")
                    } else {
                        ""
                    };
                    let result = self.callback(target);
                    let (status, body) = if result.is_ok() {
                        ("200 OK", "Return to Calendar to finish signing in.")
                    } else {
                        (
                            "400 Bad Request",
                            "Invalid sign-in callback. Return to Calendar and try again.",
                        )
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    );
                    match result {
                        Ok(Some(code)) => break code,
                        Ok(None) => bail!("Google sign-in was cancelled"),
                        Err(_) => continue,
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Wait::from_millis(50))
                }
                Err(error) => return Err(error.into()),
            }
        };
        ensure!(
            !cancel.load(std::sync::atomic::Ordering::Relaxed),
            "Sign-in cancelled"
        );
        #[derive(Deserialize)]
        struct GoogleTokens {
            id_token: String,
        }
        let mut form = vec![
            ("code", code.as_str()),
            ("client_id", config.google_id.as_str()),
            ("redirect_uri", self.redirect.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", self.verifier.as_str()),
        ];
        if let Some(secret) = &config.google_secret {
            form.push(("client_secret", secret));
        }
        let tokens: GoogleTokens = checked(
            client()?
                .post("https://oauth2.googleapis.com/token")
                .form(&form)
                .send()?,
        )?
        .json()?;
        #[derive(Deserialize)]
        struct SignedIn {
            token: String,
            user: User,
        }
        let signed_in: SignedIn = checked(client()?.post(config.auth.join("/api/auth/sign-in/social")?)
            .json(&serde_json::json!({ "provider": "google", "idToken": { "token": tokens.id_token, "nonce": self.nonce } })).send()?)?.json()?;
        ensure!(!signed_in.token.is_empty(), "Missing session token");
        let api = Api {
            config,
            token: signed_in.token,
        };
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = api.logout();
            bail!("Sign-in cancelled");
        }
        Ok((api, signed_in.user))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    calendar_id: Calendar,
    presentation: String,
    duration_minute: Option<i32>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiEvent {
    id: String,
    title: String,
    description: Option<String>,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    all_day: bool,
    metadata: Metadata,
}
impl ApiEvent {
    fn into_event(self) -> Result<Event> {
        uuid::Uuid::parse_str(&self.id)?;
        ensure!(
            matches!(
                self.metadata.presentation.as_str(),
                "standard" | "focus" | "call"
            ),
            "Invalid event presentation"
        );
        ensure!(self.end_at > self.start_at, "Invalid event interval");
        let (date, start, end) = if self.all_day {
            ensure!(
                self.start_at.time() == chrono::NaiveTime::MIN
                    && self.end_at - self.start_at == Duration::days(1),
                "Unsupported multi-day event"
            );
            (self.start_at.date_naive(), 540, 585)
        } else {
            ensure!(
                self.start_at.second() == 0
                    && self.start_at.nanosecond() == 0
                    && self.end_at.second() == 0
                    && self.end_at.nanosecond() == 0,
                "Sub-minute events cannot be displayed"
            );
            let start = self.start_at.with_timezone(&Local);
            let end = self.end_at.with_timezone(&Local);
            let minute = |d: DateTime<Local>| (d.hour() * 60 + d.minute()) as i32;
            let end_minute = if end.date_naive() == start.date_naive() {
                minute(end)
            } else if start.date_naive().succ_opt() == Some(end.date_naive())
                && end.time() == chrono::NaiveTime::MIN
            {
                1440
            } else {
                bail!("Unsupported multi-day event")
            };
            (start.date_naive(), minute(start), end_minute)
        };
        let duration =
            self.metadata
                .duration_minute
                .unwrap_or(if self.all_day { 45 } else { end - start });
        let event = Event {
            id: self.id,
            title: self.title,
            notes: self.description.unwrap_or_default(),
            calendar: self.metadata.calendar_id,
            date,
            all_day: self.all_day,
            start,
            end,
            duration,
            presentation: self.metadata.presentation,
        };
        event.validate()?;
        Ok(event)
    }
    fn from_event(event: &Event) -> Result<Self> {
        let (start_at, end_at) = if event.all_day {
            (
                event.date.and_time(chrono::NaiveTime::MIN).and_utc(),
                event
                    .date
                    .succ_opt()
                    .context("Invalid event date")?
                    .and_time(chrono::NaiveTime::MIN)
                    .and_utc(),
            )
        } else {
            let midnight = event.date.and_time(chrono::NaiveTime::MIN);
            let instant = |minute| {
                Local
                    .from_local_datetime(&(midnight + Duration::minutes(i64::from(minute))))
                    .single()
                    .context("This time is ambiguous or unavailable due to daylight saving time")
                    .map(|d| d.with_timezone(&Utc))
            };
            (instant(event.start)?, instant(event.end)?)
        };
        Ok(Self {
            id: event.id.clone(),
            title: event.title.clone(),
            description: Some(event.notes.clone()),
            start_at,
            end_at,
            all_day: event.all_day,
            metadata: Metadata {
                calendar_id: event.calendar,
                presentation: event.presentation.clone(),
                duration_minute: Some(event.duration),
            },
        })
    }
}

#[cfg(test)]
pub(crate) fn events_response(events: &[Event]) -> String {
    let rows = events
        .iter()
        .map(ApiEvent::from_event)
        .collect::<Result<Vec<_>>>()
        .unwrap();
    serde_json::to_string(&rows).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            auth: Url::parse("http://127.0.0.1:3001").unwrap(),
            calendar: Url::parse("http://127.0.0.1:8080").unwrap(),
            google_id: "desktop-client.apps.googleusercontent.com".into(),
            google_secret: None,
        }
    }

    #[test]
    fn stream_authenticates_and_receives_multiple_pushes() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let mut config = config();
        config.calendar = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut input = BufReader::new(socket.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                input.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            assert!(request.starts_with("GET /api/v1/events/stream HTTP/1.1"));
            assert!(
                request
                    .to_lowercase()
                    .contains("authorization: bearer test-session")
            );
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n: heartbeat\n\nevent: change\ndata: {}\n\nevent: change\ndata: {}\n\n").unwrap();
        });
        let api = Api {
            config,
            token: "test-session".into(),
        };
        let mut updates = 0;
        api.watch(|| {
            updates += 1;
            true
        })?;
        server.join().unwrap();
        assert_eq!(updates, 2);
        Ok(())
    }

    #[test]
    fn callback_rejects_forged_state_and_unexpected_paths() -> Result<()> {
        let login = Login::new(&config())?;
        assert!(
            login
                .callback("/callback?state=forged&code=stolen")
                .is_err()
        );
        assert!(
            login
                .callback(&format!("/other?state={}&code=code", login.state))
                .is_err()
        );
        assert!(
            login
                .callback(&format!(
                    "https://attacker.test/callback?state={}&code=code",
                    login.state
                ))
                .is_err()
        );
        assert_eq!(
            login.callback(&format!("/callback?state={}&code=valid", login.state))?,
            Some("valid".into())
        );
        assert_eq!(
            login.callback(&format!(
                "/callback?state={}&error=access_denied",
                login.state
            ))?,
            None
        );
        let pairs: std::collections::HashMap<_, _> = login.url.query_pairs().collect();
        assert_eq!(pairs["code_challenge_method"], "S256");
        assert_eq!(
            pairs["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(login.verifier.as_bytes()))
        );
        assert!(!login.url.as_str().contains(&login.verifier));
        Ok(())
    }

    #[test]
    fn event_round_trip_preserves_notes_categories_and_presentation() -> Result<()> {
        let mut event = Event::new(
            chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap(),
            Some(540),
        );
        event.title = "Shared event".into();
        event.notes = "Preserve notes".into();
        event.calendar = Calendar::Health;
        event.presentation = "focus".into();
        assert_eq!(ApiEvent::from_event(&event)?.into_event()?, event);
        event.all_day = true;
        event.duration = 80;
        let restored = ApiEvent::from_event(&event)?.into_event()?;
        assert_eq!(restored.date, event.date);
        assert!(restored.all_day);
        assert_eq!(restored.duration, 80);
        assert_eq!(restored.presentation, "focus");
        Ok(())
    }

    #[test]
    fn unsupported_intervals_fail_instead_of_truncating() -> Result<()> {
        let mut event = Event::new(chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap(), None);
        event.title = "All day".into();
        let mut row = ApiEvent::from_event(&event)?;
        row.end_at += Duration::days(1);
        assert!(row.into_event().is_err());
        event.all_day = false;
        let mut row = ApiEvent::from_event(&event)?;
        row.start_at += Duration::seconds(1);
        assert!(row.into_event().is_err());
        Ok(())
    }

    #[test]
    fn load_advances_past_a_page_of_unsupported_events() -> Result<()> {
        let mut event = Event::new(chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap(), None);
        event.title = "Supported all-day event".into();
        let mut unsupported = ApiEvent::from_event(&event)?;
        unsupported.end_at += Duration::days(1);
        let first_page = serde_json::to_string(&vec![serde_json::to_value(unsupported)?; 1000])?;
        let second_page = events_response(std::slice::from_ref(&event));
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let mut config = config();
        config.calendar = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let server = std::thread::spawn(move || -> Result<()> {
            for (offset, body) in [(0, first_page), (1000, second_page)] {
                let (mut socket, _) = listener.accept()?;
                let mut request = BufReader::new(&socket);
                let mut line = String::new();
                request.read_line(&mut line)?;
                for header in request.lines() {
                    if header?.is_empty() {
                        break;
                    }
                }
                assert_eq!(
                    line,
                    format!("GET /api/v1/events?limit=1000&offset={offset} HTTP/1.1\r\n")
                );
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )?;
            }
            Ok(())
        });
        let loaded = Api {
            config,
            token: "test-session".into(),
        }
        .load()?;
        server.join().unwrap()?;
        assert_eq!(loaded.events, [event]);
        assert_eq!(loaded.hidden_events, 1000);
        Ok(())
    }

    #[test]
    fn credential_names_separate_backends() {
        let first = config();
        let mut second = first.clone();
        second.calendar = Url::parse("https://another-calendar.test").unwrap();
        assert_ne!(first.credential_key(), second.credential_key());
    }

    #[test]
    #[ignore = "requires disposable local auth/calendar services and DESKTOP_TEST_TOKEN"]
    fn shared_services_support_calendar_mutations_and_session_revocation() -> Result<()> {
        let config = Config::from_env()?;
        for url in [&config.auth, &config.calendar] {
            ensure!(
                matches!(url.host_str(), Some("localhost" | "127.0.0.1")),
                "Tests require loopback services"
            );
        }
        let api = Api {
            config,
            token: std::env::var("DESKTOP_TEST_TOKEN")?,
        };
        let user = api.user()?;
        ensure!(!user.id.is_empty());
        let mut store = crate::model::Store::remote(api.clone())?;
        ensure!(
            store.events.is_empty(),
            "Use a fresh disposable test account"
        );
        let mut event = Event::new(Local::now().date_naive(), Some(600));
        event.title = "Desktop integration".into();
        event.notes = "Preserved notes".into();
        event.presentation = "call".into();
        store.save(event.clone())?;
        assert_eq!(api.load()?.events, [event.clone()]);
        event.title = "Edited on desktop".into();
        store.save(event.clone())?;
        assert_eq!(api.load()?.events, [event.clone()]);
        event.all_day = true;
        event.duration = 80;
        store.save(event.clone())?;
        let all_day = api.load()?.events;
        assert!(all_day[0].all_day);
        assert_eq!(all_day[0].duration, 80);
        store.delete(&event.id)?;
        assert!(api.load()?.events.is_empty());
        store.undo()?;
        assert_eq!(api.load()?.events.len(), 1);
        api.logout()?;
        let previous = store.events.clone();
        assert!(store.delete(&event.id).is_err());
        assert_eq!(store.events, previous);
        assert!(
            api.user()
                .unwrap_err()
                .downcast_ref::<SessionExpired>()
                .is_some()
        );
        Ok(())
    }
}
