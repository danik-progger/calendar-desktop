# Calendar for desktop

Native Rust and GPUI calendar. Uses the same Better Auth service and Rust calendar API as web, iOS, and Android. Google login opens in the system browser with PKCE and a temporary loopback callback. The app exchanges Google's ID token with Better Auth, stores the session in the OS credential store (Keychain on macOS), and sends bearer-authenticated event requests directly to the calendar API.

## Setup

Create a Google OAuth client of type **Desktop app**, in the same Google Cloud project as the other platforms. Set its client ID as `GOOGLE_DESKTOP_CLIENT_ID` in both the BFF deployment and this app's environment. Redeploy the BFF after changing it. Google's desktop client credentials can include an installed-app client secret; if supplied, set it as `GOOGLE_DESKTOP_CLIENT_SECRET` in desktop only. Never put the web OAuth client secret or database credentials in this app.

Copy `.env.example` to `.env.local` and set:

```dotenv
AUTH_BASE_URL=https://<auth-service-domain>
CALENDAR_API_URL=https://<calendar-service-domain>
GOOGLE_DESKTOP_CLIENT_ID=<Desktop app OAuth client ID>
GOOGLE_DESKTOP_CLIENT_SECRET=<installed-app client secret, if supplied>
```

Run from this folder with a current Rust toolchain:

```sh
cargo run --locked
```

Run `./build.sh` to package the app with its launcher icon (`./build.sh --debug` for a development build). On macOS, open `target/release/Calendar.app`; on Linux, the output directory includes `Calendar.desktop` and its icon. Running the bare executable with `cargo run` does not use the macOS app bundle's Dock icon.

The app loads `.env.local`, then `.env`, without overriding exported variables. The origins must use HTTPS; loopback HTTP is allowed for local services. `AUTH_BASE_URL` points directly to the auth service, without `/api/auth`. Native HTTP requests must be allowed by the services' deployment protection.

Choose **Continue with Google**. Saved sessions are checked on startup; expired sessions require sign-in again. **Sign out** revokes the server session and removes the saved credential. No session tokens are saved to event files or environment files. Network operations run off the UI thread; failed writes retain the current calendar and draft. Changes use the same user-owned events as other connected clients. Realtime updates arrive automatically through the server event stream. Offline writes are not implemented.

Existing local `events.json` files are preserved and are not automatically uploaded.

Click an open time or all-day slot to create an event. Click an event to edit it, drag it to move it, or drag the bottom edge of a timed event to resize it. Use Today or the arrows to navigate. Horizontal scrolling moves by a day. Undo deletion from the notice or with Command-Z on macOS and Ctrl-Z elsewhere.

Desktop preserves event notes, calendar categories, and presentation metadata. The calendar shows supported events and reports how many it couldn’t display. Timed events that span local midnight and end after it, multi-day all-day events, and events with sub-minute timestamps are among those it may omit. Saving an ambiguous or unavailable local time fails explicitly.

Verified on macOS. Other desktop platforms have not been tested.

The [publish workflow](.github/workflows/publish.yml) builds an Apple silicon DMG, Windows x64 EXE installer, and Linux amd64 DEB on `main`. After all three succeed, it updates the desktop links on the landing page. Automatic releases skip if any of the three connection variables or `BLOB_READ_WRITE_TOKEN` is missing. Run the workflow manually on `main` to see missing settings reported as an error. See [release setup](../shared/RELEASES.md).

## Checks

```sh
cargo fmt --check
cargo check --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

The ignored `shared_services_support_calendar_mutations_and_session_revocation` test requires disposable loopback auth/calendar services, a fresh account's `DESKTOP_TEST_TOKEN`, and the environment settings above. Run it with `cargo test --locked shared_services_support_calendar_mutations_and_session_revocation -- --ignored`. It creates, edits, deletes, restores events, then revokes that test session. Regular tests cover OAuth callback state, PKCE, event conversion, and GPUI interactions against a loopback API. Live Google OAuth requires real Google credentials and account approval.

The icon catalog and license are bundled in `assets/icons`, so this repository builds without sibling directories. See [shared backend setup](../shared/BACKEND.md) for deployment and API details.

Signed-in calendars update automatically through the server’s authenticated event stream. Create, edit, delete, and undo changes propagate across devices on the same account. Reconnect refreshes changes missed while offline. Mobile clients reconnect on foreground; background APNs/FCM notifications and offline writes are not implemented. Server setup requires the notification migration and direct database URL described in [shared backend setup](../shared/BACKEND.md).
