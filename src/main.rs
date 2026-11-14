mod app;
mod check;
mod document;
mod view;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

const APP_ID: &str = "io.github.ileskaa.PdfReader";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    if let Some(index) = args.iter().position(|arg| arg == "--check") {
        return check::run(args.get(index + 1).map(PathBuf::from));
    }

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_startup(app::actions::install_accels);
    app.connect_activate(activate);
    app.connect_open(|app, files, _| {
        if files.is_empty() {
            present(app, None);
        }
        for file in files {
            match file.path() {
                Some(path) => present(app, Some(path)),
                None => present_error(app, &format!("{} is not a local file.", file.uri())),
            }
        }
    });

    // gio's argv is UTF-8 only, so a non-UTF-8 path reaches connect_open lossy and fails
    // there with the usual dialog, rather than panicking out of std::env::args().
    let argv: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if app.run_with_args(&argv) == glib::ExitCode::SUCCESS {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// A bare launch restores the last session; a launch forwarded to an already running
/// instance just opens another empty window.
fn activate(application: &adw::Application) {
    if app::window::has_windows() {
        present(application, None);
        return;
    }
    let restored: Vec<PathBuf> = app::session::load()
        .into_iter()
        .filter(|path| path.is_file())
        .collect();
    if restored.is_empty() {
        present(application, None);
    }
    for path in restored {
        present(application, Some(path));
    }
}

fn present(application: &adw::Application, path: Option<PathBuf>) {
    let window = app::window::AppWindow::new(application);
    app::actions::install(&window);
    window.window.present();
    if let Some(path) = path {
        window.open(path);
    }
}

fn present_error(application: &adw::Application, message: &str) {
    let window = app::window::AppWindow::new(application);
    app::actions::install(&window);
    window.window.present();
    window.show_error(message);
}
