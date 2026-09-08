use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use mupdf::{Colorspace, Context, Document, Matrix, Page, TextPageFlags};

use super::message::{LinkTarget, PageLink, PageSize, RenderJob, RenderedPage, Response};
use super::outline;
use super::text_model::{Char, Line, PageText, Rect};

// MuPDF drops quads past this cap without reporting it, so the count is flagged as partial.
const MAX_PAGE_HITS: u32 = 4096;

#[derive(Debug)]
pub struct SearchJob {
    pub generation: u64,
    pub needle: String,
    pub pages: VecDeque<usize>,
}

#[derive(Debug, Default)]
pub struct Queue {
    pub renders: VecDeque<RenderJob>,
    pub texts: VecDeque<usize>,
    pub links: VecDeque<usize>,
    pub search: Option<SearchJob>,
    pub pings: VecDeque<u64>,
    pub shutdown: bool,
    search_turn: bool,
}

enum Job {
    Render(RenderJob),
    Text(usize),
    Links(usize),
    Search {
        generation: u64,
        needle: String,
        page: usize,
    },
    SearchFinished(u64),
    Ping(u64),
}

impl Queue {
    fn take(&mut self) -> Option<Job> {
        if let Some(job) = self.renders.pop_front() {
            return Some(Job::Render(job));
        }
        // Search and text extraction are both document-wide passes, so a strict order lets
        // either starve the other: a select-all queues every page's text, a find queues every
        // page's search. Alternating keeps both making visible progress.
        self.search_turn = !self.search_turn;
        if self.search_turn
            && let Some(job) = self.take_search()
        {
            return Some(job);
        }
        if let Some(page) = self.texts.pop_front() {
            return Some(Job::Text(page));
        }
        if let Some(page) = self.links.pop_front() {
            return Some(Job::Links(page));
        }
        if let Some(job) = self.take_search() {
            return Some(job);
        }
        self.pings.pop_front().map(Job::Ping)
    }

    fn take_search(&mut self) -> Option<Job> {
        let search = self.search.as_mut()?;
        if let Some(page) = search.pages.pop_front() {
            return Some(Job::Search {
                generation: search.generation,
                needle: search.needle.clone(),
                page,
            });
        }
        let generation = search.generation;
        self.search = None;
        Some(Job::SearchFinished(generation))
    }
}

pub type Shared = (Mutex<Queue>, Condvar);

pub fn run(path: PathBuf, shared: Arc<Shared>, tx: async_channel::Sender<Response>) {
    let mut ctx = Context::get();
    ctx.set_text_aa_level(8);
    ctx.set_graphics_aa_level(8);

    let doc = match Document::open(path.as_path()) {
        Ok(doc) => doc,
        Err(err) => {
            let _ = tx.send_blocking(Response::OpenFailed(err.to_string()));
            return;
        }
    };
    if doc.needs_password().unwrap_or(false) {
        let _ = tx.send_blocking(Response::OpenFailed(
            "The document is password protected.".to_string(),
        ));
        return;
    }

    let sizes = match page_sizes(&doc, &shared) {
        Ok(sizes) => sizes,
        Err(err) => {
            let _ = tx.send_blocking(Response::OpenFailed(err));
            return;
        }
    };
    if shared.0.lock().unwrap().shutdown {
        return;
    }
    let outline = doc
        .outlines()
        .map(|o| outline::flatten(&o))
        .unwrap_or_default();
    if tx
        .send_blocking(Response::Opened {
            pages: sizes,
            outline,
        })
        .is_err()
    {
        return;
    }

    serve(&doc, &shared, &tx);
}

fn serve(doc: &Document, shared: &Arc<Shared>, tx: &async_channel::Sender<Response>) {
    let (lock, condvar) = &**shared;
    loop {
        let job = {
            let mut queue = lock.lock().unwrap();
            loop {
                if queue.shutdown {
                    return;
                }
                if let Some(job) = queue.take() {
                    break job;
                }
                queue = condvar.wait(queue).unwrap();
            }
        };

        let response = match job {
            Job::Render(job) => Some(render(doc, job).map_or(
                Response::RenderFailed { page: job.page },
                Response::Rendered,
            )),
            Job::Text(page) => Some(match extract_text(doc, page) {
                Some(text) => Response::Text { page, text },
                None => Response::TextFailed { page },
            }),
            Job::Links(page) => Some(Response::Links {
                page,
                links: links(doc, page).unwrap_or_default(),
            }),
            Job::Search {
                generation,
                needle,
                page,
            } => search(doc, page, &needle).map(|(rects, truncated)| Response::SearchHits {
                generation,
                page,
                rects,
                truncated,
            }),
            Job::SearchFinished(generation) => Some(Response::SearchFinished { generation }),
            Job::Ping(id) => Some(Response::Idle(id)),
        };

        if let Some(response) = response
            && tx.send_blocking(response).is_err()
        {
            return;
        }
    }
}

