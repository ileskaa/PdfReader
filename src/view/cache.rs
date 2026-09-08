use std::collections::HashMap;

use gtk4::gdk;
use gtk4::prelude::*;

const BUDGET: usize = 128 * 1024 * 1024;

pub struct Entry {
    pub texture: gdk::Texture,
    pub scale: f32,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
pub struct TextureCache {
    entries: HashMap<usize, Entry>,
    clock: u64,
    bytes: usize,
}

impl TextureCache {
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

    pub fn insert(&mut self, page: usize, texture: gdk::Texture, scale: f32) {
        let bytes = (texture.width() as usize) * (texture.height() as usize) * 3;
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
        self.evict();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    fn evict(&mut self) {
        while self.bytes > BUDGET && self.entries.len() > 1 {
            let Some(oldest) = self
                .entries
                .iter()
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
