use std::collections::HashMap;
use std::ops::Range;

use crate::document::message::PageSize;

pub const GAP: i32 = 16;
pub const MARGIN: i32 = 16;
pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 8.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ZoomMode {
    Fixed(f32),
    #[default]
    FitWidth,
    FitPage,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    origin_x: i32,
    origin_y: i32,
}

impl PageRect {
    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32
            && x < (self.x + self.width) as f32
            && y >= self.y as f32
            && y < self.bottom() as f32
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub scale: f32,
    pub device_scale: i32,
    pub pages: Vec<PageRect>,
    pub width: i32,
    pub height: i32,
}

impl Layout {
    pub fn new(sizes: &[PageSize], zoom: f32, device_scale: i32) -> Self {
        let scale = zoom * device_scale as f32;
        let gap = GAP * device_scale;
        let margin = MARGIN * device_scale;
        let mut pages = Vec::with_capacity(sizes.len());
        let mut width = 0;
        let mut y = margin;
        for size in sizes {
            let x0 = floor_edge(size.x0 * scale);
            let y0 = floor_edge(size.y0 * scale);
            let w = (ceil_edge((size.x0 + size.width) * scale) - x0).max(1);
            let h = (ceil_edge((size.y0 + size.height) * scale) - y0).max(1);
            pages.push(PageRect {
                x: 0,
                y,
                width: w,
                height: h,
                origin_x: x0,
                origin_y: y0,
            });
            width = width.max(w);
            y += h + gap;
        }
        let content_width = width + margin * 2;
        for page in pages.iter_mut() {
            page.x = (content_width - page.width) / 2;
        }
        let height = if pages.is_empty() {
            0
        } else {
            y - gap + margin
        };
        Self {
            scale,
            device_scale,
            pages,
            width: content_width,
            height,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    pub fn matches(&self, zoom: f32, device_scale: i32) -> bool {
        self.device_scale == device_scale && (self.scale - zoom * device_scale as f32).abs() < 1e-4
    }

    pub fn visible_range(&self, top: i32, bottom: i32, overscan: usize) -> Range<usize> {
        if self.pages.is_empty() {
            return 0..0;
        }
        let first = self
            .pages
            .partition_point(|page| page.bottom() < top)
            .min(self.pages.len() - 1);
        let last = self.pages.partition_point(|page| page.y <= bottom);
        let start = first.saturating_sub(overscan);
        let end = (last + overscan).min(self.pages.len());
        start..end.max(start + 1)
    }

    pub fn page_to_content(&self, page: usize, x: f32, y: f32) -> (f32, f32) {
        let rect = self.pages[page];
        (
            rect.x as f32 + x * self.scale - rect.origin_x as f32,
            rect.y as f32 + y * self.scale - rect.origin_y as f32,
        )
    }

    pub fn content_to_page(&self, page: usize, x: f32, y: f32) -> (f32, f32) {
        let rect = self.pages[page];
        (
            (x - rect.x as f32 + rect.origin_x as f32) / self.scale,
            (y - rect.y as f32 + rect.origin_y as f32) / self.scale,
        )
    }

    pub fn page_at(&self, y: f32) -> usize {
        let y = y as i32;
        self.pages
            .partition_point(|page| page.bottom() <= y)
            .min(self.pages.len().saturating_sub(1))
    }
}

fn floor_edge(value: f32) -> i32 {
    (value + 0.001).floor() as i32
}

fn ceil_edge(value: f32) -> i32 {
    (value - 0.001).ceil() as i32
}

pub fn reference_size(sizes: &[PageSize]) -> (f32, f32) {
    let mut counts: HashMap<(i32, i32), usize> = HashMap::new();
    for size in sizes {
        *counts
            .entry((size.width.round() as i32, size.height.round() as i32))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|((width, height), count)| (*count, *width, *height))
        .map(|((width, height), _)| (width as f32, height as f32))
        .unwrap_or((1.0, 1.0))
}

pub fn zoom_for(
    mode: ZoomMode,
    sizes: &[PageSize],
    viewport: (i32, i32),
    device_scale: i32,
) -> f32 {
    let (width, height) = reference_size(sizes);
    let inset = (MARGIN * 2 * device_scale) as f32;
    let available_width = (viewport.0 as f32 - inset).max(1.0);
    let available_height = (viewport.1 as f32 - inset).max(1.0);
    let zoom = match mode {
        ZoomMode::Fixed(zoom) => zoom,
        ZoomMode::FitWidth => available_width / width / device_scale as f32,
        ZoomMode::FitPage => {
            (available_width / width).min(available_height / height) / device_scale as f32
        }
    };
    zoom.clamp(MIN_ZOOM, MAX_ZOOM)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes() -> Vec<PageSize> {
        vec![PageSize {
            x0: 0.0,
            y0: 0.0,
            width: 612.0,
            height: 792.0,
        }]
    }

    #[test]
    fn a_layout_is_stale_when_the_display_scale_changes() {
        let layout = Layout::new(&sizes(), 1.0, 1);
        assert!(layout.matches(1.0, 1));
        assert!(!layout.matches(1.0, 2));
        assert!(!layout.matches(2.0, 1));
    }

    #[test]
    fn device_scale_doubles_the_rasterised_page() {
        let one = Layout::new(&sizes(), 1.0, 1);
        let two = Layout::new(&sizes(), 1.0, 2);
        assert_eq!(one.pages[0].width * 2, two.pages[0].width);
        assert_eq!(one.pages[0].height * 2, two.pages[0].height);
    }

    #[test]
    fn page_and_content_coordinates_round_trip() {
        for device_scale in [1, 2] {
            for zoom in [0.5, 1.0, 1.562_5, 2.0] {
                let layout = Layout::new(&sizes(), zoom, device_scale);
                for (x, y) in [(0.0, 0.0), (100.0, 200.0), (612.0, 792.0)] {
                    let (cx, cy) = layout.page_to_content(0, x, y);
                    let (px, py) = layout.content_to_page(0, cx, cy);
                    assert!(
                        (px - x).abs() < 1e-2 && (py - y).abs() < 1e-2,
                        "({x}, {y}) came back as ({px}, {py}) at zoom {zoom} scale {device_scale}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_margin_and_the_gap_belong_to_no_page() {
        let mut two = sizes();
        two.push(two[0]);
        let layout = Layout::new(&two, 1.0, 1);
        let (first, second) = (layout.pages[0], layout.pages[1]);
        assert!(first.contains(first.x as f32 + 1.0, first.y as f32 + 1.0));
        assert!(!first.contains(first.x as f32 - 1.0, first.y as f32 + 1.0));

        let gap_y = first.bottom() as f32 + 1.0;
        let inside_x = first.x as f32 + 1.0;
        assert!(!first.contains(inside_x, gap_y));
        assert!(!second.contains(inside_x, gap_y));
        assert_eq!(layout.page_at(gap_y), 1);
    }
}
