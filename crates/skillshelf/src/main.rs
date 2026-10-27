// Release builds are GUI programs on Windows, so starting one opens no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod settings;
mod shelf;
mod theme;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::shelf::Shelf;

actions!(skillshelf, [Quit]);

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let fonts = assets::FONTS.iter().map(|f| (*f).into()).collect();
            if let Err(e) = cx.text_system().add_fonts(fonts) {
                eprintln!("skillshelf: could not load the embedded fonts: {e:#}");
            }
            shelf::bind_keys(cx);
            cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            // The app draws its own title bar, but Windows shows the window's title in
            // Alt+Tab and on the taskbar.
            let titlebar = TitlebarOptions {
                title: cfg!(windows).then(|| "skillshelf".into()),
                ..TitleBar::title_bar_options()
            };
            let options = WindowOptions {
                titlebar: Some(titlebar),
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(760.)), cx)),
                window_min_size: Some(size(px(760.), px(480.))),
                app_id: Some("skillshelf".into()),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                theme::init(window, cx);
                cx.new(|cx| Shelf::new(window, cx))
            })
            .expect("failed to open window");
        });
}
