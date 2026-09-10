use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use gtk4::gdk;
use gtk4::glib;
use gtk4::graphene;
use gtk4::gsk;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::document::DocumentHandle;
use crate::document::message::{LinkTarget, PageLink, PageSize, RenderJob, Response};
use crate::document::outline::OutlineItem;
use crate::document::text_model::Rect;
use crate::view::cache::TextureCache;
use crate::view::clipboard;
use crate::view::find::{self, FindState};
use crate::view::layout::{self, Layout, MAX_ZOOM, MIN_ZOOM, ZoomMode};
use crate::view::selection::{self, Granularity, Selection, TextPos, Texts};

const OVERSCAN: usize = 1;
const RETAINED_TEXT_PAGES: usize = 128;
const AUTOSCROLL_SPEED: f64 = 36.0;

#[derive(Default)]
struct State {
    doc: Option<DocumentHandle>,
    sizes: Vec<PageSize>,
    layout: Layout,
    zoom_mode: ZoomMode,
    zoom: f32,
    texts: Texts,
    links: HashMap<usize, Vec<PageLink>>,
    failed_renders: HashSet<usize>,
    failed_texts: HashSet<usize>,
    selection: Option<Selection>,
    selection_covers_document: bool,
    find: FindState,
    sent_renders: Vec<RenderJob>,
    sent_texts: Vec<usize>,
    sent_links: Vec<usize>,
    current_page: usize,
}

/// Where the pointer is in page space. `page` is always a real page — `page_at` names the
/// nearest one — so `on_page` carries whether the pointer is actually over it.
struct PointerTarget {
    page: usize,
    x: f32,
    y: f32,
    on_page: bool,
}

pub type OutlineCallback = Box<dyn Fn(&[OutlineItem])>;
pub type TextCallback = Box<dyn Fn(&str)>;
pub type StatusCallback = Box<dyn Fn(Option<&str>)>;
pub type PageCallback = Box<dyn Fn((usize, usize))>;

type Slot<T> = RefCell<Option<T>>;

struct Policy(Cell<gtk4::ScrollablePolicy>);

impl Default for Policy {
    fn default() -> Self {
        Self(Cell::new(gtk4::ScrollablePolicy::Minimum))
    }
}

#[derive(Default)]
pub struct PageView {
    hadjustment: RefCell<Option<gtk4::Adjustment>>,
    vadjustment: RefCell<Option<gtk4::Adjustment>>,
    hscroll_policy: Policy,
    vscroll_policy: Policy,
    hsignal: RefCell<Option<glib::SignalHandlerId>>,
    vsignal: RefCell<Option<glib::SignalHandlerId>>,
    style_signals: RefCell<Vec<glib::SignalHandlerId>>,
    state: RefCell<State>,
    cache: RefCell<TextureCache>,
    dragging: Cell<bool>,
    select_all: Cell<bool>,
    copy_pending: Cell<bool>,
    primary_pending: Cell<bool>,
    pending_press: Cell<Option<(f64, f64, i32, bool)>>,
    pending_find: RefCell<Option<String>>,
    pending_select_all: Cell<bool>,
    epoch: Cell<u64>,
    pointer: Cell<(f64, f64)>,
    tick: RefCell<Option<gtk4::TickCallbackId>>,
    last_autoscroll: Cell<i64>,
    hover_link: RefCell<Option<String>>,
    pressed_link: RefCell<Option<PageLink>>,
    cursor_name: Cell<&'static str>,
    on_loaded: Slot<OutlineCallback>,
    on_error: Slot<TextCallback>,
    on_find: Slot<TextCallback>,
    on_status: Slot<StatusCallback>,
    on_page: Slot<PageCallback>,
}

#[glib::object_subclass]
impl ObjectSubclass for PageView {
    const NAME: &'static str = "PdfReaderPageView";
    type Type = super::PageView;
    type ParentType = gtk4::Widget;
    type Interfaces = (gtk4::Scrollable,);
}

impl ObjectImpl for PageView {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
        PROPERTIES.get_or_init(|| {
            vec![
                glib::ParamSpecOverride::for_interface::<gtk4::Scrollable>("hadjustment"),
                glib::ParamSpecOverride::for_interface::<gtk4::Scrollable>("vadjustment"),
                glib::ParamSpecOverride::for_interface::<gtk4::Scrollable>("hscroll-policy"),
                glib::ParamSpecOverride::for_interface::<gtk4::Scrollable>("vscroll-policy"),
            ]
        })
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        match pspec.name() {
            "hadjustment" => self.set_adjustment(false, value.get().unwrap()),
            "vadjustment" => self.set_adjustment(true, value.get().unwrap()),
            "hscroll-policy" => self.hscroll_policy.0.set(value.get().unwrap()),
            "vscroll-policy" => self.vscroll_policy.0.set(value.get().unwrap()),
            other => unimplemented!("unknown property {other}"),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "hadjustment" => self.hadjustment.borrow().to_value(),
            "vadjustment" => self.vadjustment.borrow().to_value(),
            "hscroll-policy" => self.hscroll_policy.0.get().to_value(),
            "vscroll-policy" => self.vscroll_policy.0.get().to_value(),
            other => unimplemented!("unknown property {other}"),
        }
    }

    fn constructed(&self) {
        self.parent_constructed();
        let widget = self.obj();
        widget.set_focusable(true);
        widget.set_can_focus(true);
        widget.set_overflow(gtk4::Overflow::Hidden);
        widget.connect_scale_factor_notify(glib::clone!(
            #[weak(rename_to = imp)]
            self,
            move |widget| {
                widget.queue_resize();
                imp.relayout(true);
            }
        ));
        // A light/dark or accent change emits nothing on the widget, so nothing else
        // observes it, and snapshot reads the accent fresh on every frame.
        let style = libadwaita::StyleManager::default();
        self.style_signals.replace(vec![
            style.connect_dark_notify(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| imp.obj().queue_draw()
            )),
            style.connect_accent_color_notify(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| imp.obj().queue_draw()
            )),
        ]);
        self.install_controllers();
    }

    fn dispose(&self) {
        if let Some(tick) = self.tick.take() {
            tick.remove();
        }
        // An adjustment can outlive the widget, so drop our handlers with it.
        // Not via set_adjustment: that queues an allocation on a disposing widget.
        for (slot, signal) in [
            (&self.vadjustment, &self.vsignal),
            (&self.hadjustment, &self.hsignal),
        ] {
            if let (Some(adjustment), Some(id)) = (slot.take(), signal.take()) {
                adjustment.disconnect(id);
            }
        }
        // The style manager is a process-global singleton, so it would hold these
        // closures — and their weak refs — for the life of the process.
        let style = libadwaita::StyleManager::default();
        for id in self.style_signals.take() {
            style.disconnect(id);
        }
    }
}

