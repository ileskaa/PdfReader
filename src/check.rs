use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::document::DocumentHandle;
use crate::document::message::{RenderJob, Response};
use crate::view::layout::Layout;

const ROUNDS: usize = 3;
const PING: u64 = 1;

pub fn run(path: Option<PathBuf>) -> ExitCode {
    let Some(path) = path else {
        eprintln!("usage: pdf-reader --check <file.pdf>");
        return ExitCode::FAILURE;
    };
    let mut first: Option<String> = None;
    for round in 1..=ROUNDS {
        let summary = match round_trip(&path) {
            Ok(summary) => summary,
            Err(message) => {
                eprintln!("round {round} failed: {message}");
                return ExitCode::FAILURE;
            }
        };
        println!("round {round}: {summary}");
        match &first {
            None => first = Some(summary),
            Some(expected) if *expected != summary => {
                eprintln!("round {round} diverged from round 1: {expected:?} != {summary:?}");
                return ExitCode::FAILURE;
            }
            Some(_) => {}
        }
    }
    ExitCode::SUCCESS
}

fn round_trip(path: &Path) -> Result<String, String> {
    let (handle, receiver) = DocumentHandle::open(path.to_path_buf());
    let pages = match receiver.recv_blocking() {
        Ok(Response::Opened { pages, .. }) => pages,
        Ok(Response::OpenFailed(message)) => return Err(message),
        Ok(other) => return Err(format!("unexpected response {other:?}")),
        Err(_) => return Err("worker stopped".to_string()),
    };

    let indices: Vec<usize> = (0..pages.len()).collect();
    handle.request_renders(
        indices
            .iter()
            .map(|&page| RenderJob { page, scale: 1.0 })
            .collect(),
    );
    handle.request_texts(indices.clone());
    handle.request_links(indices.clone());
    handle.search(1, "the".to_string(), indices.clone());
    handle.ping(PING);

    let layout = Layout::new(&pages, 1.0, 1);
    let mut rendered = 0usize;
    let mut failed = Vec::new();
    let mut unreadable = Vec::new();
    let mut chars = 0usize;
    let mut links = 0usize;
    let mut hits = 0usize;
    loop {
        match receiver.recv_blocking() {
            Ok(Response::Rendered(page)) => {
                rendered += 1;
                let rect = layout.pages[page.page];
                if (page.width, page.height) != (rect.width, rect.height) {
                    return Err(format!(
                        "page {} rasterised {}x{} but the layout reserves {}x{}",
                        page.page + 1,
                        page.width,
                        page.height,
                        rect.width,
                        rect.height
                    ));
                }
                std::hint::black_box(&page.pixels);
            }
            Ok(Response::RenderFailed { page }) => failed.push(page + 1),
            Ok(Response::Text { text, .. }) => {
                chars += text
                    .lines
                    .iter()
                    .map(|line| line.chars.len())
                    .sum::<usize>();
            }
            Ok(Response::TextFailed { page }) => unreadable.push(page + 1),
            Ok(Response::Links { links: found, .. }) => links += found.len(),
            Ok(Response::SearchHits { rects, .. }) => hits += rects.len(),
            Ok(Response::Idle(id)) if id == PING => break,
            Ok(_) => {}
            Err(_) => return Err("worker stopped early".to_string()),
        }
    }
    handle.join();
    if !failed.is_empty() {
        return Err(format!("pages failed to render: {failed:?}"));
    }
    if !unreadable.is_empty() {
        return Err(format!("text extraction failed on pages: {unreadable:?}"));
    }
    if rendered != pages.len() {
        return Err(format!("rendered {rendered} of {} pages", pages.len()));
    }
    if chars == 0 {
        return Err("no text was extracted from any page; --check expects a text PDF".to_string());
    }
    Ok(format!(
        "{} pages, {chars} chars, {links} links, {hits} search hits",
        pages.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZOOMS: [f32; 6] = [0.5, 0.75, 1.0, 1.333_333_3, 1.562_5, 2.0];

    #[test]
    fn rasterised_pages_fill_their_layout_rects_exactly() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ILP-at-30.pdf");
        if !path.exists() {
            eprintln!("skipping: {} is not present", path.display());
            return;
        }
        let (handle, receiver) = DocumentHandle::open(path);
        let pages = match receiver.recv_blocking() {
            Ok(Response::Opened { pages, .. }) => pages,
            other => panic!("expected Opened, got {other:?}"),
        };
        let probes = [0usize, pages.len() - 1];

        for device_scale in [1, 2] {
            for zoom in ZOOMS {
                let layout = Layout::new(&pages, zoom, device_scale);
                let scale = zoom * device_scale as f32;
                handle.request_renders(
                    probes
                        .iter()
                        .map(|&page| RenderJob { page, scale })
                        .collect(),
                );
                handle.ping(PING);
                let mut seen = 0;
                loop {
                    match receiver.recv_blocking().expect("worker stopped") {
                        Response::Rendered(rendered) => {
                            seen += 1;
                            let rect = layout.pages[rendered.page];
                            assert_eq!(
                                (rendered.width, rendered.height),
                                (rect.width, rect.height),
                                "page {} at zoom {zoom} scale {device_scale}",
                                rendered.page
                            );
                        }
                        Response::Idle(_) => break,
                        _ => {}
                    }
                }
                assert_eq!(seen, probes.len(), "rendered pages at zoom {zoom}");
            }
        }
    }
}
