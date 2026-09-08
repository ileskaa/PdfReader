use gtk4::prelude::*;

/// Every clipboard write goes through this module so the empty-text guard and the
/// two-clipboard policy stay in one place.
pub fn copy(widget: &impl IsA<gtk4::Widget>, text: &str) {
    set(widget, text);
    primary(widget, text);
}

pub fn set(widget: &impl IsA<gtk4::Widget>, text: &str) {
    if text.is_empty() {
        return;
    }
    widget.as_ref().clipboard().set_text(text);
}

pub fn primary(widget: &impl IsA<gtk4::Widget>, text: &str) {
    if text.is_empty() {
        return;
    }
    widget.as_ref().primary_clipboard().set_text(text);
}
