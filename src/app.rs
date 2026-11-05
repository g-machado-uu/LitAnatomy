//! The application: navigation, cached lists, background lookups, autosave
//! and auto-export. Each screen draws itself in `ui/*` through `impl Session`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{RichText, Ui};
use egui_commonmark::CommonMarkCache;

use crate::db::{Library, Scope};
use crate::export;
use crate::import;
use crate::model::{Collection, NewPaper, PaperRow, ReadingStatus};
use crate::net::{Job, Net, NetMsg};
use crate::pdf;
use crate::settings::{self, Settings};
use crate::ui::dialogs::{Candidate, Dialog, ImportState};
use crate::ui::list::ListState;
use crate::ui::matrix::MatrixState;
use crate::ui::paper::PaperPage;
use crate::ui::settings::SettingsForm;
use crate::ui::{icons, theme};

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Home,
    List(Scope),
    Paper(i64),
    Matrix(Scope),
    /// A collection's synthesis: themes and a related-work outline.
    Synthesis(i64),
    Settings,
}

/// What a batch of lookups is for.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    /// One typed or pasted identifier: added straight away.
    Quick { target: Option<i64> },
    /// Results go to the import preview dialog.
    Preview,
    /// Refresh a paper's metadata from its DOI, arXiv ID or ISBN.
    Refresh(i64),
}

/// Lists for the sidebar and dashboard, rebuilt when the library changes.
#[derive(Default)]
pub struct Cache {
    pub rev: u64,
    pub collections: Vec<Collection>,
    pub stats: HashMap<i64, (usize, usize)>,
    pub tags: Vec<(i64, String, usize)>,
    pub trash: usize,
    pub all: Vec<PaperRow>,
    pub unfiled: usize,
}

#[derive(Default)]
pub struct AutoExport {
    pub done_rev: u64,
    pub pending_since: Option<f64>,
    pub last_error: Option<String>,
}

/// Payload of a paper dragged from a list onto a collection.
#[derive(Clone)]
pub struct PaperDrag(pub Vec<i64>);

pub struct Toast {
    pub text: String,
    pub at: f64,
    pub error: bool,
}

pub struct Session {
    pub lib: Library,
    pub settings: Settings,
    pub settings_path: Option<PathBuf>,
    pub view: View,
    /// Where "Back" goes from a paper.
    pub back: Vec<View>,
    pub cache: Cache,
    pub list: ListState,
    pub page: Option<PaperPage>,
    pub matrix: MatrixState,
    pub dialog: Option<Dialog>,
    pub net: Net,
    pub lookups: HashMap<u64, Lookup>,
    pub toast: Option<Toast>,
    pub auto: AutoExport,
    pub md_cache: CommonMarkCache,
    pub quick_doi: String,
    pub synthesis: Option<crate::ui::synthesis::SynthesisPage>,
    pub settings_form: SettingsForm,
    pub archived_open: bool,
    /// Set by the UI when something needs the time (autosave), read in `logic`.
    pub now: f64,
    /// A widget had keyboard focus at the end of the last frame. egui drops
    /// focus on Esc before the app sees the key, so "was the user typing?"
    /// has to be asked of the previous frame.
    pub had_focus: bool,
}

pub enum Screen {
    Ready(Box<Session>),
    /// The library could not be opened.
    Failed { error: String, settings: Settings, settings_path: Option<PathBuf> },
}

pub struct LitAnatomyApp {
    pub screen: Screen,
}

impl LitAnatomyApp {
    pub fn new(cc: &eframe::CreationContext<'_>, library: Option<PathBuf>) -> Self {
        let settings_path = settings::default_settings_path();
        let settings = settings_path.as_deref().map(Settings::load).unwrap_or_default();
        theme::install(&cc.egui_ctx, &settings);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        Self::with_settings(settings, settings_path, library)
    }

    /// Also used by tests, with a temporary library and no settings file.
    pub fn with_settings(mut settings: Settings, settings_path: Option<PathBuf>, library: Option<PathBuf>) -> Self {
        if let Some(path) = library {
            settings.library_path = Some(path);
        }
        let path = settings.library_path();
        let screen = match Library::open(&path) {
            Ok(lib) => Screen::Ready(Box::new(Session::new(lib, settings, settings_path))),
            Err(e) => Screen::Failed { error: format!("{e:#}"), settings, settings_path },
        };
        Self { screen }
    }
}

