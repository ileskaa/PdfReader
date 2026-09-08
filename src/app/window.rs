use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::view::PageView;
use crate::view::clipboard;

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<AppWindow>>> = const { RefCell::new(Vec::new()) };
}

pub struct AppWindow {
    pub window: adw::ApplicationWindow,
    pub view: PageView,
    pub split: adw::OverlaySplitView,
    pub find_bar: gtk4::Revealer,
    pub find_entry: gtk4::SearchEntry,
    find_status: gtk4::Label,
    status_label: gtk4::Label,
    page_label: gtk4::Label,
    title: adw::WindowTitle,
    outline_model: gtk4::StringList,
    outline_targets: RefCell<Vec<Option<usize>>>,
    sidebar_button: gtk4::ToggleButton,
}

impl AppWindow {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let view = PageView::new();
        let scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Automatic)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .child(&view)
            .build();

        let title = adw::WindowTitle::new("PDF Reader", "");
        let sidebar_button = gtk4::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle contents")
            .visible(false)
            .build();
        let open_button = gtk4::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Open a document")
            .action_name("win.open")
            .build();
        let zoom_out = gtk4::Button::builder()
            .icon_name("zoom-out-symbolic")
            .tooltip_text("Zoom out")
            .action_name("win.zoom-out")
            .build();
        let zoom_in = gtk4::Button::builder()
            .icon_name("zoom-in-symbolic")
            .tooltip_text("Zoom in")
            .action_name("win.zoom-in")
            .build();
        let find_button = gtk4::Button::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("Find in document")
            .action_name("win.find")
            .build();
        let page_label = gtk4::Label::builder().css_classes(["dim-label"]).build();

        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&sidebar_button);
        header.pack_start(&open_button);
        header.pack_end(&build_menu_button());
        header.pack_end(&find_button);
        header.pack_end(&zoom_in);
        header.pack_end(&zoom_out);
        header.pack_end(&page_label);

        let find_entry = gtk4::SearchEntry::builder()
            .placeholder_text("Find in document")
            .hexpand(true)
            .build();
        let find_status = gtk4::Label::builder()
            .css_classes(["dim-label"])
            .width_chars(12)
            .xalign(1.0)
            .build();
        let find_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        find_box.append(&find_entry);
        find_box.append(&find_status);
        find_box.append(&icon_button(
            "go-up-symbolic",
            "Previous match",
            "win.find-previous",
        ));
        find_box.append(&icon_button(
            "go-down-symbolic",
            "Next match",
            "win.find-next",
        ));
        find_box.append(&icon_button(
            "window-close-symbolic",
            "Close",
            "win.find-close",
        ));
        let find_bar = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideUp)
            .child(&find_box)
            .build();

        let status_label = gtk4::Label::builder()
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::Middle)
            .margin_start(8)
            .margin_end(8)
            .margin_top(2)
            .margin_bottom(2)
            .build();

        let bottom = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        bottom.append(&status_label);
        bottom.append(&find_bar);

        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.set_content(Some(&scroller));
        content.add_bottom_bar(&bottom);

        let outline_model = gtk4::StringList::new(&[]);
        let outline_view = build_outline_view(&outline_model);
        let outline_scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vexpand(true)
            .child(&outline_view)
            .build();
        let sidebar = adw::ToolbarView::new();
        sidebar.add_top_bar(
            &adw::HeaderBar::builder()
                .title_widget(&adw::WindowTitle::new("Contents", ""))
                .show_end_title_buttons(false)
                .build(),
        );
        sidebar.set_content(Some(&outline_scroller));

        let split = adw::OverlaySplitView::builder()
            .sidebar(&sidebar)
            .content(&content)
            .max_sidebar_width(360.0)
            .show_sidebar(false)
            .build();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("PDF Reader")
            .default_width(1000)
            .default_height(760)
            .content(&split)
            .build();

        sidebar_button
            .bind_property("active", &split, "show-sidebar")
            .bidirectional()
            .sync_create()
            .build();

        let this = Rc::new(Self {
            window,
            view,
            split,
            find_bar,
            find_entry,
            find_status,
            status_label,
            page_label,
            title,
            outline_model,
            outline_targets: RefCell::new(Vec::new()),
            sidebar_button,
        });

        this.wire(&outline_view);
        WINDOWS.with_borrow_mut(|list| list.push(Rc::clone(&this)));
        this.window.connect_close_request(glib::clone!(
            #[weak(rename_to = this)]
            this,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_| {
                WINDOWS.with_borrow_mut(|list| list.retain(|other| !Rc::ptr_eq(other, &this)));
                glib::Propagation::Proceed
            }
        ));
        this
    }

    fn wire(self: &Rc<Self>, outline_view: &gtk4::ListView) {
        self.view.connect_loaded(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |outline| {
                this.set_outline(outline);
            }
        ));
        self.view.connect_error(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |message| {
                this.show_error(message);
            }
        ));
        self.view.connect_find_status(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |status| {
                this.find_status.set_label(status);
            }
        ));
        self.view.connect_status(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |status| {
                this.status_label.set_label(status.unwrap_or(""));
            }
        ));
        self.view.connect_page_changed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |(page, total)| {
                let label = if total == 0 {
                    String::new()
                } else {
                    format!("{} of {total}", page + 1)
                };
                this.page_label.set_label(&label);
            }
        ));

        self.find_entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |entry| {
                this.view.find(entry.text().as_str());
            }
        ));
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| this.on_find_key(key, modifiers)
        ));
        self.window.add_controller(keys);

        outline_view.connect_activate(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_, index| {
                let target = this
                    .outline_targets
                    .borrow()
                    .get(index as usize)
                    .copied()
                    .flatten();
                if let Some(page) = target {
                    this.view.goto_page(page);
                }
                if this.split.is_collapsed() {
                    this.split.set_show_sidebar(false);
                }
            }
        ));
    }

    pub fn open(&self, path: PathBuf) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.title.set_title(&name);
        self.title.set_subtitle(&path.to_string_lossy());
        self.view.load(path);
        let needle = self.find_entry.text();
        if self.find_bar.reveals_child() && !needle.is_empty() {
            self.view.find(needle.as_str());
        }
    }

    pub fn open_dialog(self: &Rc<Self>) {
        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("PDF documents"));
        filter.add_mime_type("application/pdf");
        filter.add_suffix("pdf");
        let filters = gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);

        let dialog = gtk4::FileDialog::builder()
            .title("Open Document")
            .filters(&filters)
            .default_filter(&filter)
            .modal(true)
            .build();
        dialog.open(
            Some(&self.window),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |result| {
                    let Ok(file) = result else {
                        return;
                    };
                    match file.path() {
                        Some(path) => this.open(path),
                        None => this.show_error(&format!("{} is not a local file.", file.uri())),
                    }
                }
            ),
        );
    }

    fn on_find_key(
        &self,
        key: gtk4::gdk::Key,
        modifiers: gtk4::gdk::ModifierType,
    ) -> glib::Propagation {
        use gtk4::gdk::Key;
        if !self.find_bar.reveals_child() {
            return glib::Propagation::Proceed;
        }
        let editing = self.entry_has_focus();
        let control = modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
        match key {
            Key::a if control && editing => self.find_entry.select_region(0, -1),
            Key::c if control && editing => self.copy_entry_selection(),
            Key::Escape if self.find_scope_has_focus() => self.close_find(),
            Key::Return | Key::KP_Enter | Key::ISO_Enter if editing => self
                .view
                .find_step(!modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK)),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    // A dialog is a descendant of the window, so an ungated Escape would close the find bar
    // instead of the dialog the user is looking at.
    fn find_scope_has_focus(&self) -> bool {
        gtk4::prelude::RootExt::focus(&self.window).is_some_and(|focused| {
            focused.is_ancestor(&self.find_bar)
                || focused == *self.view.upcast_ref::<gtk4::Widget>()
                || focused.is_ancestor(&self.view)
        })
    }

    fn entry_has_focus(&self) -> bool {
        gtk4::prelude::RootExt::focus(&self.window).is_some_and(|focused| {
            focused == *self.find_entry.upcast_ref::<gtk4::Widget>()
                || focused.is_ancestor(&self.find_entry)
        })
    }

    fn copy_entry_selection(&self) {
        let text = self.find_entry.text();
        let Some((start, end)) = self.find_entry.selection_bounds() else {
            return;
        };
        let selected: String = text
            .chars()
            .skip(start.max(0) as usize)
            .take((end - start).max(0) as usize)
            .collect();
        clipboard::set(&self.find_entry, &selected);
    }

    pub fn show_find(&self) {
        let revealed = self.find_bar.reveals_child();
        self.find_bar.set_reveal_child(true);
        self.find_entry.grab_focus();
        self.find_entry.select_region(0, -1);
        let needle = self.find_entry.text();
        if !revealed && !needle.is_empty() {
            self.view.find(needle.as_str());
        }
    }

    pub fn close_find(&self) {
        if !self.find_bar.reveals_child() {
            return;
        }
        self.find_bar.set_reveal_child(false);
        self.view.find_close();
        self.view.grab_focus();
    }

    fn set_outline(&self, items: &[crate::document::outline::OutlineItem]) {
        let labels: Vec<String> = items
            .iter()
            .map(|item| format!("{}{}", "    ".repeat(item.depth.min(6)), item.title))
            .collect();
        let targets: Vec<Option<usize>> = items.iter().map(|item| item.page).collect();
        let rows: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.outline_model
            .splice(0, self.outline_model.n_items(), &rows);
        *self.outline_targets.borrow_mut() = targets;
        self.sidebar_button.set_visible(!items.is_empty());
        if items.is_empty() {
            self.split.set_show_sidebar(false);
        }
    }

    pub fn show_error(&self, message: &str) {
        self.title.set_title("PDF Reader");
        self.title.set_subtitle("");
        let dialog = adw::AlertDialog::new(Some("Cannot open document"), Some(message));
        dialog.add_response("ok", "OK");
        dialog.present(Some(&self.window));
    }
}

