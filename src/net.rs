//! Lookups and PDF reading on a background thread, so the window never freezes.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use anyhow::Result;

use crate::import::ident::Identifier;
use crate::import::{http_agent, lookup, pdf};
use crate::model::NewPaper;

/// One thing to look up.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Id(Identifier),
    /// A PDF to read and identify.
    Pdf(PathBuf),
}

impl Job {
    pub fn label(&self) -> String {
        match self {
            Self::Id(id) => id.display(),
            Self::Pdf(path) => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }
}

/// A paper found by a job, with a note on how (PDFs only).
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub paper: NewPaper,
    pub note: Option<String>,
    /// False when the user should check the details.
    pub sure: bool,
}

pub enum NetMsg {
    /// One job of batch `batch` finished.
    Fetched { batch: u64, label: String, result: Result<Found, String> },
    /// Every job of the batch has been tried.
    Done { batch: u64 },
}

pub struct Net {
    tx: Sender<NetMsg>,
    rx: Receiver<NetMsg>,
    next_batch: u64,
    /// Tests: never touch the network (lookups fail as if offline).
    pub offline: bool,
}

impl Default for Net {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, next_batch: 1, offline: false }
    }
}

/// The real services.
struct Web(ureq::Agent);

impl pdf::Online for Web {
    fn lookup(&self, id: &Identifier) -> Result<NewPaper> {
        lookup::lookup(&self.0, id)
    }

    fn search(&self, query: &str) -> Result<Vec<NewPaper>> {
        lookup::search_crossref(&self.0, query)
    }
}

struct Offline;

impl pdf::Online for Offline {
    fn lookup(&self, _: &Identifier) -> Result<NewPaper> {
        anyhow::bail!("offline")
    }

    fn search(&self, _: &str) -> Result<Vec<NewPaper>> {
        anyhow::bail!("offline")
    }
}

impl Net {
    /// Runs `jobs` one after another; results arrive through [`Net::poll`].
    pub fn run(&mut self, ctx: &egui::Context, jobs: Vec<Job>, contact_email: String) -> u64 {
        let batch = self.next_batch;
        self.next_batch += 1;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let offline = self.offline;
        std::thread::spawn(move || {
            let online: Box<dyn pdf::Online> = if offline { Box::new(Offline) } else { Box::new(Web(http_agent(&contact_email))) };
            for (i, job) in jobs.iter().enumerate() {
                if i > 0 && !offline {
                    // Be gentle with the registries when adding many papers.
                    std::thread::sleep(Duration::from_millis(150));
                }
                let result = match job {
                    Job::Id(id) => online
                        .lookup(id)
                        .map(|paper| Found { paper, note: None, sure: true })
                        .map_err(|e| format!("{e:#}")),
                    Job::Pdf(path) => {
                        let f = pdf::identify(path, online.as_ref());
                        Ok(Found { paper: f.paper, note: Some(f.note), sure: f.sure })
                    }
                };
                if tx.send(NetMsg::Fetched { batch, label: job.label(), result }).is_err() {
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
