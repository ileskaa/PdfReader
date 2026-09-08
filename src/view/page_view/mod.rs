mod imp;

use std::path::PathBuf;

use gtk4::glib;
use gtk4::subclass::prelude::*;

use crate::document::outline::OutlineItem;
use crate::view::layout::ZoomMode;

glib::wrapper! {
    pub struct PageView(ObjectSubclass<imp::PageView>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget, gtk4::Scrollable;
}

impl Default for PageView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl PageView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(&self, path: PathBuf) {
        self.imp().load(path);
    }

    pub fn set_zoom_mode(&self, mode: ZoomMode) {
        self.imp().set_zoom_mode(mode);
    }

    pub fn zoom_by(&self, factor: f32) {
        self.imp().zoom_by(factor);
    }

    pub fn goto_page(&self, page: usize) {
        self.imp().goto_page(page);
    }

    pub fn select_all(&self) {
        self.imp().select_all();
    }

    pub fn copy_selection(&self) {
        self.imp().copy_selection();
    }

    pub fn find(&self, needle: &str) {
        self.imp().find(needle);
    }

    pub fn find_step(&self, forward: bool) {
        self.imp().find_step(forward);
    }

    pub fn find_close(&self) {
        self.imp().find_close();
    }

    pub fn connect_loaded(&self, callback: impl Fn(&[OutlineItem]) + 'static) {
        self.imp().set_on_loaded(Box::new(callback));
    }

    pub fn connect_error(&self, callback: impl Fn(&str) + 'static) {
        self.imp().set_on_error(Box::new(callback));
    }

    pub fn connect_find_status(&self, callback: impl Fn(&str) + 'static) {
        self.imp().set_on_find(Box::new(callback));
    }

    pub fn connect_status(&self, callback: impl Fn(Option<&str>) + 'static) {
        self.imp().set_on_status(Box::new(callback));
    }

    pub fn connect_page_changed(&self, callback: impl Fn((usize, usize)) + 'static) {
        self.imp().set_on_page(Box::new(callback));
    }
}
