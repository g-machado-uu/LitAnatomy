//! DOI lookups on a background thread, so the window never freezes.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use crate::import::doi;
use crate::model::NewPaper;

pub enum NetMsg {
    /// One DOI of batch `batch` finished.
    Fetched { batch: u64, doi: String, result: Result<NewPaper, String> },
    /// Every DOI of the batch has been tried.
    Done { batch: u64 },
}

pub struct Net {
    tx: Sender<NetMsg>,
    rx: Receiver<NetMsg>,
    next_batch: u64,
}

impl Default for Net {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, next_batch: 1 }
    }
}

impl Net {
    /// Looks up `dois` one after another; results arrive through [`Net::poll`].
    pub fn fetch(&mut self, ctx: &egui::Context, dois: Vec<String>, contact_email: String) -> u64 {
        let batch = self.next_batch;
        self.next_batch += 1;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for (i, d) in dois.iter().enumerate() {
                if i > 0 {
                    // Be gentle with the registries when pasting many DOIs.
                    std::thread::sleep(Duration::from_millis(150));
                }
                let result = doi::fetch(d, &contact_email).map_err(|e| e.to_string());
                if tx.send(NetMsg::Fetched { batch, doi: d.clone(), result }).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            let _ = tx.send(NetMsg::Done { batch });
            ctx.request_repaint();
        });
        batch
    }

    pub fn poll(&self) -> Vec<NetMsg> {
        self.rx.try_iter().collect()
    }
}
