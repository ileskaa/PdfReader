pub mod message;
pub mod outline;
pub mod text_model;
mod worker;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use message::{RenderJob, Response};
use worker::{Queue, SearchJob, Shared};

pub struct DocumentHandle {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl DocumentHandle {
    pub fn open(path: PathBuf) -> (Self, async_channel::Receiver<Response>) {
        let shared: Arc<Shared> = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let (tx, rx) = async_channel::unbounded();
        let worker_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("pdf-render".to_string())
            .spawn(move || worker::run(path, worker_shared, tx))
            .expect("failed to spawn render thread");
        (
            Self {
                shared,
                thread: Some(thread),
            },
            rx,
        )
    }

    fn update<F: FnOnce(&mut Queue)>(&self, f: F) {
        let (lock, condvar) = &*self.shared;
        let mut queue = lock.lock().unwrap();
        f(&mut queue);
        condvar.notify_one();
    }

    pub fn request_renders(&self, jobs: Vec<RenderJob>) {
        self.update(|queue| queue.renders = jobs.into());
    }

    pub fn request_texts(&self, pages: Vec<usize>) {
        self.update(|queue| queue.texts = pages.into());
    }

    pub fn request_links(&self, pages: Vec<usize>) {
        self.update(|queue| queue.links = pages.into());
    }

    pub fn search(&self, generation: u64, needle: String, pages: Vec<usize>) {
        self.update(|queue| {
            queue.search = Some(SearchJob {
                generation,
                needle,
                pages: VecDeque::from(pages),
            })
        });
    }

    pub fn ping(&self, id: u64) {
        self.update(|queue| queue.pings.push_back(id));
    }

    pub fn cancel_search(&self) {
        self.update(|queue| queue.search = None);
    }

    /// Waits for the render thread to exit. Only for the headless harness, where deterministic
    /// teardown is what makes the leak figures meaningful, and where the queue is already
    /// drained so the wait is immediate.
    pub fn join(mut self) {
        self.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn shutdown(&self) {
        self.update(|queue| {
            queue.shutdown = true;
            queue.renders.clear();
            queue.texts.clear();
            queue.links.clear();
            queue.search = None;
        });
    }
}

impl Drop for DocumentHandle {
    // Detached, not joined. `shutdown` is only observed between jobs, and no MuPDF call can be
    // interrupted mid-flight, so joining here would freeze the UI for the whole of one render,
    // extraction or repair scan. The thread owns its own `Arc` and sender, so it exits on its
    // own at the next job boundary.
    fn drop(&mut self) {
        self.shutdown();
    }
}
