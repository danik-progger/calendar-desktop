use gpui::{prelude::*, *};
use gpui_component::{Root, Theme, ThemeMode};

use super::Desktop;
use crate::style::{CLEAR_SURFACE, CONTROL_FOCUS, HOVER, SURFACE, TEXT_SECONDARY, TEXT_STRONG};

actions!(desktop, [Quit]);

pub(crate) fn run() {
    for path in [".env.local", ".env"] {
        if let Err(error) = dotenvy::from_path(path)
            && !error.not_found()
        {
            eprintln!("Could not load desktop environment file");
            return;
        }
    }
    Application::new().run(|cx| {
        // GPUI matches installed faces by weight, so register each Mulish instance separately.
        let fonts = [
            include_bytes!("../../assets/fonts/Mulish-Regular.ttf").as_slice(),
            include_bytes!("../../assets/fonts/Mulish-Medium.ttf").as_slice(),
            include_bytes!("../../assets/fonts/Mulish-SemiBold.ttf").as_slice(),
            include_bytes!("../../assets/fonts/Mulish-Bold.ttf").as_slice(),
        ];
        if let Err(error) = cx
            .text_system()
            .add_fonts(fonts.into_iter().map(Into::into).collect())
        {
            eprintln!("Could not load bundled Mulish font: {error}");
            cx.quit();
            return;
        }
        gpui_component::init(cx);
        cx.bind_keys([KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-q"
            } else {
                "ctrl-q"
            },
            Quit,
            None::<&str>,
        )]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(vec![Menu {
            name: "Calendar".into(),
            items: vec![MenuItem::action("Quit Calendar", Quit)],
        }]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(360.), px(420.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Calendar".into()),
                ..Default::default()
            }),
            app_id: Some("dev.calendar.gpui".into()),
            ..WindowOptions::default()
        };
        if let Err(error) = cx.open_window(options, |window, cx| {
            #[cfg(target_os = "macos")]
            set_surface_titlebar(window);
            Theme::change(ThemeMode::Light, Some(window), cx);
            Theme::global_mut(cx).font_family = "Mulish".into();
            let colors = &mut Theme::global_mut(cx).colors;
            colors.foreground = rgb(TEXT_STRONG).into();
            colors.muted_foreground = rgb(TEXT_SECONDARY).into();
            colors.ring = rgb(CONTROL_FOCUS).into();
            colors.popover = rgb(SURFACE).into();
            colors.popover_foreground = rgb(TEXT_STRONG).into();
            colors.border = rgba(CLEAR_SURFACE).into();
            colors.accent = rgb(HOVER).into();
            colors.accent_foreground = rgb(TEXT_STRONG).into();
            let content = cx.new(|cx| Desktop::new(window, cx));
            content.update(cx, |app, cx| app.connect(cx));
            cx.new(|cx| Root::new(content, window, cx))
        }) {
            eprintln!("Couldn’t open Calendar: {error}");
            cx.quit();
        }
        cx.activate(true);
    });
}

#[cfg(target_os = "macos")]
fn set_surface_titlebar(window: &Window) {
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSColor, NSView,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // GPUI owns the NSView for the lifetime of the borrowed window handle.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    if let Some(window) = view.window() {
        let appearance = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameAqua });
        window.setAppearance(appearance.as_deref());
        let surface = rgb(SURFACE);
        let color = NSColor::colorWithSRGBRed_green_blue_alpha(
            f64::from(surface.r),
            f64::from(surface.g),
            f64::from(surface.b),
            f64::from(surface.a),
        );
        window.setBackgroundColor(Some(&color));
        window.setTitlebarAppearsTransparent(true);
    }
}
