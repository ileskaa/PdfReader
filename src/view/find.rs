use crate::document::text_model::Rect;

pub fn search_order(current: usize, total: usize) -> Vec<usize> {
    if total == 0 {
        return Vec::new();
    }
    let current = current.min(total - 1);
    (current..total).chain(0..current).collect()
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub page: usize,
    pub rect: Rect,
}

#[derive(Debug, Default)]
pub struct FindState {
    pub needle: String,
    pub generation: u64,
    pub hits: Vec<Hit>,
    pub current: Option<usize>,
    pub searching: bool,
    pub truncated: bool,
}

impl FindState {
    pub fn start(&mut self, needle: String) -> u64 {
        self.generation += 1;
        self.needle = needle;
        self.hits.clear();
        self.current = None;
        self.searching = true;
        self.truncated = false;
        self.generation
    }

    pub fn reset(&mut self) {
        self.generation += 1;
        self.needle.clear();
        self.hits.clear();
        self.current = None;
        self.searching = false;
        self.truncated = false;
    }

    pub fn add(&mut self, page: usize, rects: Vec<Rect>) {
        for rect in rects {
            let hit = Hit { page, rect };
            let at = self.hits.partition_point(|existing| {
                (existing.page, existing.rect.y0, existing.rect.x0)
                    < (hit.page, hit.rect.y0, hit.rect.x0)
            });
            self.hits.insert(at, hit);
            if let Some(current) = self.current.as_mut()
                && at <= *current
            {
                *current += 1;
            }
        }
    }

    pub fn select_nearest(&mut self, page: usize) {
        if self.hits.is_empty() {
            self.current = None;
            return;
        }
        let index = self
            .hits
            .iter()
            .position(|hit| hit.page >= page)
            .unwrap_or(0);
        self.current = Some(index);
    }

    pub fn step(&mut self, forward: bool) -> Option<Hit> {
        if self.hits.is_empty() {
            return None;
        }
        let len = self.hits.len();
        let next = match self.current {
            Some(current) if forward => (current + 1) % len,
            Some(current) => (current + len - 1) % len,
            None => 0,
        };
        self.current = Some(next);
        Some(self.hits[next])
    }

    pub fn current_hit(&self) -> Option<Hit> {
        self.current.and_then(|index| self.hits.get(index)).copied()
    }

    pub fn status(&self) -> String {
        if self.needle.is_empty() {
            return String::new();
        }
        if self.hits.is_empty() {
            return if self.searching {
                "Searching…".to_string()
            } else {
                "No results".to_string()
            };
        }
        let position = self.current.map_or(0, |index| index + 1);
        let more = if self.truncated { "+" } else { "" };
        let suffix = if self.searching { "…" } else { "" };
        format!("{position} of {}{more}{suffix}", self.hits.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(y: f32) -> Rect {
        Rect::new(0.0, y, 10.0, y + 8.0)
    }

    #[test]
    fn current_match_follows_its_hit_when_earlier_pages_arrive() {
        let mut find = FindState::default();
        find.start("x".to_string());
        find.add(10, vec![rect(100.0), rect(200.0)]);
        find.select_nearest(10);
        let selected = find.current_hit().expect("a current hit");
        assert_eq!(find.current, Some(0));

        find.add(0, vec![rect(10.0), rect(20.0), rect(30.0)]);

        let after = find.current_hit().expect("a current hit");
        assert_eq!(
            (after.page, after.rect.y0),
            (selected.page, selected.rect.y0)
        );
        assert_eq!(find.current, Some(3));
    }

    #[test]
    fn search_order_is_visible_first_and_survives_an_unset_current_page() {
        assert_eq!(search_order(2, 5), vec![2, 3, 4, 0, 1]);
        assert_eq!(search_order(0, 3), vec![0, 1, 2]);
        assert!(search_order(usize::MAX, 0).is_empty());
        assert_eq!(search_order(usize::MAX, 3), vec![2, 0, 1]);
    }

    #[test]
    fn a_capped_page_marks_the_match_count_as_partial() {
        let mut find = FindState::default();
        find.start("x".to_string());
        find.add(0, vec![rect(10.0)]);
        find.select_nearest(0);
        find.searching = false;
        assert_eq!(find.status(), "1 of 1");
        find.truncated = true;
        assert_eq!(find.status(), "1 of 1+");
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut find = FindState::default();
        find.start("x".to_string());
        find.add(0, vec![rect(10.0), rect(20.0)]);
        find.select_nearest(0);
        assert_eq!(find.step(true).map(|hit| hit.rect.y0), Some(20.0));
        assert_eq!(find.step(true).map(|hit| hit.rect.y0), Some(10.0));
        assert_eq!(find.step(false).map(|hit| hit.rect.y0), Some(20.0));
    }
}
