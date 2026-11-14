use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

use crate::document::outline::OutlineItem;
use crate::view::PageView;
use crate::view::clipboard;

/// What a tab reports to the window that holds it.
pub enum TabEvent<'a> {
    Outline,
    Page,
    Error(&'a str),
}

type Listener = Box<dyn Fn(&DocumentTab, TabEvent)>;

/// One open document: its view, status line and find bar. Every open gets a fresh tab, so
/// the path never changes for the life of the tab.
pub struct DocumentTab {
    pub root: gtk4::Box,
    pub view: PageView,
    path: PathBuf,
    find_bar: gtk4::Revealer,
    find_entry: gtk4::SearchEntry,
    find_status: gtk4::Label,
    status_label: gtk4::Label,
    outline: RefCell<Vec<OutlineItem>>,
    page: Cell<(usize, usize)>,
    listener: RefCell<Option<Listener>>,
}

impl DocumentTab {
    pub fn new(path: PathBuf) -> Rc<Self> {
        let view = PageView::new();
        let scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Automatic)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .child(&view)
            .build();

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

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.append(&scroller);
        root.append(&status_label);
        root.append(&find_bar);

        let this = Rc::new(Self {
            root,
            view,
            path,
            find_bar,
            find_entry,
            find_status,
            status_label,
            outline: RefCell::new(Vec::new()),
            page: Cell::new((0, 0)),
            listener: RefCell::new(None),
        });
        this.wire();
        this
    }

    pub fn set_listener(&self, listener: impl Fn(&DocumentTab, TabEvent) + 'static) {
        self.listener.replace(Some(Box::new(listener)));
    }

    pub fn load(&self) {
        self.view.load(self.path.clone());
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    pub fn outline(&self) -> std::cell::Ref<'_, Vec<OutlineItem>> {
        self.outline.borrow()
    }

    /// The current page (zero-based) and the page count; the count is zero until loaded.
    pub fn page(&self) -> (usize, usize) {
        self.page.get()
    }

    fn emit(&self, event: TabEvent) {
        if let Some(listener) = self.listener.borrow().as_ref() {
            listener(self, event);
        }
    }

    fn wire(self: &Rc<Self>) {
        self.view.connect_loaded(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |outline| {
                this.outline.replace(outline.to_vec());
                this.emit(TabEvent::Outline);
            }
        ));
        self.view.connect_error(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |message| this.emit(TabEvent::Error(message))
        ));
        self.view.connect_find_status(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |status| this.find_status.set_label(status)
        ));
        self.view.connect_status(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |status| this.status_label.set_label(status.unwrap_or(""))
        ));
        self.view.connect_page_changed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |page| {
                this.page.set(page);
                this.emit(TabEvent::Page);
            }
        ));
        self.find_entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |entry| this.view.find(entry.text().as_str())
        ));
    }

    pub fn on_find_key(
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

    fn focused(&self) -> Option<gtk4::Widget> {
        self.root.root().and_then(|root| root.focus())
    }

    // A dialog is a descendant of the window, so an ungated Escape would close the find bar
    // instead of the dialog the user is looking at.
    fn find_scope_has_focus(&self) -> bool {
        self.focused().is_some_and(|focused| {
            focused.is_ancestor(&self.find_bar)
                || focused == *self.view.upcast_ref::<gtk4::Widget>()
                || focused.is_ancestor(&self.view)
        })
    }

    fn entry_has_focus(&self) -> bool {
        self.focused().is_some_and(|focused| {
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
}

fn icon_button(icon: &str, tooltip: &str, action: &str) -> gtk4::Button {
    gtk4::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .action_name(action)
        .build()
}