impl Session {
    pub fn new(lib: Library, settings: Settings, settings_path: Option<PathBuf>) -> Self {
        let settings_form = SettingsForm::from(&settings);
        let mut s = Self {
            auto: AutoExport { done_rev: lib.revision(), ..Default::default() },
            lib,
            settings,
            settings_path,
            view: View::Home,
            back: Vec::new(),
            cache: Cache::default(),
            list: ListState::default(),
            page: None,
            matrix: MatrixState::default(),
            dialog: None,
            net: Net::default(),
            lookups: HashMap::new(),
            toast: None,
            md_cache: CommonMarkCache::default(),
            quick_doi: String::new(),
            synthesis: None,
            settings_form,
            archived_open: false,
            now: 0.0,
            had_focus: false,
        };
        s.refresh_cache();
        s
    }

    // ----- Library cache ---------------------------------------------------

    pub fn refresh_cache(&mut self) {
        let rev = self.lib.revision();
        if self.cache.rev == rev {
            return;
        }
        let collections = self.lib.collections();
        let stats = self.lib.collection_stats();
        let tags = self.lib.tags();
        let trash = self.lib.trash_count();
        let all = self.lib.rows(Scope::All);
        let unfiled = self.lib.rows(Scope::Unfiled).map(|r| r.len());
        match (collections, stats, tags, trash, all, unfiled) {
            (Ok(collections), Ok(stats), Ok(tags), Ok(trash), Ok(all), Ok(unfiled)) => {
                self.cache = Cache { rev, collections, stats, tags, trash, all, unfiled };
            }
            (a, b, c, d, e, f) => {
                let err = [a.err(), b.err(), c.err(), d.err(), e.err(), f.err()].into_iter().flatten().next();
                if let Some(e) = err {
                    self.error(format!("Could not read the library: {e:#}"));
                }
                self.cache.rev = rev;
            }
        }
        if let Some(page) = &mut self.page {
            page.reload_meta(&self.lib);
        }
    }

    pub fn collection(&self, id: i64) -> Option<&Collection> {
        self.cache.collections.iter().find(|c| c.id == id)
    }

    pub fn scope_title(&self, scope: Scope) -> String {
        match scope {
            Scope::All => "All papers".into(),
            Scope::Unfiled => "Unfiled".into(),
            Scope::Trash => "Trash".into(),
            Scope::Collection(id) => self.collection(id).map(|c| c.name.clone()).unwrap_or_else(|| "Collection".into()),
        }
    }

    // ----- Messages ---------------------------------------------------------

