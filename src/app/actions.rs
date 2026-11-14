use std::rc::Rc;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::app::tab::DocumentTab;
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
    add(window, "close", |window| window.close_selected_tab());
    add_tab(window, "find", |tab| tab.show_find());
    add_tab(window, "find-close", |tab| tab.close_find());
    add_tab(window, "find-next", |tab| tab.view.find_step(true));
    add_tab(window, "find-previous", |tab| tab.view.find_step(false));
    add_tab(window, "copy", |tab| tab.view.copy_selection());
    add_tab(window, "select-all", |tab| tab.view.select_all());
    add_tab(window, "zoom-in", |tab| tab.view.zoom_by(1.25));
    add_tab(window, "zoom-out", |tab| tab.view.zoom_by(1.0 / 1.25));
    add_tab(window, "zoom-reset", |tab| {
        tab.view.set_zoom_mode(ZoomMode::Fixed(1.0))
    });
    add_tab(window, "fit-width", |tab| {
        tab.view.set_zoom_mode(ZoomMode::FitWidth)
    });
    add_tab(window, "fit-page", |tab| {
        tab.view.set_zoom_mode(ZoomMode::FitPage)
    });
}

/// A document action, applied to the selected tab; with no tab open it does nothing.
fn add_tab<F>(window: &Rc<AppWindow>, name: &str, callback: F)
where
    F: Fn(&DocumentTab) + 'static,
{
    add(window, name, move |window| {
        if let Some(tab) = window.selected_tab() {
            callback(&tab);
        }
    });
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
