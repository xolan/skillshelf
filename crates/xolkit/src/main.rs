// Release builds are GUI programs on Windows, so starting one opens no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod profiles;
mod settings;
mod shelf;
mod theme;

use gpui_kit::component::{TitleBar, WindowExt, notification::Notification};
use gpui_kit::*;

use crate::shelf::Shelf;

actions!(xolkit, [Quit]);

fn main() {
    let migrated = settings::migrate();
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let fonts = assets::FONTS.iter().map(|f| (*f).into()).collect();
            if let Err(e) = cx.text_system().add_fonts(fonts) {
                eprintln!("xolkit: could not load the embedded fonts: {e:#}");
            }
            shelf::bind_keys(cx);
            cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            // The app draws its own title bar, but Windows shows the window's title in
            // Alt+Tab and on the taskbar.
            let titlebar = TitlebarOptions {
                title: cfg!(windows).then(|| "xolkit".into()),
                ..TitleBar::title_bar_options()
            };
            let options = WindowOptions {
                titlebar: Some(titlebar),
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(760.)), cx)),
                window_min_size: Some(size(px(760.), px(480.))),
                app_id: Some("xolkit".into()),
                ..TitleBar::window_options()
            };
            let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
                theme::init(window, cx);
                cx.new(|cx| Shelf::new(window, cx))
            })
            .expect("failed to open window");
            if let Err(message) = migrated {
                eprintln!("xolkit: {message}");
                let _ = window.update(cx, |_, window, cx| {
                    // It says what to do, so it stays until closed.
                    window.push_notification(Notification::error(message).autohide(false), cx)
                });
            }
        });
}