    pub fn info(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), at: self.now, error: false });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), at: self.now, error: true });
    }

    /// Shows the error of a failed library call; returns the value otherwise.
    pub fn check<T>(&mut self, r: anyhow::Result<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.error(format!("{e:#}"));
                None
            }
        }
    }

    // ----- Navigation -------------------------------------------------------

    /// Goes to a top-level view (from the sidebar): forgets the back stack.
    pub fn go(&mut self, view: View) {
        self.flush_page(true);
        self.leave_synthesis();
        self.back.clear();
        if self.view != view {
            self.list.selected.clear();
        }
        self.page = None;
        self.view = view;
    }

    pub fn open_paper(&mut self, id: i64) {
        self.flush_page(true);
        let context = match &self.view {
            View::List(Scope::Collection(c)) | View::Matrix(Scope::Collection(c)) | View::Synthesis(c) => Some(*c),
            View::Paper(_) => self.page.as_ref().and_then(|p| p.context),
            _ => None,
        };
        self.leave_synthesis();
        match PaperPage::load(&self.lib, id, context) {
            Ok(page) => {
                if !matches!(self.view, View::Paper(p) if p == id) {
                    self.back.push(self.view.clone());
                }
                self.page = Some(page);
                self.view = View::Paper(id);
            }
            Err(e) => self.error(format!("Could not open the paper: {e:#}")),
        }
    }

    pub fn go_back(&mut self) {
        self.flush_page(true);
        self.leave_synthesis();
        let view = self.back.pop().unwrap_or(View::Home);
        self.page = None;
        if let View::Paper(id) = view {
            self.open_paper(id);
            self.back.pop();
        } else {
            self.view = view;
        }
    }

    /// Saves typed notes: all of them, or only those left alone for a moment.
    pub fn flush_page(&mut self, all: bool) {
        let now = self.now;
        if let Some(page) = &mut self.page {
            let result = page.flush(&mut self.lib, if all { None } else { Some(now) });
            if let Err(e) = result {
                self.error(format!("Could not save your notes: {e:#}"));
            }
        }
    }

    // ----- Adding papers ----------------------------------------------------

    /// The collection new papers go to: the one on screen, if any.
    pub fn current_collection(&self) -> Option<i64> {
        match &self.view {
            View::List(Scope::Collection(c)) | View::Matrix(Scope::Collection(c)) | View::Synthesis(c) => Some(*c),
            View::Paper(_) => self.page.as_ref().and_then(|p| p.context),
            _ => None,
        }
    }

    /// Looks up the identifiers (DOIs, arXiv IDs, ISBNs, PubMed IDs) in
    /// `text`. One is added straight away; several open the import preview.
    pub fn lookup_ids(&mut self, ctx: &egui::Context, text: &str, target: Option<i64>) -> bool {
        // Paths of files (e.g. copied in a file manager) are added as files.
        let files = file_paths(text);
        if !files.is_empty() {
            self.add_files(ctx, files);
            return true;
        }
        let ids = import::ident::find_all(text);
        if ids.is_empty() {
            self.error("No identifier found. Paste a DOI (10.1109/…), an arXiv ID (2301.12345), an ISBN or a PubMed ID.");
            return false;
        }
        let count = ids.len();
        let what = ids[0].kind();
        let jobs = ids.into_iter().map(Job::Id).collect();
        let batch = self.net.run(ctx, jobs, self.settings.contact_email.clone());
        if count == 1 {
            self.lookups.insert(batch, Lookup::Quick { target });
            self.info(format!("Looking up the {what}…"));
        } else {
            self.lookups.insert(batch, Lookup::Preview);
            let mut state = self.new_import(format!("{count} identifiers"), None, target);
            state.pending = Some((batch, count));
            self.dialog = Some(Dialog::Import(state));
        }
        true
    }

    /// Reads PDFs, finds which papers they are, and opens the import preview.
    /// Each paper keeps a link to its PDF.
    pub fn import_pdfs(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, target: Option<i64>) {
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        let jobs = paths.into_iter().map(Job::Pdf).collect();
        let batch = self.net.run(ctx, jobs, self.settings.contact_email.clone());
        self.lookups.insert(batch, Lookup::Preview);
        let mut state = self.new_import(crate::ui::sidebar::plural(count, "PDF"), None, target);
        state.pending = Some((batch, count));
        state.reading_pdfs = true;
        self.dialog = Some(Dialog::Import(state));
    }

    pub fn pick_pdfs(&mut self, ctx: &egui::Context) {
        let target = self.current_collection();
        let mut dialog = rfd::FileDialog::new().set_title("Add papers from their PDFs").add_filter("PDF", &["pdf"]);
        if let Some(folder) = &self.settings.pdf_folder {
            dialog = dialog.set_directory(folder);
        }
        if let Some(paths) = dialog.pick_files() {
            self.import_pdfs(ctx, paths, target);
        }
    }

    fn new_import(&self, source: String, base_dir: Option<PathBuf>, target: Option<i64>) -> ImportState {
        ImportState::new(source, base_dir, target, self.settings.keep_readable_keys)
    }

    fn handle_net(&mut self) {
        for msg in self.net.poll() {
            match msg {
                NetMsg::Fetched { batch, label, result } => match self.lookups.get(&batch).cloned() {
                    Some(Lookup::Quick { target }) => match result {
                        Ok(found) => self.add_single(found.paper, target),
                        Err(e) => self.error(e),
                    },
                    Some(Lookup::Preview) => {
                        if let Some(Dialog::Import(state)) = &mut self.dialog
                            && state.pending.is_some_and(|(b, _)| b == batch)
                        {
                            match result {
                                Ok(found) => state.candidates.push(Candidate::found(found)),
                                Err(e) => state.errors.push(format!("{label}: {e}")),
                            }
                            if let Some((_, left)) = &mut state.pending {
                                *left = left.saturating_sub(1);
                            }
                            state.refresh(&self.lib, &self.settings.citekey_pattern);
                        }
                    }
                    Some(Lookup::Refresh(id)) => match result {
                        Ok(found) => {
                            if let Some(page) = &mut self.page
                                && page.id == id
                            {
                                page.apply_lookup(found.paper.bib);
                                self.info(format!("Details filled in from {label}. Review them and press Save."));
                            }
                        }
                        Err(e) => self.error(e),
                    },
                    None => {}
                },
                NetMsg::Done { batch } => {
                    self.lookups.remove(&batch);
                    if let Some(Dialog::Import(state)) = &mut self.dialog
                        && state.pending.is_some_and(|(b, _)| b == batch)
                    {
                        state.pending = None;
                    }
                }
            }
        }
    }

    /// Adds one looked-up paper, or points at the copy already in the library.
    fn add_single(&mut self, paper: NewPaper, target: Option<i64>) {
        let existing = self.lib.find_duplicate(&paper.bib);
        let Some(existing) = self.check(existing) else { return };
        let id = match existing {
            Some(id) => {
                let r = self.lib.restore(&[id]);
                self.check(r);
                self.info("That paper is already in your library.");
                id
            }
            None => {
                let added = self.lib.add_paper(&paper, &self.settings.citekey_pattern);
                let Some(id) = self.check(added) else { return };
                let key = self.lib.paper(id).map(|p| p.citekey).unwrap_or_default();
                self.info(format!("Added {key}."));
                id
            }
        };
        if let Some(c) = target {
            let r = self.lib.add_to_collection(c, &[id]);
            self.check(r);
        }
        self.refresh_cache();
        self.open_paper(id);
    }

    /// Reads a reference file and opens the import preview.
    pub fn import_file(&mut self, path: &Path, target: Option<i64>) {
        match import::read_file(path) {
            Ok((format, parsed)) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut state = self.new_import(format!("{name} ({})", format.label()), path.parent().map(Path::to_path_buf), target);
                state.errors = parsed.errors;
                state.candidates = parsed.papers.into_iter().map(Candidate::new).collect();
                state.refresh(&self.lib, &self.settings.citekey_pattern);
                if state.candidates.is_empty() && state.errors.is_empty() {
                    self.error(format!("No references found in {name}."));
                } else {
                    self.dialog = Some(Dialog::Import(state));
                }
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    /// Imports the papers whose PDFs Zotero has downloaded, from Zotero's
    /// data folder in its usual place, or one the user chooses.
    pub fn import_zotero(&mut self) {
        let dir = match import::zotero::default_dir() {
            Some(dir) => dir,
            None => match rfd::FileDialog::new().set_title("Choose Zotero's data folder (the one with \"storage\" in it)").pick_folder() {
                Some(dir) => dir,
                None => return,
            },
        };
        self.import_zotero_from(&dir, self.current_collection());
    }

    pub fn import_zotero_from(&mut self, dir: &Path, target: Option<i64>) {
        match import::zotero::read(dir) {
            Ok(zotero) => {
                let source = format!("Zotero ({})", dir.display());
                let mut state = self.new_import(source, None, target);
                if zotero.without_pdf > 0 {
                    let items = crate::ui::sidebar::plural(zotero.without_pdf, "other item");
                    state.note = Some(format!("Only papers with a downloaded PDF are imported. Left out: {items} in Zotero."));
                }
                state.errors = zotero.parsed.errors;
                state.candidates = zotero.parsed.papers.into_iter().map(Candidate::new).collect();
                state.refresh(&self.lib, &self.settings.citekey_pattern);
                if state.candidates.is_empty() && state.errors.is_empty() {
                    self.error("Zotero has no downloaded PDFs to import. Download them in Zotero first.");
                } else {
                    self.dialog = Some(Dialog::Import(state));
                }
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    pub fn pick_import_file(&mut self) {
        let target = self.current_collection();
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Import references")
            .add_filter("References", import::EXTENSIONS)
            .add_filter("All files", &["*"])
            .pick_file()
        {
            self.import_file(&path, target);
        }
    }

    /// A blank entry to fill in by hand; opens its Metadata tab.
    pub fn new_blank_paper(&mut self) {
        let mut paper = NewPaper::default();
        paper.bib.entry_type = "article".into();
        paper.bib.title = "Untitled paper".into();
        let added = self.lib.add_paper(&paper, &self.settings.citekey_pattern);
        let Some(id) = self.check(added) else { return };
        if let Some(c) = self.current_collection() {
            let r = self.lib.add_to_collection(c, &[id]);
            self.check(r);
        }
        self.refresh_cache();
        self.open_paper(id);
        if let Some(page) = &mut self.page {
            page.tab = crate::ui::paper::PaperTab::Metadata;
        }
    }

    // ----- PDFs -------------------------------------------------------------

    pub fn pdf_path(&self, stored: &str) -> PathBuf {
        pdf::resolve(stored, self.settings.pdf_folder.as_deref())
    }

    pub fn attach_pdf(&mut self, paper: i64, file: &Path) {
        let stored = pdf::to_stored(file, self.settings.pdf_folder.as_deref());
        let r = self.lib.set_pdf(paper, Some(&stored));
        if self.check(r).is_some() {
            self.info("PDF linked.");
        }
    }

    pub fn open_pdf(&mut self, stored: &str) {
        let path = self.pdf_path(stored);
        let r = pdf::open_file(&path);
        self.check(r);
    }

    // ----- Export -----------------------------------------------------------

    /// Asks where to save, writes the .bib and pins the exported keys.
    pub fn export_bib(&mut self, ids: &[i64], suggested: &str) {
        let papers = self.lib.papers(ids);
        let Some(papers) = self.check(papers) else { return };
        if papers.is_empty() {
            self.error("There are no papers to export.");
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export BibTeX")
            .set_file_name(format!("{}.bib", safe_file_name(suggested)))
            .add_filter("BibTeX", &["bib"])
            .save_file()
        else {
            return;
        };
        let text = export::bibtex::bibliography(&papers, self.settings.export_options());
        match export::write_if_changed(&path, &text) {
            Ok(_) => {
                let r = self.lib.pin_citekeys(ids);
                self.check(r);
                self.info(format!("Exported {} references to {}.", papers.len(), path.display()));
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    /// Writes every collection's .bib (when auto-export is on), a moment after the last change.
    fn auto_export(&mut self, now: f64) {
        if !self.settings.auto_export || self.lib.revision() == self.auto.done_rev {
            self.auto.pending_since = None;
            return;
        }
        let since = *self.auto.pending_since.get_or_insert(now);
        if now - since < 1.5 {
            return;
        }
        self.auto.pending_since = None;
        match self.run_auto_export() {
            Ok(_) => self.auto.last_error = None,
            Err(e) => {
                let msg = format!("Auto-export failed: {e:#}");
                if self.auto.last_error.as_deref() != Some(msg.as_str()) {
                    self.error(msg.clone());
                }
                self.auto.last_error = Some(msg);
            }
        }
        self.auto.done_rev = self.lib.revision();
    }

    /// Returns how many files changed.
    pub fn run_auto_export(&mut self) -> anyhow::Result<usize> {
        let mut written = 0;
        let targets: Vec<(i64, String)> = self
            .lib
            .collections()?
            .into_iter()
            .filter_map(|c| c.bib_export_path.map(|p| (c.id, p)))
            .collect();
        let mut first_error = None;
        for (id, path) in targets {
            let papers = self.lib.export_papers(id)?;
            let text = export::bibtex::bibliography(&papers, self.settings.export_options());
            match export::write_if_changed(Path::new(&path), &text) {
                Ok(changed) => {
                    written += usize::from(changed);
                    let unpinned: Vec<i64> = papers.iter().filter(|p| !p.key_pinned).map(|p| p.id).collect();
                    if !unpinned.is_empty() {
                        self.lib.pin_citekeys(&unpinned)?;
                    }
                }
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(written),
        }
    }

    // ----- Settings ---------------------------------------------------------

    pub fn save_settings(&mut self, ctx: &egui::Context) {
        theme::apply(ctx, &self.settings);
        if let Some(path) = &self.settings_path {
            let r = self.settings.save(path);
            self.check(r);
        }
        // A newly turned-on auto-export should run now.
        self.auto.done_rev = 0;
    }

    /// Opens another library file (or creates it), keeping the settings.
    pub fn switch_library(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.flush_page(true);
        match Library::open(&path) {
            Ok(lib) => {
                let mut settings = self.settings.clone();
                settings.library_path = Some(path.clone());
                let settings_path = self.settings_path.clone();
                *self = Session::new(lib, settings, settings_path);
                self.view = View::Settings;
                self.save_settings(ctx);
                self.info(format!("Opened {}.", path.display()));
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    // ----- Frame ------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut Ui) {
        self.now = ui.input(|i| i.time);
        self.refresh_cache();
        self.handle_dropped_files(ui);
        self.shortcuts(ui);
        self.sidebar(ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(egui::Margin { left: 28, right: 28, top: 20, bottom: 12 }))
            .show(ui, |ui| match self.view.clone() {
                View::Home => self.home_ui(ui),
                View::List(scope) => self.list_ui(ui, scope),
                View::Paper(_) => self.paper_ui(ui),
                View::Matrix(scope) => self.matrix_ui(ui, scope),
                View::Synthesis(id) => self.synthesis_ui(ui, id),
                View::Settings => self.settings_ui(ui),
            });
        self.dialogs(ui);
        self.toast_ui(ui);
        self.drop_hint(ui);
        self.had_focus = ui.memory(|m| m.focused().is_some());
    }

    pub fn logic(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.now = now;
        self.handle_net();
        self.flush_page(false);
        self.flush_synthesis(false);
        self.auto_export(now);
        let unsaved = self.page.as_ref().is_some_and(|p| p.has_unsaved()) || self.synthesis.as_ref().is_some_and(|s| s.has_unsaved());
        if unsaved || self.auto.pending_since.is_some() {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.flush_page(true);
            self.flush_synthesis(true);
            if self.settings.auto_export && self.lib.revision() != self.auto.done_rev {
                let r = self.run_auto_export();
                self.check(r);
            }
        }
    }

    /// Keyboard shortcuts (the list is in [`crate::ui::dialogs`], shown with F1).
    fn shortcuts(&mut self, ui: &mut Ui) {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            match self.dialog {
                Some(Dialog::Shortcuts) => self.dialog = None,
                None => self.dialog = Some(Dialog::Shortcuts),
                _ => {}
            }
        }
        if let (View::Paper(_), Some(page)) = (&self.view, &mut self.page) {
            page.handle_tab(ui, self.dialog.is_none());
        }
        // Dialogs handle their own keys (Esc closes them, Ctrl+Enter confirms some).
        if self.dialog.is_some() {
            return;
        }
        let ctx = ui.ctx().clone();
        let focused = ctx.memory(|m| m.focused());
        // Ctrl+Enter: stop editing the field (a filled-in note shows formatted).
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter))
            && let Some(id) = focused
        {
            ctx.memory_mut(|m| m.surrender_focus(id));
            if let Some(page) = &mut self.page {
                page.finish_editing(id);
            }
            self.flush_page(true);
            self.flush_synthesis(true);
        }
        let trash = |i: &mut egui::InputState| {
            let m = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
            i.consume_key(m, egui::Key::Delete) || i.consume_key(m, egui::Key::Backspace)
        };
        if let View::Paper(id) = self.view
            && !self.had_focus
            && focused.is_none()
            && ui.input_mut(trash)
        {
            if self.page.as_ref().is_some_and(|p| p.paper.deleted) {
                self.info("This paper is already in the Trash.");
            } else {
                self.dialog = Some(Dialog::ConfirmTrash(id));
            }
        }
        // Esc: the first one leaves a field (egui does that), the next goes back a level.
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if esc && !self.had_focus && focused.is_none() && !egui::Popup::is_any_open(&ctx) {
            self.escape();
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            if !matches!(self.view, View::List(_)) {
                self.go(View::List(Scope::All));
            }
            self.list.focus_search = true;
        }
    }

    /// Esc: from a paper back to where it was opened, from a matrix or
    /// synthesis to its list, from a list (with nothing selected) to Home.
    pub fn escape(&mut self) {
        match self.view.clone() {
            View::Paper(_) => self.go_back(),
            View::Matrix(scope) => self.go(View::List(scope)),
            View::Synthesis(c) => self.go(View::List(Scope::Collection(c))),
            View::List(_) if !self.list.selected.is_empty() => self.list.selected.clear(),
            View::List(_) | View::Settings => self.go(View::Home),
            View::Home => {}
        }
    }

    /// Moves the paper on screen to the Trash and goes back.
    pub fn trash_open_paper(&mut self, id: i64) {
        let r = self.lib.trash(&[id]);
        if self.check(r).is_some() {
            self.info("Moved to the Trash. It can be restored from there.");
            if self.view == View::Paper(id) {
                self.go_back();
            }
        }
    }

    /// A PDF dropped on an open paper is linked to it. Other PDFs (and
    /// folders of them) are imported as papers; reference files are imported.
    fn handle_dropped_files(&mut self, ui: &mut Ui) {
        let mut dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        // Files copied in a file manager and pasted (Ctrl+V) with no field
        // focused: the same as dropping them. Works where dropping can't (Wayland).
        if !self.had_focus && self.dialog.is_none() && ui.memory(|m| m.focused().is_none()) {
            let pasted: Vec<String> = ui.input(|i| {
                i.events.iter().filter_map(|e| if let egui::Event::Paste(t) = e { Some(t.clone()) } else { None }).collect()
            });
            for text in pasted {
                dropped.extend(file_paths(&text));
            }
        }
        self.add_files(ui.ctx(), dropped);
    }

    /// Files dropped or pasted: PDFs become papers (one on an open paper is
    /// linked to it), folders are searched for PDFs, reference files imported.
    pub fn add_files(&mut self, ctx: &egui::Context, dropped: Vec<PathBuf>) {
        if dropped.is_empty() {
            return;
        }
        let is_pdf = |p: &Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        let mut pdfs = Vec::new();
        let mut others = Vec::new();
        let mut folders = false;
        for path in dropped {
            if path.is_dir() {
                folders = true;
                pdfs.extend(pdfs_in(&path));
            } else if is_pdf(&path) {
                pdfs.push(path);
            } else {
                others.push(path);
            }
        }
        let target = self.current_collection();
        if let (View::Paper(id), [one], false) = (&self.view, pdfs.as_slice(), folders) {
            let (id, one) = (*id, one.clone());
            self.attach_pdf(id, &one);
        } else if !pdfs.is_empty() {
            self.import_pdfs(ctx, pdfs, target);
        } else if folders && others.is_empty() {
            self.error("No PDFs found in that folder.");
        }
        for path in others {
            self.import_file(&path, target);
        }
    }

    /// While files are dragged over the window: what dropping them will do.
    /// Home has its own drop zone, which lights up instead.
    fn drop_hint(&self, ui: &mut Ui) {
        let hovered: Vec<PathBuf> = ui.input(|i| i.raw.hovered_files.iter().filter_map(|f| f.path.clone()).collect());
        let any = ui.input(|i| !i.raw.hovered_files.is_empty());
        if !any || (self.view == View::Home && self.dialog.is_none()) {
            return;
        }
        let pdfs = hovered.iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))).count();
        let text = match (&self.view, pdfs, hovered.len()) {
            (View::Paper(_), 1, 1) => "Drop to link this PDF to the paper".to_string(),
            (_, n, total) if n > 0 && n == total => format!("Drop to add {} from {}", if n == 1 { "the paper" } else { "the papers" }, if n == 1 { "its PDF" } else { "their PDFs" }),
            _ => "Drop PDFs to add papers from them, or a .bib, .ris or .json file to import it".to_string(),
        };
        let rect = ui.ctx().content_rect();
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop-hint")));
        let t = theme::t(ui);
        painter.rect_filled(rect, 0.0, t.sidebar.gamma_multiply(0.85));
        painter.rect_stroke(rect.shrink(16.0), egui::CornerRadius::same(18), egui::Stroke::new(2.5, t.accent), egui::StrokeKind::Inside);
        painter.text(rect.center() - egui::vec2(0.0, 26.0), egui::Align2::CENTER_CENTER, icons::FILE_PDF, egui::FontId::proportional(46.0), t.accent);
        painter.text(rect.center() + egui::vec2(0.0, 22.0), egui::Align2::CENTER_CENTER, text, theme::semibold_font(19.0), t.ink);
    }

    fn toast_ui(&mut self, ui: &mut Ui) {
        let Some(toast) = &self.toast else { return };
        let age = self.now - toast.at;
        let life = if toast.error { 8.0 } else { 4.0 };
        if age >= life || age < 0.0 {
            self.toast = None;
            return;
        }
        ui.ctx().request_repaint_after(Duration::from_millis(250));
        let (icon, colour) = if toast.error { (icons::WARNING, theme::RED) } else { (icons::CHECK_CIRCLE, theme::GREEN) };
        let text = toast.text.clone();
        let mut close = false;
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-24.0, -24.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(520.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon).size(16.0).color(theme::fg(ui, colour)));
                            ui.add(egui::Label::new(text).wrap());
                            if ui.add(theme::ghost_button(icons::X)).on_hover_text("Dismiss").clicked() {
                                close = true;
                            }
                        });
                    });
            });
        if close {
            self.toast = None;
        }
    }

    /// Status counts over all papers, for the dashboard.
    pub fn status_counts(&self) -> HashMap<ReadingStatus, usize> {
        let mut counts = HashMap::new();
        for r in &self.cache.all {
            *counts.entry(r.status).or_insert(0) += 1;
        }
        counts
    }

    pub fn papers_with_tag(&self, tag: i64) -> HashSet<i64> {
        self.lib.papers_with_tag(tag).unwrap_or_default()
    }
}

