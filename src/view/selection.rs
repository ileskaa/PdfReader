use std::collections::HashMap;

use crate::document::text_model::{Line, PageText, Rect};

pub type Texts = HashMap<usize, PageText>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPos {
    pub page: usize,
    pub line: usize,
    pub ch: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    Char,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub granularity: Granularity,
    anchor: (TextPos, TextPos),
    focus: (TextPos, TextPos),
}

impl Selection {
    pub fn new(unit: (TextPos, TextPos), granularity: Granularity) -> Self {
        Self {
            granularity,
            anchor: unit,
            focus: unit,
        }
    }

    pub fn set_focus(&mut self, unit: (TextPos, TextPos)) {
        self.focus = unit;
    }

    pub fn range(&self) -> (TextPos, TextPos) {
        (
            self.anchor.0.min(self.focus.0),
            self.anchor.1.max(self.focus.1),
        )
    }

    pub fn is_empty(&self) -> bool {
        let (start, end) = self.range();
        start == end
    }
}

pub fn hit_test(text: &PageText, page: usize, x: f32, y: f32) -> Option<TextPos> {
    let (index, line) = nearest_line(text, x, y)?;
    Some(TextPos {
        page,
        line: index,
        ch: line.nearest_boundary(x),
    })
}

fn nearest_line(text: &PageText, x: f32, y: f32) -> Option<(usize, &Line)> {
    text.lines.iter().enumerate().min_by(|(_, a), (_, b)| {
        distance(a, x, y)
            .partial_cmp(&distance(b, x, y))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

// Rows before columns. Blending the two into one number needs a weight, and any weight is
// an assumption about page width: on a large-format sheet the horizontal gap outgrows it
// and the hit jumps to the wrong row.
fn distance(line: &Line, x: f32, y: f32) -> (f32, f32) {
    let dy = (line.rect.y0 - y).max(y - line.rect.y1).max(0.0);
    let dx = (line.rect.x0 - x).max(x - line.rect.x1).max(0.0);
    (dy, dx)
}

pub fn expand(texts: &Texts, pos: TextPos, granularity: Granularity) -> (TextPos, TextPos) {
    let Some(line) = texts.get(&pos.page).and_then(|t| t.lines.get(pos.line)) else {
        return (pos, pos);
    };
    match granularity {
        Granularity::Char => (pos, pos),
        Granularity::Word => word_range(line, pos),
        Granularity::Line => (
            TextPos { ch: 0, ..pos },
            TextPos {
                ch: line.chars.len(),
                ..pos
            },
        ),
    }
}

fn word_range(line: &Line, pos: TextPos) -> (TextPos, TextPos) {
    if line.chars.is_empty() {
        return (pos, pos);
    }
    let index = pos.ch.min(line.chars.len() - 1);
    if !is_word(line.chars[index].ch) {
        return (
            TextPos { ch: index, ..pos },
            TextPos {
                ch: index + 1,
                ..pos
            },
        );
    }
    let mut start = index;
    while start > 0 && is_word(line.chars[start - 1].ch) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < line.chars.len() && is_word(line.chars[end].ch) {
        end += 1;
    }
    (TextPos { ch: start, ..pos }, TextPos { ch: end, ..pos })
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

pub fn document_range(texts: &Texts, page_count: usize) -> Option<(TextPos, TextPos)> {
    let first = (0..page_count).find(|p| texts.get(p).is_some_and(|t| !t.is_empty()))?;
    let last = (0..page_count)
        .rev()
        .find(|p| texts.get(p).is_some_and(|t| !t.is_empty()))?;
    let last_text = texts.get(&last)?;
    let last_line = last_text.lines.len().saturating_sub(1);
    Some((
        TextPos {
            page: first,
            line: 0,
            ch: 0,
        },
        TextPos {
            page: last,
            line: last_line,
            ch: last_text.lines.get(last_line).map_or(0, |l| l.chars.len()),
        },
    ))
}

pub fn line_spans(
    text: &PageText,
    page: usize,
    start: TextPos,
    end: TextPos,
) -> Vec<(usize, usize, usize)> {
    if start >= end || page < start.page || page > end.page {
        return Vec::new();
    }
    let first_line = if page == start.page { start.line } else { 0 };
    let last_line = if page == end.page {
        end.line.min(text.lines.len().saturating_sub(1))
    } else {
        text.lines.len().saturating_sub(1)
    };
    let mut spans = Vec::new();
    for index in first_line..=last_line {
        let Some(line) = text.lines.get(index) else {
            continue;
        };
        let from = if page == start.page && index == start.line {
            start.ch
        } else {
            0
        };
        let to = if page == end.page && index == end.line {
            end.ch
        } else {
            line.chars.len()
        };
        if from < to {
            spans.push((index, from, to));
        }
    }
    spans
}

pub fn highlight_rects(text: &PageText, page: usize, start: TextPos, end: TextPos) -> Vec<Rect> {
    line_spans(text, page, start, end)
        .into_iter()
        .filter_map(|(index, from, to)| {
            let rect = text.lines[index].span_rect(from, to);
            (!rect.is_empty()).then_some(rect)
        })
        .collect()
}

pub fn selected_text(texts: &Texts, start: TextPos, end: TextPos) -> String {
    let mut out = String::new();
    let mut previous_block: Option<(usize, usize)> = None;
    for page in start.page..=end.page {
        let Some(text) = texts.get(&page) else {
            continue;
        };
        for (index, from, to) in line_spans(text, page, start, end) {
            let line = &text.lines[index];
            if let Some(previous) = previous_block {
                out.push('\n');
                if previous != (page, line.block) {
                    out.push('\n');
                }
            }
            out.push_str(line.text(from, to).trim_end());
            previous_block = Some((page, line.block));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::text_model::Char;

    fn line(block: usize, y: f32, text: &str) -> Line {
        let chars = text
            .chars()
            .enumerate()
            .map(|(index, ch)| Char {
                rect: Rect::new(index as f32 * 10.0, y, index as f32 * 10.0 + 10.0, y + 10.0),
                ch,
            })
            .collect();
        Line {
            rect: Rect::new(0.0, y, text.chars().count() as f32 * 10.0, y + 10.0),
            block,
            chars,
        }
    }

    fn page(lines: Vec<Line>) -> PageText {
        PageText { lines }
    }

    fn texts() -> Texts {
        let mut texts = Texts::new();
        texts.insert(
            0,
            page(vec![
                line(0, 0.0, "hello big world"),
                line(1, 20.0, "second line"),
            ]),
        );
        texts.insert(1, page(vec![line(0, 0.0, "next page")]));
        texts
    }

    #[test]
    fn hit_test_lands_on_the_nearest_character_boundary() {
        let texts = texts();
        let text = &texts[&0];
        assert_eq!(hit_test(text, 0, 4.0, 5.0).unwrap().ch, 0);
        assert_eq!(hit_test(text, 0, 6.0, 5.0).unwrap().ch, 1);
        assert_eq!(hit_test(text, 0, 1000.0, 5.0).unwrap().ch, 15);
    }

    #[test]
    fn a_wide_horizontal_gap_never_outweighs_a_nearer_row() {
        let text = page(vec![line(0, 0.0, "ab"), line(1, 11.0, &"a".repeat(500))]);
        assert_eq!(hit_test(&text, 0, 6000.0, 10.4).unwrap().line, 0);
    }

    #[test]
    fn hit_test_clamps_to_the_nearest_line_vertically() {
        let texts = texts();
        let text = &texts[&0];
        assert_eq!(hit_test(text, 0, 5.0, -50.0).unwrap().line, 0);
        assert_eq!(hit_test(text, 0, 5.0, 500.0).unwrap().line, 1);
    }

    #[test]
    fn double_click_expands_to_a_word_and_triple_click_to_a_line() {
        let texts = texts();
        let pos = TextPos {
            page: 0,
            line: 0,
            ch: 7,
        };
        let (start, end) = expand(&texts, pos, Granularity::Word);
        assert_eq!((start.ch, end.ch), (6, 9));
        assert_eq!(selected_text(&texts, start, end), "big");

        let (start, end) = expand(&texts, pos, Granularity::Line);
        assert_eq!(selected_text(&texts, start, end), "hello big world");
    }

    #[test]
    fn copying_across_pages_keeps_line_and_block_breaks() {
        let texts = texts();
        let start = TextPos {
            page: 0,
            line: 0,
            ch: 6,
        };
        let end = TextPos {
            page: 1,
            line: 0,
            ch: 4,
        };
        assert_eq!(
            selected_text(&texts, start, end),
            "big world\n\nsecond line\n\nnext"
        );
    }

    #[test]
    fn highlight_merges_a_span_into_one_band_per_line() {
        let texts = texts();
        let start = TextPos {
            page: 0,
            line: 0,
            ch: 6,
        };
        let end = TextPos {
            page: 0,
            line: 1,
            ch: 6,
        };
        let rects = highlight_rects(&texts[&0], 0, start, end);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], Rect::new(60.0, 0.0, 150.0, 10.0));
        assert_eq!(rects[1], Rect::new(0.0, 20.0, 60.0, 30.0));
    }
}
