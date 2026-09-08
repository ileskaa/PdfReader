#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Rect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

#[derive(Clone, Debug)]
pub struct Char {
    pub rect: Rect,
    pub ch: char,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub rect: Rect,
    pub block: usize,
    pub chars: Vec<Char>,
}

impl Line {
    pub fn nearest_boundary(&self, x: f32) -> usize {
        for (i, c) in self.chars.iter().enumerate() {
            if x < (c.rect.x0 + c.rect.x1) * 0.5 {
                return i;
            }
        }
        self.chars.len()
    }

    fn span(&self, from: usize, to: usize) -> &[Char] {
        let len = self.chars.len();
        let (from, to) = (from.min(len), to.min(len));
        if from >= to {
            return &[];
        }
        &self.chars[from..to]
    }

    pub fn span_rect(&self, from: usize, to: usize) -> Rect {
        let mut rect = Rect::default();
        for c in self.span(from, to) {
            rect = rect.union(c.rect);
        }
        if rect.is_empty() {
            return rect;
        }
        Rect::new(rect.x0, self.rect.y0, rect.x1, self.rect.y1)
    }

    pub fn text(&self, from: usize, to: usize) -> String {
        self.span(from, to).iter().map(|c| c.ch).collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct PageText {
    pub lines: Vec<Line>,
}

impl PageText {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}