/// The files in pasted text: one absolute path or file:// URI per line,
/// each of which exists. Anything else in the text means it isn't a file list.
pub fn file_paths(text: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let raw = match line.strip_prefix("file://") {
            // file:///home/… (or file://localhost/home/…), percent-encoded.
            Some(rest) => percent_decode(rest.strip_prefix("localhost").unwrap_or(rest)),
            None => line.trim_matches(|c| c == '"' || c == '\'').to_string(),
        };
        // Windows URIs look like file:///C:/Papers/x.pdf.
        let raw = match raw.as_bytes() {
            [b'/', d, b':', ..] if d.is_ascii_alphabetic() && cfg!(windows) => raw[1..].to_string(),
            _ => raw,
        };
        let path = PathBuf::from(&raw);
        if !path.is_absolute() || !path.exists() {
            return Vec::new();
        }
        out.push(path);
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The PDFs in a folder and its sub-folders (at most 500).
fn pdfs_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) && out.len() < 500 {
                out.push(p);
            }
        }
    }
    out
}

pub fn safe_file_name(s: &str) -> String {
    let cleaned: String =
        s.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') { c } else { '_' }).collect();
    let cleaned = cleaned.trim().replace(' ', "-");
    if cleaned.is_empty() { "references".into() } else { cleaned }
}

