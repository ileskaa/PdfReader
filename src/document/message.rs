use super::outline::OutlineItem;
use super::text_model::{PageText, Rect};

#[derive(Clone, Copy, Debug)]
pub struct PageSize {
    pub x0: f32,
    pub y0: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderJob {
    pub page: usize,
    pub scale: f32,
}

#[derive(Clone, Debug)]
pub enum LinkTarget {
    Page(usize),
    Uri(String),
}

#[derive(Clone, Debug)]
pub struct PageLink {
    pub rect: Rect,
    pub target: LinkTarget,
}

#[derive(Debug)]
pub struct RenderedPage {
    pub page: usize,
    pub scale: f32,
    pub width: i32,
    pub height: i32,
    pub stride: usize,
    pub pixels: Vec<u8>,
}

#[derive(Debug)]
pub enum Response {
    Opened {
        pages: Vec<PageSize>,
        outline: Vec<OutlineItem>,
    },
    OpenFailed(String),
    Rendered(RenderedPage),
    RenderFailed {
        page: usize,
    },
    Text {
        page: usize,
        text: PageText,
    },
    TextFailed {
        page: usize,
    },
    Links {
        page: usize,
        links: Vec<PageLink>,
    },
    SearchHits {
        generation: u64,
        page: usize,
        rects: Vec<Rect>,
        truncated: bool,
    },
    SearchFinished {
        generation: u64,
    },
    Idle(u64),
}