fn page_sizes(doc: &Document, shared: &Arc<Shared>) -> Result<Vec<PageSize>, String> {
    let count = doc.page_count().map_err(|e| e.to_string())?;
    let mut sizes = Vec::with_capacity(count as usize);
    for index in 0..count {
        if shared.0.lock().unwrap().shutdown {
            return Err("cancelled".to_string());
        }
        let page = doc.load_page(index).map_err(|e| e.to_string())?;
        let bounds = page.bounds().map_err(|e| e.to_string())?;
        sizes.push(PageSize {
            x0: bounds.x0,
            y0: bounds.y0,
            width: bounds.width(),
            height: bounds.height(),
        });
    }
    if sizes.is_empty() {
        return Err("The document has no pages.".to_string());
    }
    Ok(sizes)
}

fn load(doc: &Document, page: usize) -> Option<Page> {
    doc.load_page(page as i32).ok()
}

fn render(doc: &Document, job: RenderJob) -> Option<RenderedPage> {
    let page = load(doc, job.page)?;
    let matrix = Matrix::new_scale(job.scale, job.scale);
    let pixmap = page
        .to_pixmap(&matrix, &Colorspace::device_rgb(), false, true)
        .ok()?;
    Some(RenderedPage {
        page: job.page,
        scale: job.scale,
        width: pixmap.width() as i32,
        height: pixmap.height() as i32,
        stride: pixmap.stride() as usize,
        pixels: pixmap.samples().to_vec(),
    })
}

fn extract_text(doc: &Document, index: usize) -> Option<PageText> {
    let page = load(doc, index)?;
    let text_page = page.to_text_page(TextPageFlags::ACCURATE_BBOXES).ok()?;
    let structured = text_page.structured();
    let mut lines = Vec::new();
    for (block_index, block) in structured.blocks.iter().enumerate() {
        let mupdf::text_page::TextBlockContent::Text { lines: block_lines } = &block.content else {
            continue;
        };
        for line in block_lines {
            let chars: Vec<Char> = line
                .chars
                .iter()
                .map(|c| Char {
                    rect: quad_rect(&c.quad),
                    ch: c.ch,
                })
                .collect();
            if chars.is_empty() {
                continue;
            }
            lines.push(Line {
                rect: Rect::new(
                    line.bounds.x0,
                    line.bounds.y0,
                    line.bounds.x1,
                    line.bounds.y1,
                ),
                block: block_index,
                chars,
            });
        }
    }
    Some(PageText { lines })
}

fn launchable(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    !rest.is_empty()
        && matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "mailto"
        )
}

fn links(doc: &Document, index: usize) -> Option<Vec<PageLink>> {
    let page = load(doc, index)?;
    let mut out = Vec::new();
    for link in page.links().ok()? {
        let rect = Rect::new(
            link.bounds.x0,
            link.bounds.y0,
            link.bounds.x1,
            link.bounds.y1,
        );
        let target = match &link.dest {
            Some(dest) => LinkTarget::Page(dest.loc.page_number as usize),
            None if launchable(&link.uri) => LinkTarget::Uri(link.uri.clone()),
            None => continue,
        };
        out.push(PageLink { rect, target });
    }
    Some(out)
}

fn search(doc: &Document, index: usize, needle: &str) -> Option<(Vec<Rect>, bool)> {
    let page = load(doc, index)?;
    let quads = page.search(needle, MAX_PAGE_HITS).ok()?;
    let truncated = quads.len() as u32 >= MAX_PAGE_HITS;
    Some((quads.iter().map(quad_rect).collect(), truncated))
}

fn quad_rect(quad: &mupdf::Quad) -> Rect {
    let xs = [quad.ul.x, quad.ur.x, quad.ll.x, quad.lr.x];
    let ys = [quad.ul.y, quad.ur.y, quad.ll.y, quad.lr.y];
    Rect::new(
        xs.iter().copied().fold(f32::INFINITY, f32::min),
        ys.iter().copied().fold(f32::INFINITY, f32::min),
        xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
    )
}

#[cfg(test)]
mod tests {
    use super::launchable;

    #[test]
    fn only_web_and_mail_schemes_are_launchable() {
        assert!(launchable("https://example.org/a"));
        assert!(launchable("HTTP://example.org"));
        assert!(launchable("mailto:someone@example.org"));
        assert!(!launchable("file:///etc/passwd"));
        assert!(!launchable("javascript:alert(1)"));
        assert!(!launchable("smb://host/share"));
        assert!(!launchable("https:"));
        assert!(!launchable("example.org"));
        assert!(!launchable(""));
    }
}
