use std::collections::HashMap;
use std::ops::Range;

use gtk4::gdk;
use gtk4::prelude::*;

const BUDGET: usize = 128 * 1024 * 1024;

// A MemoryTexture keeps its CPU buffer for its whole lifetime, and GSK holds a separate GPU
// copy that is padded or converted to four bytes per pixel. On an iGPU both live in system
// RAM, so both count against the budget.
const GPU_BYTES_PER_PIXEL: usize = 4;

pub struct Entry {
    pub texture: gdk::Texture,
    pub scale: f32,
    bytes: usize,
    used: u64,
}

pub struct TextureCache {
    entries: HashMap<usize, Entry>,
    clock: u64,
    bytes: usize,
    budget: usize,
}

impl Default for TextureCache {
    fn default() -> Self {
        Self::with_budget(BUDGET)
    }
}

impl TextureCache {
    fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            bytes: 0,
            budget,
        }
    }

    pub fn get(&mut self, page: usize) -> Option<&Entry> {
        if !self.entries.contains_key(&page) {
            return None;
        }
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.get_mut(&page)?;
        entry.used = clock;
        Some(&*entry)
    }

    pub fn peek(&self, page: usize) -> Option<&Entry> {
        self.entries.get(&page)
    }

    /// `cpu_bytes` is the size of the pixel buffer backing `texture`.
    pub fn insert(&mut self, page: usize, texture: gdk::Texture, scale: f32, cpu_bytes: usize) {
        let gpu_bytes =
            (texture.width() as usize) * (texture.height() as usize) * GPU_BYTES_PER_PIXEL;
        let bytes = cpu_bytes + gpu_bytes;
        self.clock += 1;
        if let Some(previous) = self.entries.insert(
            page,
            Entry {
                texture,
                scale,
                bytes,
                used: self.clock,
            },
        ) {
            self.bytes -= previous.bytes;
        }
        self.bytes += bytes;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Evicts least-recently-used pages outside `keep` until the cache fits its budget. Pages
    /// in `keep` are on screen and would be re-rendered at once, so they are never evicted,
    /// even when they alone exceed the budget.
    pub fn trim(&mut self, keep: Range<usize>) {
        while self.bytes > self.budget {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(page, _)| !keep.contains(page))
                .min_by_key(|(_, entry)| entry.used)
                .map(|(page, _)| *page)
            else {
                return;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes -= entry.bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::glib;

    const SIDE: i32 = 10;
    const CPU_BYTES: usize = (SIDE * SIDE * 3) as usize;
    const ENTRY_BYTES: usize = CPU_BYTES + (SIDE * SIDE) as usize * GPU_BYTES_PER_PIXEL;

    fn texture() -> gdk::Texture {
        let bytes = glib::Bytes::from_owned(vec![255u8; CPU_BYTES]);
        gdk::MemoryTexture::new(
            SIDE,
            SIDE,
            gdk::MemoryFormat::R8g8b8,
            &bytes,
            (SIDE * 3) as usize,
        )
        .upcast()
    }

    fn filled(budget: usize, pages: Range<usize>) -> TextureCache {
        let mut cache = TextureCache::with_budget(budget);
        for page in pages {
            cache.insert(page, texture(), 1.0, CPU_BYTES);
        }
        cache
    }

    #[test]
    fn budget_counts_cpu_and_gpu_copies() {
        let cache = filled(BUDGET, 0..1);
        assert_eq!(cache.bytes, ENTRY_BYTES);
    }

    #[test]
    fn replacing_a_page_does_not_double_count() {
        let mut cache = filled(BUDGET, 0..1);
        cache.insert(0, texture(), 2.0, CPU_BYTES);
        assert_eq!(cache.bytes, ENTRY_BYTES);
    }

    #[test]
    fn trim_keeps_visible_pages_over_budget() {
        let mut cache = filled(ENTRY_BYTES / 2, 0..3);
        cache.trim(0..3);
        assert!((0..3).all(|page| cache.peek(page).is_some()));
    }

    #[test]
    fn trim_evicts_least_recently_used_outside_keep() {
        let mut cache = filled(ENTRY_BYTES * 3, 0..4);
        cache.get(0);
        cache.trim(3..4);
        assert!(cache.peek(1).is_none());
        assert!(cache.peek(0).is_some());
        assert!(cache.peek(2).is_some());
        assert!(cache.peek(3).is_some());

        cache.budget = ENTRY_BYTES;
        cache.trim(3..4);
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.peek(3).is_some());
    }
}
