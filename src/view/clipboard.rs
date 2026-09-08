use gtk4::prelude::*;

pub fn copy(widget: &impl IsA<gtk4::Widget>, text: &str) {
    if text.is_empty() {
        return;
    }
    let widget = widget.as_ref();
    widget.clipboard().set_text(text);
    widget.primary_clipboard().set_text(text);
}
