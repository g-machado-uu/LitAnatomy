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
use crate::net::{Net, NetMsg};
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
    Settings,
}

/// What a batch of DOI lookups is for.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    /// Typed or pasted DOIs: add straight away (one) or preview (several).
    Quick { target: Option<i64>, count: usize },
    /// Results go to the import preview dialog.
    Preview,
    /// Refresh a paper's metadata from its DOI.
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
    pub settings_form: SettingsForm,
    pub archived_open: bool,
    /// Set by the UI when something needs the time (autosave), read in `logic`.
    pub now: f64,
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
            settings_form,
            archived_open: false,
            now: 0.0,
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
            View::List(Scope::Collection(c)) | View::Matrix(Scope::Collection(c)) => Some(*c),
            View::Paper(_) => self.page.as_ref().and_then(|p| p.context),
            _ => None,
        };
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
            View::List(Scope::Collection(c)) | View::Matrix(Scope::Collection(c)) => Some(*c),
            View::Paper(_) => self.page.as_ref().and_then(|p| p.context),
            _ => None,
        }
    }

    /// Looks up the DOIs in `text`. One DOI is added straight away; several
    /// open the import preview.
    pub fn lookup_dois(&mut self, ctx: &egui::Context, text: &str, target: Option<i64>) -> bool {
        let dois = import::doi::find_all(text);
        if dois.is_empty() {
            self.error("No DOI found. A DOI looks like 10.1109/TAP.2025.3603900.");
            return false;
        }
        let count = dois.len();
        let batch = self.net.fetch(ctx, dois, self.settings.contact_email.clone());
        if count == 1 {
            self.lookups.insert(batch, Lookup::Quick { target, count });
            self.info("Looking up the DOI…");
        } else {
            self.lookups.insert(batch, Lookup::Preview);
            let mut state = ImportState::new(format!("{count} DOIs"), None, target, self.settings.keep_readable_keys);
            state.pending = Some((batch, count));
            self.dialog = Some(Dialog::Import(state));
        }
        true
    }

    fn handle_net(&mut self) {
        for msg in self.net.poll() {
            match msg {
                NetMsg::Fetched { batch, doi, result } => match self.lookups.get(&batch).cloned() {
                    Some(Lookup::Quick { target, .. }) => match result {
                        Ok(paper) => self.add_single(paper, target),
                        Err(e) => self.error(e),
                    },
                    Some(Lookup::Preview) => {
                        if let Some(Dialog::Import(state)) = &mut self.dialog
                            && state.pending.is_some_and(|(b, _)| b == batch)
                        {
                            match result {
                                Ok(paper) => state.candidates.push(Candidate::new(paper)),
                                Err(e) => state.errors.push(format!("{doi}: {e}")),
                            }
                            if let Some((_, left)) = &mut state.pending {
                                *left = left.saturating_sub(1);
                            }
                            state.refresh(&self.lib, &self.settings.citekey_pattern);
                        }
                    }
                    Some(Lookup::Refresh(id)) => match result {
                        Ok(paper) => {
                            if let Some(page) = &mut self.page
                                && page.id == id
                            {
                                page.apply_lookup(paper.bib);
                                self.info("Metadata filled in from the DOI. Review it and press Save.");
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
                let mut state = ImportState::new(
                    format!("{name} ({})", format.label()),
                    path.parent().map(Path::to_path_buf),
                    target,
                    self.settings.keep_readable_keys,
                );
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
                View::Settings => self.settings_ui(ui),
            });
        self.dialogs(ui);
        self.toast_ui(ui);
    }

    pub fn logic(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.now = now;
        self.handle_net();
        self.flush_page(false);
        self.auto_export(now);
        if self.page.as_ref().is_some_and(|p| p.has_unsaved()) || self.auto.pending_since.is_some() {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.flush_page(true);
            if self.settings.auto_export && self.lib.revision() != self.auto.done_rev {
                let r = self.run_auto_export();
                self.check(r);
            }
        }
    }

    fn shortcuts(&mut self, ui: &mut Ui) {
        if self.dialog.is_some() {
            return;
        }
        let (esc, find) = ui.input(|i| {
            (i.key_pressed(egui::Key::Escape), i.modifiers.command && i.key_pressed(egui::Key::F))
        });
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if esc && !typing && matches!(self.view, View::Paper(_)) {
            self.go_back();
        }
        if find {
            if !matches!(self.view, View::List(_)) {
                self.go(View::List(Scope::All));
            }
            self.list.focus_search = true;
        }
    }

    /// PDFs dropped on a paper are linked; reference files are imported.
    fn handle_dropped_files(&mut self, ui: &mut Ui) {
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        for path in dropped {
            let is_pdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
            if is_pdf {
                match &self.view {
                    View::Paper(id) => {
                        let id = *id;
                        self.attach_pdf(id, &path);
                    }
                    _ => self.info("Open a paper first, then drop its PDF on it."),
                }
            } else {
                let target = self.current_collection();
                self.import_file(&path, target);
            }
        }
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