impl WidgetImpl for PageView {
    fn measure(&self, orientation: gtk4::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
        let state = self.state.borrow();
        let dsf = self.obj().scale_factor().max(1);
        let natural = match orientation {
            gtk4::Orientation::Horizontal => state.layout.width / dsf,
            _ => state.layout.height / dsf,
        };
        (0, natural.max(1), -1, -1)
    }

    fn size_allocate(&self, _width: i32, _height: i32, _baseline: i32) {
        self.relayout(true);
    }

    fn snapshot(&self, snapshot: &gtk4::Snapshot) {
        let widget = self.obj();
        let state = self.state.borrow();
        if state.layout.is_empty() {
            return;
        }
        let dsf = widget.scale_factor().max(1) as f32;
        let (offx, offy) = self.offsets(dsf);
        let view_height = widget.height() as f32;
        let top = ((offy) * dsf).floor() as i32;
        let bottom = ((offy + view_height) * dsf).ceil() as i32;
        let range = state.layout.visible_range(top, bottom, 0);

        let selection_range = state.selection.map(|s| s.range());
        let accent = accent_color();
        let selection_color = with_alpha(accent, 0.32);
        let match_color = with_alpha(accent, 0.22);
        let current_color = with_alpha(accent, 0.55);
        let paper = gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);
        // In light mode the canvas is near-white, so a white page has no edge of its own.
        // A translucent black shadow reads on either canvas, so it needs no theme branch.
        let page_shadow = gdk::RGBA::new(0.0, 0.0, 0.0, 0.22);

        let mut cache = self.cache.borrow_mut();
        for page in range {
            let rect = state.layout.pages[page];
            let bounds = graphene::Rect::new(
                rect.x as f32 / dsf - offx,
                rect.y as f32 / dsf - offy,
                rect.width as f32 / dsf,
                rect.height as f32 / dsf,
            );
            snapshot.append_outset_shadow(
                &gsk::RoundedRect::from_rect(bounds, 0.0),
                &page_shadow,
                0.0,
                2.0,
                0.0,
                6.0,
            );
            snapshot.append_color(&paper, &bounds);
            if let Some(entry) = cache.get(page) {
                let filter = if entry.scale == state.layout.scale {
                    gsk::ScalingFilter::Nearest
                } else {
                    gsk::ScalingFilter::Trilinear
                };
                snapshot.append_scaled_texture(&entry.texture, filter, &bounds);
            }

            if let (Some(text), Some((start, end))) = (state.texts.get(&page), selection_range) {
                for hit in selection::highlight_rects(text, page, start, end) {
                    snapshot.append_color(
                        &selection_color,
                        &self.to_widget(&state.layout, page, hit, dsf, offx, offy),
                    );
                }
            }

            let first_hit = state.find.hits.partition_point(|hit| hit.page < page);
            let last_hit = state.find.hits.partition_point(|hit| hit.page <= page);
            for (offset, hit) in state.find.hits[first_hit..last_hit].iter().enumerate() {
                let is_current = Some(first_hit + offset) == state.find.current;
                let color = if is_current {
                    current_color
                } else {
                    match_color
                };
                snapshot.append_color(
                    &color,
                    &self.to_widget(&state.layout, page, hit.rect, dsf, offx, offy),
                );
            }
        }
    }
}

impl ScrollableImpl for PageView {}