impl eframe::App for LitAnatomyApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Screen::Ready(s) = &mut self.screen {
            s.logic(ctx);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let mut replace = None;
        match &mut self.screen {
            Screen::Ready(s) => s.ui(ui),
            Screen::Failed { error, settings, settings_path } => {
                if let Some(next) = failed_ui(ui, error, settings, settings_path) {
                    replace = Some(next);
                }
            }
        }
        if let Some(screen) = replace {
            self.screen = screen;
        }
    }
}

/// Shown when the library can't be opened: pick another or start a new one.
fn failed_ui(ui: &mut Ui, error: &mut String, settings: &mut Settings, settings_path: &Option<PathBuf>) -> Option<Screen> {
    let mut next = None;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(560.0);
            theme::card(ui, "", |ui| {
                ui.label(theme::serif("The library could not be opened", 26.0));
                ui.add_space(6.0);
                ui.label(RichText::new(error.as_str()).color(theme::fg(ui, theme::RED)));
                ui.add_space(4.0);
                ui.label(RichText::new(settings.library_path().display().to_string()).small().color(theme::muted(ui)));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let mut chosen = None;
                    if ui.add(theme::primary_button(ui, format!("{} Open another library…", icons::FOLDER_OPEN))).clicked() {
                        chosen = rfd::FileDialog::new()
                            .add_filter("LitAnatomy library", &["litanatomy"])
                            .pick_file();
                    }
                    if ui.button(format!("{} New library…", icons::PLUS)).clicked() {
                        chosen = rfd::FileDialog::new()
                            .set_file_name("library.litanatomy")
                            .add_filter("LitAnatomy library", &["litanatomy"])
                            .save_file();
                    }
                    if let Some(path) = chosen {
                        match Library::open(&path) {
                            Ok(lib) => {
                                settings.library_path = Some(path);
                                if let Some(p) = settings_path {
                                    let _ = settings.save(p);
                                }
                                next = Some(Screen::Ready(Box::new(Session::new(lib, settings.clone(), settings_path.clone()))));
                            }
                            Err(e) => *error = format!("{}: {e:#}", path.display()),
                        }
                    }
                });
            });
        });
    });
    next
}
