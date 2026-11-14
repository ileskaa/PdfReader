use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::app::session;
use crate::app::tab::{DocumentTab, TabEvent};
use crate::document::outline::OutlineItem;

thread_local! {
    static WINDOW: RefCell<Option<Rc<AppWindow>>> = const { RefCell::new(None) };
}

const APP_TITLE: &str = "PDF Reader";

/// The application's single window. Its header, outline sidebar and page label show the
/// selected tab; everything per document lives in `DocumentTab`.
pub struct AppWindow {
    pub window: adw::ApplicationWindow,
    pub split: adw::OverlaySplitView,
    tab_view: adw::TabView,
    stack: gtk4::Stack,
    page_label: gtk4::Label,
    title: adw::WindowTitle,
    outline_model: gtk4::StringList,
    outline_targets: RefCell<Vec<Option<usize>>>,
    sidebar_button: gtk4::ToggleButton,
    /// Tab state keyed by its page; `tab_view` owns the display order.
    tabs: RefCell<Vec<(adw::TabPage, Rc<DocumentTab>)>>,
}

impl AppWindow {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let title = adw::WindowTitle::new(APP_TITLE, "");
        let sidebar_button = gtk4::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle contents")
            .visible(false)
            .build();
        let page_label = gtk4::Label::builder().css_classes(["dim-label"]).build();

        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&sidebar_button);
        header.pack_start(&icon_button(
            "document-open-symbolic",
            "Open a document",
            "win.open",
        ));
        header.pack_end(&build_menu_button());
        header.pack_end(&icon_button(
            "system-search-symbolic",
            "Find in document",
            "win.find",
        ));
        header.pack_end(&icon_button("zoom-in-symbolic", "Zoom in", "win.zoom-in"));
        header.pack_end(&icon_button(
            "zoom-out-symbolic",
            "Zoom out",
            "win.zoom-out",
        ));
        header.pack_end(&page_label);

        let tab_view = adw::TabView::new();
        let tab_bar = adw::TabBar::builder()
            .view(&tab_view)
            .autohide(true)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("document-open-symbolic")
            .title("No Document Open")
            .child(
                &gtk4::Button::builder()
                    .label("Open…")
                    .halign(gtk4::Align::Center)
                    .css_classes(["pill", "suggested-action"])
                    .action_name("win.open")
                    .build(),
            )
            .build();
        let stack = gtk4::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&tab_view, Some("tabs"));
        stack.set_visible_child_name("empty");

        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.add_top_bar(&tab_bar);
        content.set_content(Some(&stack));

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
            .title(APP_TITLE)
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
            split,
            tab_view,
            stack,
            page_label,
            title,
            outline_model,
            outline_targets: RefCell::new(Vec::new()),
            sidebar_button,
            tabs: RefCell::new(Vec::new()),
        });
        this.wire(&outline_view);
        WINDOW.with_borrow_mut(|slot| *slot = Some(Rc::clone(&this)));
        this
    }

    fn wire(self: &Rc<Self>, outline_view: &gtk4::ListView) {
        // Closing the window is quitting, like closing a browser: the session it leaves behind
        // is what the next bare launch restores, so tearing down its tabs must not save.
        self.window.connect_close_request(|_| {
            WINDOW.with_borrow_mut(|slot| slot.take());
            glib::Propagation::Proceed
        });
        self.tab_view.connect_close_page(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, page| {
                this.tabs.borrow_mut().retain(|(other, _)| other != page);
                this.persist_session();
                glib::Propagation::Proceed
            }
        ));
        self.tab_view.connect_page_reordered(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _, _| this.persist_session()
        ));
        self.tab_view.connect_n_pages_notify(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |view| {
                let name = if view.n_pages() == 0 { "empty" } else { "tabs" };
                this.stack.set_visible_child_name(name);
                this.sync_header();
            }
        ));
        self.tab_view.connect_selected_page_notify(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                this.sync_header();
                if let Some(tab) = this.selected_tab() {
                    tab.view.grab_focus();
                }
            }
        ));

        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| match this.selected_tab() {
                Some(tab) => tab.on_find_key(key, modifiers),
                None => glib::Propagation::Proceed,
            }
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
                if let (Some(page), Some(tab)) = (target, this.selected_tab()) {
                    tab.view.goto_page(page);
                }
                if this.split.is_collapsed() {
                    this.split.set_show_sidebar(false);
                }
            }
        ));
    }

    /// Opens `path` in a new, selected tab, or selects the tab that already shows it.
    pub fn open(self: &Rc<Self>, path: PathBuf) {
        if let Some(page) = self.page_for_path(&path) {
            self.tab_view.set_selected_page(&page);
            return;
        }
        let tab = DocumentTab::new(path);
        tab.set_listener(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |tab, event| this.on_tab_event(tab, event)
        ));
        let page = self.tab_view.append(&tab.root);
        page.set_title(&tab.name());
        page.set_tooltip(&glib::markup_escape_text(&tab.path().to_string_lossy()));
        self.tabs.borrow_mut().push((page.clone(), Rc::clone(&tab)));
        tab.load();
        self.tab_view.set_selected_page(&page);
        self.persist_session();
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

    pub fn selected_tab(&self) -> Option<Rc<DocumentTab>> {
        let page = self.tab_view.selected_page()?;
        self.tab_for_page(&page)
    }

    pub fn select_first(&self) {
        if self.tab_view.n_pages() > 0 {
            self.tab_view.set_selected_page(&self.tab_view.nth_page(0));
        }
    }

    pub fn close_selected_tab(&self) {
        if let Some(page) = self.tab_view.selected_page() {
            self.tab_view.close_page(&page);
        }
    }

    pub fn show_error(&self, message: &str) {
        let dialog = adw::AlertDialog::new(Some("Cannot open document"), Some(message));
        dialog.add_response("ok", "OK");
        dialog.present(Some(&self.window));
    }

    fn on_tab_event(&self, tab: &DocumentTab, event: TabEvent) {
        let Some(page) = self.page_for_tab(tab) else {
            return;
        };
        if let TabEvent::Error(message) = event {
            // A tab that failed to open has nothing to show, so it goes away with the error.
            self.tab_view.close_page(&page);
            self.show_error(message);
            return;
        }
        if self.tab_view.selected_page().as_ref() == Some(&page) {
            self.sync_header();
        }
    }

    fn sync_header(&self) {
        let Some(tab) = self.selected_tab() else {
            self.title.set_title(APP_TITLE);
            self.title.set_subtitle("");
            self.page_label.set_label("");
            self.set_outline(&[]);
            return;
        };
        self.title.set_title(&tab.name());
        self.title.set_subtitle(&tab.path().to_string_lossy());
        let (page, total) = tab.page();
        let label = if total == 0 {
            String::new()
        } else {
            format!("{} of {total}", page + 1)
        };
        self.page_label.set_label(&label);
        self.set_outline(&tab.outline());
    }

    fn set_outline(&self, items: &[OutlineItem]) {
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

    fn tab_for_page(&self, page: &adw::TabPage) -> Option<Rc<DocumentTab>> {
        self.tabs
            .borrow()
            .iter()
            .find(|(other, _)| other == page)
            .map(|(_, tab)| Rc::clone(tab))
    }

    fn page_for_tab(&self, tab: &DocumentTab) -> Option<adw::TabPage> {
        self.tabs
            .borrow()
            .iter()
            .find(|(_, other)| std::ptr::eq(Rc::as_ptr(other), tab))
            .map(|(page, _)| page.clone())
    }

    fn page_for_path(&self, path: &Path) -> Option<adw::TabPage> {
        self.tabs
            .borrow()
            .iter()
            .find(|(_, tab)| tab.path() == path)
            .map(|(page, _)| page.clone())
    }

    /// Saves the open tabs in display order. A tab whose open fails closes itself, which
    /// saves again, so failed paths never linger in the session.
    fn persist_session(&self) {
        let paths: Vec<PathBuf> = (0..self.tab_view.n_pages())
            .filter_map(|index| self.tab_for_page(&self.tab_view.nth_page(index)))
            .map(|tab| tab.path().to_path_buf())
            .collect();
        session::save(&paths);
    }
}

/// The window, if one is open; there is at most one.
pub fn current() -> Option<Rc<AppWindow>> {
    WINDOW.with_borrow(|slot| slot.clone())
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