impl PageView {
    fn set_adjustment(&self, vertical: bool, adjustment: Option<gtk4::Adjustment>) {
        let slot = if vertical {
            &self.vadjustment
        } else {
            &self.hadjustment
        };
        let signal_slot = if vertical {
            &self.vsignal
        } else {
            &self.hsignal
        };
        if let (Some(old), Some(id)) = (slot.borrow().as_ref(), signal_slot.take()) {
            old.disconnect(id);
        }
        if let Some(adjustment) = &adjustment {
            let id = adjustment.connect_value_changed(glib::clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.on_scrolled();
                }
            ));
            *signal_slot.borrow_mut() = Some(id);
        }
        *slot.borrow_mut() = adjustment;
        self.obj().queue_allocate();
    }

    fn on_scrolled(&self) {
        self.obj().queue_draw();
        self.schedule();
        self.notify_page();
    }

    fn centering_slack(&self, dsf: f32) -> f32 {
        ((self.obj().width() as f32 - self.state.borrow().layout.width as f32 / dsf) / 2.0)
            .max(0.0)
            .floor()
    }

    fn offsets(&self, dsf: f32) -> (f32, f32) {
        let round = |value: f64| ((value as f32) * dsf).round() / dsf;
        let scroll_x = self
            .hadjustment
            .borrow()
            .as_ref()
            .map_or(0.0, |a| round(a.value()));
        let scroll_y = self
            .vadjustment
            .borrow()
            .as_ref()
            .map_or(0.0, |a| round(a.value()));
        (scroll_x - self.centering_slack(dsf), scroll_y)
    }

    fn to_widget(
        &self,
        layout: &Layout,
        page: usize,
        rect: Rect,
        dsf: f32,
        offx: f32,
        offy: f32,
    ) -> graphene::Rect {
        let (x0, y0) = layout.page_to_content(page, rect.x0, rect.y0);
        let (x1, y1) = layout.page_to_content(page, rect.x1, rect.y1);
        graphene::Rect::new(
            x0 / dsf - offx,
            y0 / dsf - offy,
            (x1 - x0).max(1.0) / dsf,
            (y1 - y0).max(1.0) / dsf,
        )
    }

    fn anchor(&self) -> Option<(usize, f32)> {
        let state = self.state.borrow();
        if state.layout.is_empty() {
            return None;
        }
        let dsf = self.obj().scale_factor().max(1) as f32;
        let (_, offy) = self.offsets(dsf);
        let content_y = offy * dsf;
        let page = state.layout.page_at(content_y);
        let (_, page_y) = state.layout.content_to_page(page, 0.0, content_y);
        Some((page, page_y))
    }

    fn restore_anchor(&self, anchor: Option<(usize, f32)>) {
        let Some((page, page_y)) = anchor else {
            return;
        };
        let target = {
            let state = self.state.borrow();
            if page >= state.layout.pages.len() {
                return;
            }
            let dsf = self.obj().scale_factor().max(1) as f32;
            let (_, content_y) = state.layout.page_to_content(page, 0.0, page_y);
            (content_y / dsf) as f64
        };
        self.scroll_to(target);
    }

    pub(super) fn relayout(&self, keep_anchor: bool) {
        let widget = self.obj();
        let dsf = widget.scale_factor().max(1);
        let view_width = widget.width().max(1);
        let view_height = widget.height().max(1);
        let anchor = keep_anchor.then(|| self.anchor()).flatten();
        let (width, height, rebuilt) = {
            let mut state = self.state.borrow_mut();
            if state.sizes.is_empty() {
                (0, 0, false)
            } else {
                let zoom = layout::zoom_for(
                    state.zoom_mode,
                    &state.sizes,
                    (view_width * dsf, view_height * dsf),
                    dsf,
                );
                let rebuilt = !state.layout.matches(zoom, dsf);
                if rebuilt {
                    state.layout = Layout::new(&state.sizes, zoom, dsf);
                    state.sent_renders.clear();
                    state.failed_renders.clear();
                    if let Some(doc) = state.doc.as_ref() {
                        doc.request_renders(Vec::new());
                    }
                }
                state.zoom = zoom;
                (state.layout.width, state.layout.height, rebuilt)
            }
        };

        self.configure(false, width as f64 / dsf as f64, view_width as f64);
        self.configure(true, height as f64 / dsf as f64, view_height as f64);
        if keep_anchor {
            self.restore_anchor(anchor);
        }
        if rebuilt {
            self.center_horizontally();
        }
        self.schedule();
        self.notify_page();
        widget.queue_draw();
    }

    fn center_horizontally(&self) {
        let Some(adjustment) = self.hadjustment.borrow().clone() else {
            return;
        };
        let dsf = self.obj().scale_factor().max(1) as f64;
        let target = {
            let state = self.state.borrow();
            let index = if state.current_page < state.layout.pages.len() {
                state.current_page
            } else {
                0
            };
            let Some(rect) = state.layout.pages.get(index) else {
                return;
            };
            (rect.x + rect.width / 2) as f64 / dsf - adjustment.page_size() / 2.0
        };
        let upper = (adjustment.upper() - adjustment.page_size()).max(0.0);
        adjustment.set_value(target.clamp(0.0, upper));
    }

    fn configure(&self, vertical: bool, upper: f64, page_size: f64) {
        let slot = if vertical {
            &self.vadjustment
        } else {
            &self.hadjustment
        };
        let Some(adjustment) = slot.borrow().clone() else {
            return;
        };
        let upper = upper.max(page_size);
        let value = adjustment.value().clamp(0.0, upper - page_size);
        adjustment.configure(
            value,
            0.0,
            upper,
            page_size * 0.1,
            page_size * 0.9,
            page_size,
        );
    }

    fn visible_pages(&self) -> std::ops::Range<usize> {
        let state = self.state.borrow();
        if state.layout.is_empty() {
            return 0..0;
        }
        let dsf = self.obj().scale_factor().max(1) as f32;
        let (_, offy) = self.offsets(dsf);
        let height = self.obj().height().max(1) as f32;
        state.layout.visible_range(
            (offy * dsf) as i32,
            ((offy + height) * dsf) as i32,
            OVERSCAN,
        )
    }

    fn schedule(&self) {
        let range = self.visible_pages();
        if range.is_empty() {
            return;
        }
        let cache = self.cache.borrow();
        let mut state = self.state.borrow_mut();
        let scale = state.layout.scale;
        let mut renders = Vec::new();
        let mut texts = Vec::new();
        let mut links = Vec::new();
        let mut wanted: Vec<usize> = range.clone().collect();
        if let Some((start, end)) = state.selection.map(|s| s.range()) {
            wanted.extend(start.page..=end.page);
        }
        for page in range {
            let fresh = cache.peek(page).is_some_and(|entry| entry.scale == scale);
            if !fresh && !state.failed_renders.contains(&page) {
                renders.push(RenderJob { page, scale });
            }
            if !state.links.contains_key(&page) {
                links.push(page);
            }
        }
        if self.select_all.get() {
            wanted.extend(0..state.layout.pages.len());
        }
        wanted.sort_unstable();
        wanted.dedup();
        for page in wanted.iter().copied() {
            if !state.texts.contains_key(&page) && !state.failed_texts.contains(&page) {
                texts.push(page);
            }
        }
        // Per-character rects make page text far heavier than it looks, so pages nothing
        // needs any more are dropped once enough of them have accumulated.
        if state.texts.len() > RETAINED_TEXT_PAGES || state.links.len() > RETAINED_TEXT_PAGES {
            state
                .texts
                .retain(|page, _| wanted.binary_search(page).is_ok());
            state
                .links
                .retain(|page, _| wanted.binary_search(page).is_ok());
        }
        drop(cache);
        let state = &mut *state;
        let Some(doc) = state.doc.as_ref() else {
            return;
        };
        if renders != state.sent_renders {
            doc.request_renders(renders.clone());
            state.sent_renders = renders;
        }
        if texts != state.sent_texts {
            doc.request_texts(texts.clone());
            state.sent_texts = texts;
        }
        if links != state.sent_links {
            doc.request_links(links.clone());
            state.sent_links = links;
        }
    }

    fn notify_page(&self) {
        let (page, total) = {
            let state = self.state.borrow();
            if state.layout.is_empty() {
                return;
            }
            let dsf = self.obj().scale_factor().max(1) as f32;
            let (_, offy) = self.offsets(dsf);
            let height = self.obj().height().max(1) as f32 * 0.5;
            (
                state.layout.page_at((offy + height) * dsf),
                state.layout.pages.len(),
            )
        };
        if self.state.borrow().current_page == page {
            return;
        }
        self.state.borrow_mut().current_page = page;
        if let Some(callback) = self.on_page.borrow().as_ref() {
            callback((page, total));
        }
    }

    pub(super) fn load(&self, path: PathBuf) {
        let (handle, receiver) = DocumentHandle::open(path);
        let epoch = self.epoch.get().wrapping_add(1);
        self.epoch.set(epoch);
        self.select_all.set(false);
        self.copy_pending.set(false);
        self.primary_pending.set(false);
        self.pending_press.set(None);
        self.dragging.set(false);
        self.hover_link.take();
        self.pending_find.take();
        self.pending_select_all.set(false);
        {
            let mut state = self.state.borrow_mut();
            *state = State {
                doc: Some(handle),
                current_page: usize::MAX,
                ..State::default()
            };
        }
        self.cache.borrow_mut().clear();
        for slot in [&self.hadjustment, &self.vadjustment] {
            if let Some(adjustment) = slot.borrow().as_ref() {
                adjustment.set_value(0.0);
            }
        }
        if let Some(callback) = self.on_loaded.borrow().as_ref() {
            callback(&[]);
        }
        if let Some(callback) = self.on_page.borrow().as_ref() {
            callback((0, 0));
        }
        self.emit_find();
        self.report_status(None);
        self.obj().queue_draw();
        let weak = self.obj().downgrade();
        glib::spawn_future_local(async move {
            while let Ok(response) = receiver.recv().await {
                let Some(view) = weak.upgrade() else {
                    break;
                };
                let imp = view.imp();
                if imp.epoch.get() != epoch {
                    break;
                }
                imp.handle(response);
            }
        });
    }

    fn handle(&self, response: Response) {
        match response {
            Response::Opened { pages, outline } => {
                self.state.borrow_mut().sizes = pages;
                self.obj().queue_resize();
                self.relayout(false);
                if let Some(callback) = self.on_loaded.borrow().as_ref() {
                    callback(&outline);
                }
                if self.pending_select_all.replace(false) {
                    self.select_all();
                }
                if let Some(needle) = self.pending_find.take() {
                    self.find(&needle);
                }
            }
            Response::OpenFailed(message) => {
                if let Some(callback) = self.on_error.borrow().as_ref() {
                    callback(&message);
                }
            }
            Response::Rendered(rendered) => {
                if rendered.scale != self.state.borrow().layout.scale {
                    return;
                }
                let bytes = glib::Bytes::from_owned(rendered.pixels);
                let texture = gdk::MemoryTexture::new(
                    rendered.width,
                    rendered.height,
                    gdk::MemoryFormat::R8g8b8,
                    &bytes,
                    rendered.stride,
                );
                self.cache
                    .borrow_mut()
                    .insert(rendered.page, texture.upcast(), rendered.scale);
                self.state
                    .borrow_mut()
                    .sent_renders
                    .retain(|job| job.page != rendered.page);
                self.schedule();
                self.obj().queue_draw();
            }
            Response::RenderFailed { page } => {
                let mut state = self.state.borrow_mut();
                state.failed_renders.insert(page);
                state.sent_renders.retain(|job| job.page != page);
            }
            Response::Text { page, text } => {
                self.state.borrow_mut().texts.insert(page, text);
                self.on_text_settled();
            }
            Response::TextFailed { page } => {
                self.state.borrow_mut().failed_texts.insert(page);
                self.on_text_settled();
            }
            Response::Links { page, links } => {
                self.state.borrow_mut().links.insert(page, links);
            }
            Response::SearchHits {
                generation,
                page,
                rects,
                truncated,
            } => {
                let mut state = self.state.borrow_mut();
                if state.find.generation != generation {
                    return;
                }
                let first = state.find.hits.is_empty();
                state.find.truncated |= truncated;
                state.find.add(page, rects);
                if first && !state.find.hits.is_empty() {
                    let current = state.current_page;
                    state.find.select_nearest(current);
                    let hit = state.find.current_hit();
                    drop(state);
                    if let Some(hit) = hit {
                        self.scroll_to_rect(hit.page, hit.rect);
                    }
                } else {
                    drop(state);
                }
                self.emit_find();
                self.obj().queue_draw();
            }
            Response::SearchFinished { generation } => {
                let mut state = self.state.borrow_mut();
                if state.find.generation == generation {
                    state.find.searching = false;
                }
                drop(state);
                self.emit_find();
            }
            Response::Idle(_) => {}
        }
    }

    fn on_text_settled(&self) {
        if self.select_all.get() {
            self.apply_select_all();
        }
        self.apply_pending_pointer();
        if self.selection_complete() {
            if self.copy_pending.get() {
                self.copy_selection();
            }
            if self.primary_pending.replace(false) {
                self.write_primary();
            }
        }
        self.obj().queue_draw();
    }

    fn report_status(&self, status: Option<&str>) {
        if let Some(callback) = self.on_status.borrow().as_ref() {
            callback(status);
        }
    }

    fn report_hover(&self, status: Option<&str>) {
        if self.copy_pending.get() {
            return;
        }
        self.report_status(status);
    }

    fn finish_pending_copy(&self) {
        if self.copy_pending.take() {
            let hover = self.hover_link.borrow().clone();
            self.report_status(hover.as_deref());
        }
    }

    fn emit_find(&self) {
        let status = self.state.borrow().find.status();
        if let Some(callback) = self.on_find.borrow().as_ref() {
            callback(&status);
        }
    }

    fn install_controllers(&self) {
        let widget = self.obj();

        let click = gtk4::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = imp)]
            self,
            move |gesture, presses, x, y| {
                imp.on_pressed(gesture, presses, x, y);
            }
        ));
        click.connect_released(glib::clone!(
            #[weak(rename_to = imp)]
            self,
            move |_, _, x, y| {
                imp.on_released(x, y);
            }
        ));
        click.connect_cancel(glib::clone!(
            #[weak(rename_to = imp)]
            self,
            move |_, _| {
                imp.cancel_drag();
            }
        ));
        widget.add_controller(click);

        let motion = gtk4::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = imp)]
            self,
            move |_, x, y| {
                imp.on_motion(x, y);
            }
        ));
        motion.connect_leave(glib::clone!(
            #[weak(rename_to = imp)]
            self,
            move |_| {
                // Mid-drag the pointer stays meaningful: autoscroll and extend_to
                // must not act on the off-widget sentinel.
                if !imp.dragging.get() {
                    imp.pointer.set((-1.0, -1.0));
                }
                if imp.hover_link.borrow_mut().take().is_some() {
                    imp.report_hover(None);
                }
            }
        ));
        widget.add_controller(motion);

        let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(glib::clone!(
            #[weak(rename_to = imp)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |controller, _, dy| {
                if !controller
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK)
                {
                    return glib::Propagation::Proceed;
                }
                if dy == 0.0 {
                    return glib::Propagation::Stop;
                }
                imp.zoom_by(if dy < 0.0 { 1.1 } else { 1.0 / 1.1 });
                glib::Propagation::Stop
            }
        ));
        widget.add_controller(scroll);
    }

    fn pointer_target(&self, x: f64, y: f64) -> Option<PointerTarget> {
        let state = self.state.borrow();
        if state.layout.is_empty() {
            return None;
        }
        let dsf = self.obj().scale_factor().max(1) as f32;
        let (offx, offy) = self.offsets(dsf);
        let content_x = (x as f32 + offx) * dsf;
        let content_y = (y as f32 + offy) * dsf;
        let page = state.layout.page_at(content_y);
        let (px, py) = state.layout.content_to_page(page, content_x, content_y);
        Some(PointerTarget {
            page,
            x: px,
            y: py,
            on_page: state.layout.pages[page].contains(content_x, content_y),
        })
    }

    /// Nearest position on the nearest page, however far away the pointer is. For extending a
    /// drag, where the pointer routinely leaves the page.
    fn pointer_to_pos(&self, x: f64, y: f64) -> Option<TextPos> {
        self.pos_at(&self.pointer_target(x, y)?)
    }

    fn pos_at(&self, target: &PointerTarget) -> Option<TextPos> {
        let state = self.state.borrow();
        let text = state.texts.get(&target.page)?;
        selection::hit_test(text, target.page, target.x, target.y)
    }

    /// Position under the pointer, or `None` in the margins and the inter-page gaps. For
    /// starting a selection and for the cursor shape, neither of which should react to
    /// background the way a browser does not.
    fn pointer_to_pos_on_page(&self, x: f64, y: f64) -> Option<TextPos> {
        let target = self.pointer_target(x, y).filter(|target| target.on_page)?;
        self.pos_at(&target)
    }

    fn link_at(&self, x: f64, y: f64) -> Option<PageLink> {
        let target = self.pointer_target(x, y).filter(|target| target.on_page)?;
        self.link_for_target(&target)
    }

    fn link_for_target(&self, target: &PointerTarget) -> Option<PageLink> {
        let state = self.state.borrow();
        state
            .links
            .get(&target.page)?
            .iter()
            .find(|link| link.rect.contains(target.x, target.y))
            .cloned()
    }

    fn page_text_pending(&self, x: f64, y: f64) -> bool {
        let Some(target) = self.pointer_target(x, y) else {
            return false;
        };
        target.on_page && !text_settled(&self.state.borrow(), target.page)
    }

    fn on_pressed(&self, gesture: &gtk4::GestureClick, presses: i32, x: f64, y: f64) {
        self.obj().grab_focus();
        self.cancel_pending_copy();
        if presses == 1
            && let Some(link) = self.link_at(x, y)
        {
            // Armed on press, fired on release: a mis-press can still be
            // aborted by releasing off the link.
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            *self.pressed_link.borrow_mut() = Some(link);
            self.state.borrow_mut().selection = None;
            self.obj().queue_draw();
            return;
        }
        let extend = gesture
            .current_event_state()
            .contains(gdk::ModifierType::SHIFT_MASK);
        if self.pointer_to_pos_on_page(x, y).is_some() {
            self.begin_selection(x, y, presses, extend);
        } else if self.page_text_pending(x, y) {
            // The press is replayed from the Text response rather than dropped. The old
            // selection goes now: a release cancels the replay, and a click that appears
            // to do nothing must not leave a stale highlight behind.
            if !extend {
                self.state.borrow_mut().selection = None;
            }
            self.pending_press.set(Some((x, y, presses, extend)));
        } else {
            self.state.borrow_mut().selection = None;
            self.obj().queue_draw();
            return;
        }
        self.dragging.set(true);
        self.pointer.set((x, y));
        self.start_autoscroll();
        self.obj().queue_draw();
    }

    fn begin_selection(&self, x: f64, y: f64, presses: i32, extend: bool) {
        let Some(pos) = self.pointer_to_pos_on_page(x, y) else {
            return;
        };
        let granularity = match presses {
            1 => Granularity::Char,
            2 => Granularity::Word,
            _ => Granularity::Line,
        };
        let mut state = self.state.borrow_mut();
        state.selection_covers_document = false;
        let extending = extend
            .then(|| state.selection.map(|s| s.granularity))
            .flatten();
        let unit = selection::expand(&state.texts, pos, extending.unwrap_or(granularity));
        match (extending, state.selection.as_mut()) {
            (Some(_), Some(selection)) => selection.set_focus(unit),
            _ => state.selection = Some(Selection::new(unit, granularity)),
        }
    }

    fn apply_pending_pointer(&self) {
        if let Some((x, y, presses, extend)) = self.pending_press.get() {
            if self.pointer_to_pos_on_page(x, y).is_none() {
                return;
            }
            self.pending_press.set(None);
            self.begin_selection(x, y, presses, extend);
        }
        if self.dragging.get() {
            let (x, y) = self.pointer.get();
            self.extend_to(x, y);
        }
    }

    fn cancel_drag(&self) {
        self.dragging.set(false);
        self.pending_press.set(None);
        self.pressed_link.borrow_mut().take();
        self.stop_autoscroll();
    }

    fn on_released(&self, x: f64, y: f64) {
        let armed = self.pressed_link.borrow_mut().take();
        if let Some(link) = armed {
            self.cancel_drag();
            // A link has no identity of its own: same area, same target is the same link.
            if self.link_at(x, y).is_some_and(|hit| hit == link) {
                self.activate_link(&link);
            }
            return;
        }
        self.cancel_drag();
        if self.selection_complete() {
            self.write_primary();
        } else {
            self.primary_pending.set(true);
        }
        self.schedule();
    }

    fn selection_text(&self) -> String {
        let state = self.state.borrow();
        match state.selection {
            Some(selection) if !selection.is_empty() => {
                let (start, end) = selection.range();
                selection::selected_text(&state.texts, start, end)
            }
            _ => String::new(),
        }
    }

    fn write_primary(&self) {
        clipboard::primary(&*self.obj(), &self.selection_text());
    }

    fn on_motion(&self, x: f64, y: f64) {
        self.pointer.set((x, y));
        if self.dragging.get() {
            self.extend_to(x, y);
            return;
        }
        // One pointer_target for the hover link and the cursor shape: this runs per motion event.
        let target = self.pointer_target(x, y).filter(|target| target.on_page);
        let link = target
            .as_ref()
            .and_then(|target| self.link_for_target(target));
        let uri = link.as_ref().map(|link| match &link.target {
            LinkTarget::Uri(uri) => uri.clone(),
            LinkTarget::Page(page) => format!("Page {}", page + 1),
        });
        if *self.hover_link.borrow() != uri {
            *self.hover_link.borrow_mut() = uri.clone();
            self.report_hover(uri.as_deref());
        }
        let name = if link.is_some() {
            "pointer"
        } else if target.and_then(|target| self.pos_at(&target)).is_some() {
            "text"
        } else {
            "default"
        };
        if self.cursor_name.get() != name {
            self.cursor_name.set(name);
            self.obj().set_cursor_from_name(Some(name));
        }
    }

    fn extend_to(&self, x: f64, y: f64) {
        let Some(pos) = self.pointer_to_pos(x, y) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let granularity = match state.selection {
                Some(selection) => selection.granularity,
                None => return,
            };
            let unit = selection::expand(&state.texts, pos, granularity);
            if let Some(selection) = state.selection.as_mut() {
                selection.set_focus(unit);
            }
        }
        self.obj().queue_draw();
        self.schedule();
    }

    fn start_autoscroll(&self) {
        if self.tick.borrow().is_some() {
            return;
        }
        self.last_autoscroll.set(0);
        let id = self.obj().add_tick_callback(glib::clone!(
            #[weak(rename_to = imp)]
            self,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move |_, clock| {
                imp.autoscroll(clock.frame_time());
                glib::ControlFlow::Continue
            }
        ));
        *self.tick.borrow_mut() = Some(id);
    }

    fn stop_autoscroll(&self) {
        if let Some(tick) = self.tick.take() {
            tick.remove();
        }
    }

    fn autoscroll(&self, frame_time: i64) {
        let previous = self.last_autoscroll.replace(frame_time);
        if !self.dragging.get() {
            return;
        }
        let (x, y) = self.pointer.get();
        let height = self.obj().height() as f64;
        let delta = if y < 0.0 {
            y
        } else if y > height {
            y - height
        } else {
            return;
        };
        let Some(adjustment) = self.vadjustment.borrow().clone() else {
            return;
        };
        let elapsed = match previous {
            0 => 0.0,
            previous => ((frame_time - previous) as f64 / 1_000_000.0).clamp(0.0, 0.1),
        };
        let upper = (adjustment.upper() - adjustment.page_size()).max(0.0);
        let value = (adjustment.value() + delta * AUTOSCROLL_SPEED * elapsed).clamp(0.0, upper);
        adjustment.set_value(value);
        self.extend_to(x, y.clamp(0.0, height));
    }

    fn activate_link(&self, link: &PageLink) {
        match &link.target {
            LinkTarget::Page(page) => self.goto_page(*page),
            LinkTarget::Uri(uri) => {
                let launcher = gtk4::UriLauncher::new(uri);
                let window = self.obj().root().and_downcast::<gtk4::Window>();
                launcher.launch(window.as_ref(), gtk4::gio::Cancellable::NONE, |_| {});
            }
        }
    }

    pub(super) fn goto_page(&self, page: usize) {
        let target = {
            let state = self.state.borrow();
            let Some(rect) = state.layout.pages.get(page) else {
                return;
            };
            let dsf = self.obj().scale_factor().max(1) as f32;
            (rect.y as f32 / dsf) as f64
        };
        self.scroll_to(target);
    }

    fn scroll_to_rect(&self, page: usize, rect: Rect) {
        let dsf = self.obj().scale_factor().max(1) as f32;
        let Some((top, bottom, target, left, right)) = ({
            let state = self.state.borrow();
            (page < state.layout.pages.len()).then(|| {
                let (left, content_y) = state.layout.page_to_content(page, rect.x0, rect.y0);
                let (right, content_y1) = state.layout.page_to_content(page, rect.x1, rect.y1);
                let height = self.obj().height().max(1) as f32;
                (
                    (content_y / dsf) as f64,
                    (content_y1 / dsf) as f64,
                    (content_y / dsf - height * 0.3) as f64,
                    left / dsf,
                    right / dsf,
                )
            })
        }) else {
            return;
        };
        self.scroll_y_into_view(top, bottom, target);
        self.scroll_x_into_view(left, right);
    }

    /// Like `scroll_x_into_view`: a match already on screen must not jump the view,
    /// otherwise stepping through neighbouring matches re-parks the page on each step.
    fn scroll_y_into_view(&self, top: f64, bottom: f64, target: f64) {
        let Some(adjustment) = self.vadjustment.borrow().clone() else {
            return;
        };
        let (value, page_size) = (adjustment.value(), adjustment.page_size());
        if top >= value && bottom <= value + page_size {
            return;
        }
        self.scroll_to(target);
    }

    /// Minimal horizontal scroll: a match already on screen must not shift the view sideways.
    fn scroll_x_into_view(&self, left: f32, right: f32) {
        let Some(adjustment) = self.hadjustment.borrow().clone() else {
            return;
        };
        let dsf = self.obj().scale_factor().max(1) as f32;
        let slack = self.centering_slack(dsf) as f64;
        let (left, right) = (left as f64 + slack, right as f64 + slack);
        let (value, page_size) = (adjustment.value(), adjustment.page_size());
        let target = if left < value {
            left
        } else if right > value + page_size {
            right - page_size
        } else {
            return;
        };
        adjustment.set_value(target.clamp(0.0, (adjustment.upper() - page_size).max(0.0)));
    }

    fn scroll_to(&self, value: f64) {
        let Some(adjustment) = self.vadjustment.borrow().clone() else {
            return;
        };
        let upper = (adjustment.upper() - adjustment.page_size()).max(0.0);
        adjustment.set_value(value.clamp(0.0, upper));
    }

    pub(super) fn zoom_by(&self, factor: f32) {
        if self.state.borrow().layout.is_empty() {
            return;
        }
        let dsf = self.obj().scale_factor().max(1) as f32;
        let focus = self.pointer_anchor(dsf);
        let zoom = (self.state.borrow().zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.state.borrow_mut().zoom_mode = ZoomMode::Fixed(zoom);
        self.obj().queue_resize();
        self.relayout(focus.is_none());
        let Some((page, x, y)) = focus else {
            return;
        };
        let pointer = self.pointer.get();
        let target = {
            let state = self.state.borrow();
            if page >= state.layout.pages.len() {
                return;
            }
            let (content_x, content_y) = state.layout.page_to_content(page, x, y);
            (
                content_x / dsf - pointer.0 as f32,
                content_y / dsf - pointer.1 as f32,
            )
        };
        self.set_scroll(target.0 + self.centering_slack(dsf), target.1);
    }

    fn pointer_anchor(&self, dsf: f32) -> Option<(usize, f32, f32)> {
        let (x, y) = self.pointer.get();
        if x < 0.0 || y < 0.0 || x >= self.obj().width() as f64 || y >= self.obj().height() as f64 {
            return None;
        }
        let state = self.state.borrow();
        if state.layout.is_empty() {
            return None;
        }
        let (offx, offy) = self.offsets(dsf);
        let content_x = (x as f32 + offx) * dsf;
        let content_y = (y as f32 + offy) * dsf;
        let page = state.layout.page_at(content_y);
        let (px, py) = state.layout.content_to_page(page, content_x, content_y);
        Some((page, px, py))
    }

    fn set_scroll(&self, x: f32, y: f32) {
        let pairs = [
            (self.hadjustment.borrow().clone(), x),
            (self.vadjustment.borrow().clone(), y),
        ];
        for (adjustment, value) in pairs {
            let Some(adjustment) = adjustment else {
                continue;
            };
            let upper = (adjustment.upper() - adjustment.page_size()).max(0.0);
            adjustment.set_value((value as f64).clamp(0.0, upper));
        }
    }

    pub(super) fn set_zoom_mode(&self, mode: ZoomMode) {
        self.state.borrow_mut().zoom_mode = mode;
        self.obj().queue_resize();
        self.relayout(true);
    }

    pub(super) fn select_all(&self) {
        if self.state.borrow().layout.is_empty() {
            self.pending_select_all.set(true);
            return;
        }
        self.select_all.set(true);
        self.schedule();
        self.apply_select_all();
    }

    fn apply_select_all(&self) {
        let complete = {
            let mut state = self.state.borrow_mut();
            let pages = state.layout.pages.len();
            let complete = (0..pages).all(|page| text_settled(&state, page));
            if let Some((start, end)) = selection::document_range(&state.texts, pages) {
                let mut selection = Selection::new((start, start), Granularity::Char);
                selection.set_focus((end, end));
                state.selection = Some(selection);
                state.selection_covers_document = true;
            }
            complete
        };
        if complete {
            self.select_all.set(false);
        }
        self.obj().queue_draw();
    }

    fn selection_complete(&self) -> bool {
        if self.select_all.get() || self.pending_select_all.get() {
            return false;
        }
        let state = self.state.borrow();
        let Some(selection) = state.selection else {
            return true;
        };
        let (start, end) = selection.range();
        (start.page..=end.page).all(|page| text_settled(&state, page))
    }

    /// Pages a copy silently skipped. Select-all is measured over the whole document, not over
    /// the selection: `document_range` stops at the outermost pages that *have* text, so a
    /// scanned cover or back page falls outside the range that dropped it.
    fn unreadable_pages(&self) -> usize {
        let state = self.state.borrow();
        let Some(selection) = state.selection else {
            return 0;
        };
        let (start, end) = selection.range();
        let range = if state.selection_covers_document {
            0..state.layout.pages.len()
        } else {
            start.page..end.page + 1
        };
        range
            .filter(|page| state.failed_texts.contains(page))
            .count()
    }

    fn cancel_pending_copy(&self) {
        self.select_all.set(false);
        self.pending_select_all.set(false);
        self.primary_pending.set(false);
        self.finish_pending_copy();
    }

    pub(super) fn copy_selection(&self) {
        if !self.selection_complete() {
            self.copy_pending.set(true);
            self.report_status(Some("Extracting text…"));
            self.schedule();
            return;
        }
        self.finish_pending_copy();
        clipboard::copy(&*self.obj(), &self.selection_text());
        let unreadable = self.unreadable_pages();
        if unreadable > 0 {
            let noun = if unreadable == 1 { "page" } else { "pages" };
            self.report_status(Some(&format!(
                "Copied — no text could be read from {unreadable} {noun}"
            )));
        }
    }

    pub(super) fn find(&self, needle: &str) {
        if needle.is_empty() {
            self.pending_find.take();
            self.find_close();
            return;
        }
        if self.state.borrow().layout.is_empty() {
            *self.pending_find.borrow_mut() = Some(needle.to_string());
            return;
        }
        let (generation, pages) = {
            let mut state = self.state.borrow_mut();
            let pages = find::search_order(state.current_page, state.layout.pages.len());
            (state.find.start(needle.to_string()), pages)
        };
        if pages.is_empty() {
            self.find_close();
            return;
        }
        if let Some(doc) = self.state.borrow().doc.as_ref() {
            doc.search(generation, needle.to_string(), pages);
        }
        self.emit_find();
        self.obj().queue_draw();
    }

    pub(super) fn find_step(&self, forward: bool) {
        let hit = self.state.borrow_mut().find.step(forward);
        if let Some(hit) = hit {
            self.scroll_to_rect(hit.page, hit.rect);
        }
        self.emit_find();
        self.obj().queue_draw();
    }

    pub(super) fn find_close(&self) {
        if let Some(doc) = self.state.borrow().doc.as_ref() {
            doc.cancel_search();
        }
        self.state.borrow_mut().find.reset();
        self.emit_find();
        self.obj().queue_draw();
    }

    pub(super) fn set_on_loaded(&self, callback: OutlineCallback) {
        *self.on_loaded.borrow_mut() = Some(callback);
    }

    pub(super) fn set_on_error(&self, callback: TextCallback) {
        *self.on_error.borrow_mut() = Some(callback);
    }

    pub(super) fn set_on_find(&self, callback: TextCallback) {
        *self.on_find.borrow_mut() = Some(callback);
    }

    pub(super) fn set_on_status(&self, callback: StatusCallback) {
        *self.on_status.borrow_mut() = Some(callback);
    }

    pub(super) fn set_on_page(&self, callback: PageCallback) {
        *self.on_page.borrow_mut() = Some(callback);
    }
}

// A page whose extraction failed never yields text, so it counts as settled: otherwise every
// gate waiting on it — the deferred copy, select-all — would wait forever.
fn text_settled(state: &State, page: usize) -> bool {
    state.texts.contains_key(&page) || state.failed_texts.contains(&page)
}

fn accent_color() -> gdk::RGBA {
    libadwaita::StyleManager::default().accent_color_rgba()
}

fn with_alpha(color: gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(color.red(), color.green(), color.blue(), alpha)
}