fn icon_button(icon: &str, tooltip: &str, action: &str) -> gtk4::Button {
    gtk4::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .action_name(action)
        .build()
}

fn build_menu_button() -> gtk4::MenuButton {
    let menu = gio::Menu::new();
    let zoom = gio::Menu::new();
    zoom.append(Some("Zoom In"), Some("win.zoom-in"));
    zoom.append(Some("Zoom Out"), Some("win.zoom-out"));
    zoom.append(Some("Actual Size"), Some("win.zoom-reset"));
    zoom.append(Some("Fit Width"), Some("win.fit-width"));
    zoom.append(Some("Fit Page"), Some("win.fit-page"));
    menu.append_section(None, &zoom);
    let edit = gio::Menu::new();
    edit.append(Some("Select All"), Some("win.select-all"));
    edit.append(Some("Copy"), Some("win.copy"));
    menu.append_section(None, &edit);
    gtk4::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Main menu")
        .menu_model(&menu)
        .build()
}

fn build_outline_view(model: &gtk4::StringList) -> gtk4::ListView {
    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let label = gtk4::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        item.downcast_ref::<gtk4::ListItem>()
            .unwrap()
            .set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk4::ListItem>().unwrap();
        let Some(text) = item.item().and_downcast::<gtk4::StringObject>() else {
            return;
        };
        let Some(label) = item.child().and_downcast::<gtk4::Label>() else {
            return;
        };
        label.set_label(&text.string());
    });
    let selection = gtk4::SingleSelection::new(Some(model.clone()));
    gtk4::ListView::builder()
        .model(&selection)
        .factory(&factory)
        .single_click_activate(true)
        .build()
}
