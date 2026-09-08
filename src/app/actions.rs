use std::rc::Rc;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::app::window::AppWindow;
use crate::view::layout::ZoomMode;

const ACCELS: &[(&str, &[&str])] = &[
    ("win.open", &["<Control>o"]),
    ("win.find", &["<Control>f"]),
    ("win.find-close", &["Escape"]),
    ("win.find-next", &["<Control>g"]),
    ("win.find-previous", &["<Control><Shift>g"]),
    ("win.copy", &["<Control>c"]),
    ("win.select-all", &["<Control>a"]),
    ("win.zoom-in", &["<Control>plus", "<Control>equal"]),
    ("win.zoom-out", &["<Control>minus"]),
    ("win.zoom-reset", &["<Control>0"]),
    ("win.fit-width", &["<Control>1"]),
    ("win.fit-page", &["<Control>2"]),
    ("win.close", &["<Control>w"]),
];

/// Accelerators are application-scoped, so this belongs to startup, not to a window.
pub fn install_accels(app: &adw::Application) {
    for (action, accels) in ACCELS {
        app.set_accels_for_action(action, accels);
    }
}

pub fn install(window: &Rc<AppWindow>) {
    add(window, "open", |window| window.open_dialog());
    add(window, "find", |window| window.show_find());
    add(window, "find-close", |window| window.close_find());
    add(window, "find-next", |window| window.view.find_step(true));
    add(window, "find-previous", |window| {
        window.view.find_step(false)
    });
    add(window, "copy", |window| window.view.copy_selection());
    add(window, "select-all", |window| window.view.select_all());
    add(window, "zoom-in", |window| window.view.zoom_by(1.25));
    add(window, "zoom-out", |window| window.view.zoom_by(1.0 / 1.25));
    add(window, "zoom-reset", |window| {
        window.view.set_zoom_mode(ZoomMode::Fixed(1.0))
    });
    add(window, "fit-width", |window| {
        window.view.set_zoom_mode(ZoomMode::FitWidth)
    });
    add(window, "fit-page", |window| {
        window.view.set_zoom_mode(ZoomMode::FitPage)
    });
    add(window, "close", |window| window.window.close());
}

fn add<F>(window: &Rc<AppWindow>, name: &str, callback: F)
where
    F: Fn(&Rc<AppWindow>) + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(glib::clone!(
        #[weak]
        window,
        move |_, _| callback(&window)
    ));
    window.window.add_action(&action);
}
