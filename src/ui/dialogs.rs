//! Modal dialogs: adding by DOI, the import preview, collections, confirmations.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::{RichText, Ui};

use crate::app::{Session, View};
use crate::citekey;
use crate::db::{Library, Scope};
use crate::import::ident::Identifier;
use crate::import::sheet::SheetNote;
use crate::model::{Collection, Criterion, CriterionKind, Field, FieldKind, NewPaper, authors_short};
use crate::net::Found;
use crate::pdf;
use crate::ui::sidebar::plural;
use crate::ui::{icons, theme};

pub enum Dialog {
    AddDoi { text: String, target: Option<i64> },
    Import(ImportState),
    NewCollection { name: String, parent: Option<i64>, colour: u8 },
    EditCollection(Box<CollectionForm>),
    ConfirmDeleteCollection { id: i64, name: String },
    ConfirmEmptyTrash,
    ConfirmPurge(Vec<i64>),
    EditCell { paper: i64, field: i64, label: String, title: String, text: String },
    /// Ctrl+Shift+Delete on an open paper.
    ConfirmTrash(i64),
    /// The list of keyboard shortcuts (F1).
    Shortcuts,
    /// A review spreadsheet: which column goes where.
    ImportSheet(Box<crate::ui::sheet::SheetForm>),
    /// Saving papers' notes as a document or table.
    ExportNotes(Box<crate::ui::export::ExportForm>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Add,
    /// Duplicate: use the paper already in the library (added to the target collection).
    UseExisting,
    /// Duplicate: fill the existing paper's gaps from this record.
    Merge,
    Skip,
}

pub struct Candidate {
    pub paper: NewPaper,
    pub key: String,
    pub duplicate: Option<(i64, String)>,
    pub choice: Choice,
    /// How it was found (papers from PDFs), and whether to check it.
    pub note: Option<(String, bool)>,
    /// From a spreadsheet: its notes, written once the paper is imported.
    pub sheet: Vec<SheetNote>,
}

impl Candidate {
    pub fn new(paper: NewPaper) -> Self {
        Self { paper, key: String::new(), duplicate: None, choice: Choice::Add, note: None, sheet: Vec::new() }
    }

    pub fn found(f: Found) -> Self {
        Self { note: f.note.map(|n| (n, f.sure)), ..Self::new(f.paper) }
    }
}

pub struct ImportState {
    pub source: String,
    /// Folder of the imported file, for relative PDF paths.
    pub base_dir: Option<PathBuf>,
    pub candidates: Vec<Candidate>,
    pub errors: Vec<String>,
    pub target: Option<i64>,
    /// DOI lookups still running: (batch, how many left).
    pub pending: Option<(u64, usize)>,
    pub keep_keys: bool,
    /// A line under the summary, e.g. what was left out.
    pub note: Option<String>,
    /// The pending jobs are PDFs being read, not identifiers.
    pub reading_pdfs: bool,
    checked: usize,
}

impl ImportState {
    pub fn new(source: String, base_dir: Option<PathBuf>, target: Option<i64>, keep_keys: bool) -> Self {
        Self {
            source,
            base_dir,
            candidates: Vec::new(),
            errors: Vec::new(),
            target,
            pending: None,
            keep_keys,
            note: None,
            reading_pdfs: false,
            checked: 0,
        }
    }

    /// Works out keys (unique against the library and each other) and
    /// looks for duplicates of candidates not checked yet.
    pub fn refresh(&mut self, lib: &Library, pattern: &str) {
        let mut taken: HashSet<String> = lib.citekeys().unwrap_or_default();
        for c in &mut self.candidates {
            let source = c.paper.source_key.clone().unwrap_or_default();
            let base = if self.keep_keys && citekey::is_valid_key(&source) && !citekey::looks_opaque(&source) {
                source
            } else {
                citekey::generate(pattern, &c.paper.bib)
            };
            c.key = citekey::make_unique(&base, |k| taken.contains(&k.to_lowercase()));
            taken.insert(c.key.to_lowercase());
        }
        for i in self.checked..self.candidates.len() {
            let dup = lib.find_duplicate(&self.candidates[i].paper.bib).ok().flatten();
            // A duplicate within the same file counts too.
            let fp = crate::db::title_fingerprint(&self.candidates[i].paper.bib.title);
            let earlier = self.candidates[..i].iter().any(|o| {
                let same_doi = o.paper.bib.doi.is_some() && o.paper.bib.doi == self.candidates[i].paper.bib.doi;
                same_doi || (fp.len() >= 8 && crate::db::title_fingerprint(&o.paper.bib.title) == fp)
            });
            let c = &mut self.candidates[i];
            if let Some(id) = dup {
                let title = lib.paper(id).map(|p| p.citekey).unwrap_or_default();
                c.duplicate = Some((id, title));
                c.choice = Choice::UseExisting;
            } else if earlier {
                c.choice = Choice::Skip;
            }
        }
        self.checked = self.candidates.len();
    }

    /// Looks for duplicates again (after lookups changed the details),
    /// keeping what the user chose for papers that aren't duplicates.
    pub fn recheck(&mut self, lib: &Library, pattern: &str) {
        for c in &mut self.candidates {
            if c.duplicate.take().is_some() || c.choice == Choice::Skip {
                c.choice = Choice::Add;
            }
        }
        self.checked = 0;
        self.refresh(lib, pattern);
    }
}

/// The collection settings dialog.
pub struct CollectionForm {
    pub collection: Collection,
    pub fields: Vec<Field>,
    pub new_field: String,
    pub new_kind: FieldKind,
    pub criteria: Vec<Criterion>,
    pub new_include: String,
    pub new_exclude: String,
}

impl CollectionForm {
    pub fn load(lib: &Library, id: i64) -> Option<Self> {
        let collection = lib.collection(id).ok().flatten()?;
        let fields = lib.fields(Some(id)).ok()?.into_iter().filter(|f| f.collection == Some(id)).collect();
        Some(Self {
            collection,
            fields,
            new_field: String::new(),
            new_kind: FieldKind::ShortText,
            criteria: lib.criteria(id).ok()?,
            new_include: String::new(),
            new_exclude: String::new(),
        })
    }
}

impl Session {
    pub(crate) fn dialogs(&mut self, ui: &mut Ui) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let keep = match &mut dialog {
            Dialog::AddDoi { text, target } => self.add_doi_dialog(ui, text, *target),
            Dialog::Import(state) => self.import_dialog(ui, state),
            Dialog::NewCollection { name, parent, colour } => self.new_collection_dialog(ui, name, *parent, colour),
            Dialog::EditCollection(form) => self.edit_collection_dialog(ui, form),
            Dialog::ConfirmDeleteCollection { id, name } => {
                let (id, name) = (*id, name.clone());
                self.confirm(ui, "Delete collection", &format!("Delete \"{name}\" and its sub-collections? The papers stay in your library, with their notes."), "Delete collection", |s| {
                    let r = s.lib.delete_collection(id);
                    s.check(r);
                    let showing = match &s.view {
                        View::List(Scope::Collection(c)) | View::Matrix(Scope::Collection(c)) | View::Synthesis(c) => *c == id,
                        _ => false,
                    };
                    if showing {
                        s.synthesis = None;
                        s.go(View::Home);
                    }
                })
            }
            Dialog::ConfirmEmptyTrash => {
                let n = self.cache.trash;
                self.confirm(ui, "Empty the Trash", &format!("Delete {} for good, with their notes? This can't be undone.", plural(n, "paper")), "Empty Trash", |s| {
                    let r = s.lib.empty_trash();
                    if let Some(n) = s.check(r) {
                        s.info(format!("Deleted {}.", plural(n, "paper")));
                    }
                    s.list.selected.clear();
                })
            }
            Dialog::ConfirmPurge(ids) => {
                let ids = ids.clone();
                self.confirm(ui, "Delete for good", &format!("Delete {} for good, with their notes? This can't be undone.", plural(ids.len(), "paper")), "Delete", |s| {
                    let r = s.lib.purge(&ids);
                    s.check(r);
                    s.list.selected.clear();
                })
            }
            Dialog::EditCell { paper, field, label, title, text } => self.edit_cell_dialog(ui, *paper, *field, label, title, text),
            Dialog::ConfirmTrash(id) => {
                let id = *id;
                let title = self.page.as_ref().filter(|p| p.id == id).map(|p| p.paper.bib.title.clone()).unwrap_or_default();
                self.confirm(ui, "Move to the Trash", &format!("Move \"{title}\" to the Trash? It can be restored from there."), "Move to Trash", |s| {
                    s.trash_open_paper(id);
                })
            }
            Dialog::Shortcuts => self.shortcuts_dialog(ui),
            Dialog::ImportSheet(form) => self.sheet_dialog(ui, form),
            Dialog::ExportNotes(form) => self.export_notes_dialog(ui, form),
        };
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    /// A yes/no dialog. Returns whether it stays open.
    fn confirm(&mut self, ui: &mut Ui, title: &str, text: &str, action: &str, then: impl FnOnce(&mut Session)) -> bool {
        let mut result = None;
        let resp = egui::Modal::new(egui::Id::unique(("confirm", title))).frame(theme::modal_frame(ui)).show(ui.ctx(), |ui| {
            ui.set_width(440.0);
            ui.label(theme::serif(title, 22.0));
            ui.add_space(6.0);
            ui.add(egui::Label::new(text).wrap());
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.add(theme::danger_button(action)).clicked() {
                    result = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    result = Some(false);
                }
            });
        });
        if resp.should_close() && result.is_none() {
            result = Some(false);
        }
        match result {
            Some(true) => {
                then(self);
                false
            }
            Some(false) => false,
            None => true,
        }
    }

    fn add_doi_dialog(&mut self, ui: &mut Ui, text: &mut String, target: Option<i64>) -> bool {
        let mut open = true;
        let mut go = false;
        let resp = egui::Modal::new(egui::Id::unique("add-doi")).frame(theme::modal_frame(ui)).show(ui.ctx(), |ui| {
            ui.set_width(540.0);
            ui.label(theme::serif("Add by identifier", 24.0));
            ui.add_space(4.0);
            theme::hint(
                ui,
                "Paste DOIs, arXiv IDs, ISBNs or PubMed IDs, one per line or mixed with other text: \
                 \"10.1109/…\", \"https://doi.org/…\", \"arXiv:2301.12345\", \"ISBN 978-…\", \"PMID: 31452104\".",
            );
            ui.add_space(6.0);
            let edit = ui.add(
                egui::TextEdit::multiline(text)
                    .hint_text("10.1109/TAP.2025.3603900\narXiv:1706.03762")
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            edit.request_focus();
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
                go = true;
            }
            let ids = crate::import::ident::find_all(text);
            if !ids.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for id in ids.iter().take(12) {
                        let colour = match id {
                            Identifier::Doi(_) => theme::accent(ui),
                            Identifier::Arxiv(_) => theme::RED,
                            Identifier::Isbn(_) => theme::AMBER,
                            Identifier::Pmid(_) => theme::GREEN,
                        };
                        theme::chip(ui, id.display(), colour);
                    }
                    if ids.len() > 12 {
                        ui.label(RichText::new(format!("and {} more", ids.len() - 12)).small().color(theme::muted(ui)));
                    }
                });
            }
            if let Some(c) = target.and_then(|c| self.collection(c)) {
                theme::hint(ui, format!("New papers go into the collection \"{}\".", c.name));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let label = match ids.len() {
                    0 => "Look up".to_string(),
                    1 => format!("Look up the {}", ids[0].kind()),
                    n => format!("Look up {n} papers"),
                };
                if ui.add_enabled(!ids.is_empty(), theme::primary_button(ui, format!("{} {label}", icons::MAGIC_WAND))).clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    open = false;
                }
            });
        });
        if resp.should_close() {
            open = false;
        }
        if go && !text.trim().is_empty() {
            let text = text.clone();
            return !self.lookup_ids(ui.ctx(), &text, target);
        }
        open
    }

    fn import_dialog(&mut self, ui: &mut Ui, state: &mut ImportState) -> bool {
        let mut open = true;
        let mut commit = false;
        let mut keys_changed = false;
        let collections = self.cache.collections.clone();
        let resp = egui::Modal::new(egui::Id::unique("import")).frame(theme::modal_frame(ui)).show(ui.ctx(), |ui| {
            let width = (ui.ctx().content_rect().width() - 120.0).clamp(520.0, 980.0);
            ui.set_width(width);
            ui.label(theme::serif("Import references", 24.0));
            let new = state.candidates.iter().filter(|c| c.duplicate.is_none() && c.choice == Choice::Add).count();
            let dups = state.candidates.iter().filter(|c| c.duplicate.is_some()).count();
            let mut summary = format!("{} · {} found", state.source, plural(state.candidates.len(), "reference"));
            if dups > 0 {
                summary.push_str(&format!(" · {dups} already in your library"));
            }
            ui.label(RichText::new(summary).color(theme::muted(ui)));
            if let Some(note) = &state.note {
                ui.label(RichText::new(note).small().color(theme::muted(ui)));
            }
            if let Some((_, left)) = state.pending {
                ui.horizontal(|ui| {
                    ui.spinner();
                    if state.reading_pdfs {
                        ui.label(format!("Reading {} and looking them up…", plural(left, "more PDF")));
                    } else {
                        ui.label(format!("Looking up {} more…", plural(left, "identifier")));
                    }
                });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Add to");
                let current = state.target.and_then(|t| collections.iter().find(|c| c.id == t)).map(|c| c.name.clone()).unwrap_or_else(|| "No collection".into());
                egui::ComboBox::from_id_salt("import-target").selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut state.target, None, "No collection");
                    for c in &collections {
                        ui.selectable_value(&mut state.target, Some(c.id), &c.name);
                    }
                });
                if state.target.is_none() {
                    ui.label(RichText::new("You can drag them onto a collection later.").small().color(theme::muted(ui)));
                }
                ui.add_space(12.0);
                // Only files carry keys; PDFs and lookups don't.
                let has_keys = state.candidates.iter().any(|c| c.paper.source_key.is_some());
                if has_keys
                    && ui
                        .checkbox(&mut state.keep_keys, "Keep readable keys from the file")
                        .on_hover_text("Numeric or database keys (like IEEE's 10234847) are always replaced.")
                        .changed()
                {
                    keys_changed = true;
                }
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height((ui.ctx().content_rect().height() - 330.0).max(200.0)).show(ui, |ui| {
                egui::Grid::new("import-grid").num_columns(3).striped(true).spacing([12.0, 8.0]).show(ui, |ui| {
                    for c in &mut state.candidates {
                        ui.vertical(|ui| {
                            ui.set_width(width - 470.0);
                            ui.add(egui::Label::new(theme::semibold(&c.paper.bib.title)).wrap());
                            let meta = [
                                authors_short(&c.paper.bib.authors),
                                c.paper.bib.year.map(|y| y.to_string()).unwrap_or_default(),
                                c.paper.bib.container.clone(),
                            ]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(" · ");
                            ui.add(egui::Label::new(RichText::new(meta).small().color(theme::muted(ui))).wrap());
                            if let Some(pdf) = &c.paper.pdf_path {
                                let name = Path::new(pdf).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                ui.label(RichText::new(format!("{} {name}", icons::FILE_PDF)).small().color(theme::muted(ui)))
                                    .on_hover_text(pdf.as_str());
                            }
                            if !c.sheet.is_empty() {
                                let text = format!("{} {} from the spreadsheet", icons::NOTE_PENCIL, plural(c.sheet.len(), "note"));
                                ui.label(RichText::new(text).small().color(theme::muted(ui)));
                            }
                            if let Some((note, sure)) = &c.note {
                                let (icon, colour) = if *sure { (icons::CHECK, theme::muted(ui)) } else { (icons::WARNING, theme::fg(ui, theme::AMBER)) };
                                ui.add(egui::Label::new(RichText::new(format!("{icon} {note}")).small().color(colour)).wrap());
                            }
                        });
                        ui.vertical(|ui| {
                            ui.set_width(210.0);
                            let old = c.paper.source_key.as_deref().filter(|k| *k != c.key);
                            ui.label(RichText::new(&c.key).monospace());
                            if let Some(old) = old {
                                ui.label(RichText::new(format!("was {old}")).small().color(theme::muted(ui)));
                            }
                        });
                        ui.vertical(|ui| {
                            match &c.duplicate {
                                Some((_, key)) => {
                                    theme::chip(ui, format!("Already in library as {key}"), theme::AMBER);
                                    egui::ComboBox::from_id_salt(("dup", c.key.as_str())).selected_text(choice_label(c.choice)).show_ui(ui, |ui| {
                                        for ch in [Choice::UseExisting, Choice::Merge, Choice::Add, Choice::Skip] {
                                            ui.selectable_value(&mut c.choice, ch, choice_label(ch));
                                        }
                                    });
                                }
                                None => {
                                    let mut on = c.choice == Choice::Add;
                                    if ui.checkbox(&mut on, "Import").changed() {
                                        c.choice = if on { Choice::Add } else { Choice::Skip };
                                    }
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
                if !state.errors.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(format!("{} Skipped {}:", icons::WARNING, plural(state.errors.len(), "entry"))).color(theme::fg(ui, theme::AMBER)));
                    for e in &state.errors {
                        ui.label(RichText::new(e).small().color(theme::muted(ui)));
                    }
                }
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let busy = state.pending.is_some();
                let label = format!("{} Import {}", icons::DOWNLOAD_SIMPLE, plural(new, "new paper"));
                if ui.add_enabled(!busy && !state.candidates.is_empty(), theme::primary_button(ui, label)).clicked() {
                    commit = true;
                }
                if ui.button("Cancel").clicked() {
                    open = false;
                }
            });
        });
        if resp.should_close() && state.pending.is_none() {
            open = false;
        }
        if keys_changed {
            state.refresh(&self.lib, &self.settings.citekey_pattern);
        }
        if commit {
            self.commit_import(state);
            return false;
        }
        open
    }

    fn commit_import(&mut self, state: &mut ImportState) {
        let mut added = 0;
        let mut touched = Vec::new();
        // Spreadsheet notes, written once the papers are in their collection: (paper, row, already in the library).
        let mut sheet_notes: Vec<(i64, usize, bool)> = Vec::new();
        let mut first_error = None;
        for (ci, c) in state.candidates.iter().enumerate() {
            let id = match (c.choice, &c.duplicate) {
                (Choice::Skip, _) => continue,
                (Choice::UseExisting, Some((id, _))) => Some(*id),
                (Choice::Merge, Some((id, _))) => match self.lib.merge_into(*id, &c.paper.bib) {
                    Ok(_) => Some(*id),
                    Err(e) => {
                        first_error.get_or_insert(e);
                        None
                    }
                },
                _ => {
                    let mut paper = c.paper.clone();
                    paper.pdf_path = paper.pdf_path.as_deref().map(|p| self.import_pdf_path(p, state.base_dir.as_deref()));
                    let key = self.lib.unique_citekey(&c.key, None);
                    match self.lib.insert_paper(&paper, &key) {
                        Ok(id) => {
                            added += 1;
                            if !c.sheet.is_empty() {
                                sheet_notes.push((id, ci, false));
                            }
                            Some(id)
                        }
                        Err(e) => {
                            first_error.get_or_insert(e);
                            None
                        }
                    }
                }
            };
            if let Some(id) = id {
                if c.duplicate.is_some() && matches!(c.choice, Choice::UseExisting | Choice::Merge) && !c.sheet.is_empty() {
                    sheet_notes.push((id, ci, true));
                }
                touched.push(id);
                // A paper already in the library gets the PDF if it has none.
                if c.duplicate.is_some()
                    && matches!(c.choice, Choice::UseExisting | Choice::Merge)
                    && let Some(raw) = c.paper.pdf_path.as_deref()
                    && self.lib.paper(id).is_ok_and(|p| p.pdf_path.is_none())
                {
                    let stored = self.import_pdf_path(raw, state.base_dir.as_deref());
                    if let Err(e) = self.lib.set_pdf(id, Some(&stored)) {
                        first_error.get_or_insert(e);
                    }
                }
            }
        }
        if let Some(target) = state.target {
            let r = self.lib.add_to_collection(target, &touched);
            self.check(r);
        }
        let r = self.lib.restore(&touched);
        self.check(r);
        let mut created = std::collections::HashMap::new();
        for (id, ci, existing) in sheet_notes {
            let notes = &state.candidates[ci].sheet;
            if let Err(e) = crate::import::sheet::apply(&mut self.lib, id, notes, state.target, &mut created, existing) {
                first_error.get_or_insert(e);
            }
        }
        match first_error {
            Some(e) => self.error(format!("Some papers could not be imported: {e:#}")),
            None => {
                let existing = touched.len() - added;
                let mut msg = format!("Imported {}.", plural(added, "new paper"));
                if existing > 0 {
                    msg.push_str(&format!(" {} were already in your library.", existing));
                }
                if !created.is_empty() {
                    let mut names: Vec<String> = created.keys().map(|n| format!("\"{n}\"")).collect();
                    names.sort();
                    msg.push_str(&format!(" New fields: {}. Rename or hide them in Settings.", names.join(", ")));
                }
                self.info(msg);
            }
        }
        if let Some(target) = state.target {
            self.go(View::List(Scope::Collection(target)));
        } else if !matches!(self.view, View::List(_)) {
            self.go(View::List(Scope::All));
        }
    }

    /// PDF paths in imported files may be relative to the file.
    fn import_pdf_path(&self, raw: &str, base: Option<&Path>) -> String {
        let p = PathBuf::from(raw);
        let full = if p.is_absolute() { p } else { base.map(|b| b.join(&p)).unwrap_or(p) };
        pdf::to_stored(&full, self.settings.pdf_folder.as_deref())
    }

    fn new_collection_dialog(&mut self, ui: &mut Ui, name: &mut String, parent: Option<i64>, colour: &mut u8) -> bool {
        let mut open = true;
        let mut create = false;
        let resp = egui::Modal::new(egui::Id::unique("new-collection")).frame(theme::modal_frame(ui)).show(ui.ctx(), |ui| {
            ui.set_width(420.0);
            ui.label(theme::serif(if parent.is_some() { "New sub-collection" } else { "New collection" }, 24.0));
            if let Some(p) = parent.and_then(|p| self.collection(p)) {
                ui.label(RichText::new(format!("Inside \"{}\"", p.name)).color(theme::muted(ui)));
            }
            ui.add_space(8.0);
            let edit = ui.add(egui::TextEdit::singleline(name).hint_text("e.g. First-year assessment, Journal paper, Viva").desired_width(f32::INFINITY));
            edit.request_focus();
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                create = true;
            }
            ui.add_space(6.0);
            colour_picker(ui, colour);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button(ui, format!("{} Create", icons::FOLDER_SIMPLE_PLUS))).clicked() {
                    create = true;
                }
                if ui.button("Cancel").clicked() {
                    open = false;
                }
            });
        });
        if resp.should_close() {
            open = false;
        }
        if create {
            if name.trim().is_empty() {
                self.error("Give the collection a name.");
                return true;
            }
            let r = self.lib.create_collection(name, parent, *colour);
            if let Some(id) = self.check(r) {
                self.go(View::List(Scope::Collection(id)));
            }
            return false;
        }
        open
    }

    fn edit_collection_dialog(&mut self, ui: &mut Ui, form: &mut CollectionForm) -> bool {
        let mut open = true;
        let mut save = false;
        let mut delete = false;
        let id = form.collection.id;
        let parents: Vec<Collection> = {
            let below = self.lib.descendants(id).unwrap_or_default();
            self.cache.collections.iter().filter(|c| c.id != id && !below.contains(&c.id)).cloned().collect()
        };
        let resp = egui::Modal::new(egui::Id::unique("edit-collection")).frame(theme::modal_frame(ui)).show(ui.ctx(), |ui| {
            ui.set_width(560.0);
            ui.label(theme::serif("Collection settings", 24.0));
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height((ui.ctx().content_rect().height() - 220.0).max(240.0)).show(ui, |ui| {
                let c = &mut form.collection;
                egui::Grid::new("collection-grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(380.0));
                    ui.end_row();
                    ui.label("Description");
                    ui.add(egui::TextEdit::multiline(&mut c.description).desired_rows(2).desired_width(380.0).hint_text("What is this collection for?"));
                    ui.end_row();
                    ui.label("Colour");
                    colour_picker(ui, &mut c.colour);
                    ui.end_row();
                    ui.label("Inside");
                    let current = c.parent.and_then(|p| parents.iter().find(|x| x.id == p)).map(|x| x.name.clone()).unwrap_or_else(|| "— (top level)".into());
                    egui::ComboBox::from_id_salt("parent").selected_text(current).show_ui(ui, |ui| {
                        ui.selectable_value(&mut c.parent, None, "— (top level)");
                        for p in &parents {
                            ui.selectable_value(&mut c.parent, Some(p.id), &p.name);
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut c.archived, "Archived (hidden from the sidebar list)");
                    ui.end_row();
                });
                ui.add_space(10.0);
                theme::card_title(ui, "Auto-export .bib");
                let mut path = c.bib_export_path.clone().unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut path).hint_text("e.g. ~/thesis/references.bib").desired_width(330.0));
                    if ui.button(format!("{} Choose…", icons::FOLDER_OPEN)).clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .set_file_name(format!("{}.bib", crate::app::safe_file_name(&c.name)))
                            .add_filter("BibTeX", &["bib"])
                            .save_file()
                    {
                        path = p.display().to_string();
                    }
                    if !path.is_empty() && ui.add(theme::ghost_button(icons::X)).on_hover_text("Stop exporting").clicked() {
                        path.clear();
                    }
                });
                c.bib_export_path = (!path.trim().is_empty()).then(|| path.trim().to_string());
                let note = if self.settings.auto_export {
                    "This file is rewritten whenever the collection (or a sub-collection) changes, e.g. in your LaTeX or Overleaf-synced folder."
                } else {
                    "Auto-export is off. Turn it on in Settings to keep this file up to date."
                };
                theme::hint(ui, note);
                ui.add_space(10.0);
                theme::card_title(ui, "Extra fields for this collection");
                theme::hint(ui, "Fields only papers in this collection need, such as \"Frequency range\". Short text and metrics also make good matrix columns.");
                let mut remove = None;
                let mut moves = None;
                for (i, f) in form.fields.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut f.label).desired_width(200.0));
                        ui.label(RichText::new(f.kind.label()).small().color(theme::muted(ui)));
                        ui.add(egui::TextEdit::singleline(&mut f.prompt).hint_text("prompt shown in the field").desired_width(150.0));
                        if ui.add(theme::ghost_button(icons::CARET_UP)).clicked() {
                            moves = Some((f.id, true));
                        }
                        if ui.add(theme::ghost_button(icons::CARET_DOWN)).clicked() {
                            moves = Some((f.id, false));
                        }
                        if ui.add(theme::ghost_button(icons::TRASH)).on_hover_text("Delete the field and its notes").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    let f = form.fields.remove(i);
                    let r = self.lib.delete_field(f.id);
                    self.check(r);
                }
                if let Some((fid, up)) = moves {
                    for f in &form.fields {
                        let r = self.lib.update_field(f);
                        self.check(r);
                    }
                    let r = self.lib.move_field(fid, up);
                    self.check(r);
                    form.fields = self.lib.fields(Some(id)).unwrap_or_default().into_iter().filter(|f| f.collection == Some(id)).collect();
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.new_field).hint_text("New field, e.g. Frequency range").desired_width(200.0));
                    egui::ComboBox::from_id_salt("new-kind").selected_text(form.new_kind.label()).show_ui(ui, |ui| {
                        for k in FieldKind::USER_KINDS {
                            ui.selectable_value(&mut form.new_kind, k, k.label());
                        }
                    });
                    if ui.add_enabled(!form.new_field.trim().is_empty(), egui::Button::new(format!("{} Add", icons::PLUS))).clicked() {
                        let r = self.lib.add_field(&form.new_field, "", form.new_kind, Some(id));
                        if self.check(r).is_some() {
                            form.new_field.clear();
                            form.fields = self.lib.fields(Some(id)).unwrap_or_default().into_iter().filter(|f| f.collection == Some(id)).collect();
                        }
                    }
                });
                ui.add_space(10.0);
                self.criteria_editor(ui, form);
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button(ui, format!("{} Save", icons::CHECK))).clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    open = false;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(theme::ghost_button(RichText::new(format!("{} Delete collection…", icons::TRASH)).color(theme::fg(ui, theme::RED)).text())).clicked() {
                        delete = true;
                    }
                });
            });
        });
        if resp.should_close() {
            open = false;
        }
        if save {
            if form.collection.name.trim().is_empty() {
                self.error("The collection needs a name.");
                return true;
            }
            let r = self.lib.update_collection(&form.collection);
            self.check(r);
            for f in &form.fields {
                let r = self.lib.update_field(f);
                self.check(r);
            }
            for c in form.criteria.iter().filter(|c| !c.label.trim().is_empty()) {
                let r = self.lib.update_criterion(c.id, &c.label);
                self.check(r);
            }
            // Changing the export file should export straight away.
            self.auto.done_rev = 0;
            return false;
        }
        if delete {
            self.dialog = Some(Dialog::ConfirmDeleteCollection { id, name: form.collection.name.clone() });
            return false;
        }
        open
    }

    /// Turns screening on or off, and edits the inclusion and exclusion criteria.
    fn criteria_editor(&mut self, ui: &mut Ui, form: &mut CollectionForm) {
        let id = form.collection.id;
        theme::card_title(ui, "Systematic review");
        ui.checkbox(&mut form.collection.screening, "Screen these papers: include or exclude each one against criteria");
        if !form.collection.screening {
            theme::hint(ui, "For a systematic review. Each paper gets a Screening tab, and the list counts what was included and excluded, and why.");
            return;
        }
        theme::hint(ui, "Excluded papers stay in the collection, so the counts for a PRISMA flow diagram add up.");
        let mut remove = None;
        let mut reload = false;
        for kind in [CriterionKind::Include, CriterionKind::Exclude] {
            ui.add_space(6.0);
            let title = if kind == CriterionKind::Include { "Inclusion criteria" } else { "Exclusion criteria" };
            ui.label(theme::semibold(title));
            let mut n = 0;
            for c in form.criteria.iter_mut().filter(|c| c.kind == kind) {
                n += 1;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}{n}", kind.prefix())).monospace().color(theme::muted(ui)));
                    ui.add(egui::TextEdit::singleline(&mut c.label).desired_width(400.0));
                    if ui.add(theme::ghost_button(icons::TRASH)).on_hover_text("Delete the criterion (papers that cite it lose the tick)").clicked() {
                        remove = Some(c.id);
                    }
                });
            }
            let (input, hint) = match kind {
                CriterionKind::Include => (&mut form.new_include, "e.g. Peer-reviewed, published 2015 or later"),
                CriterionKind::Exclude => (&mut form.new_exclude, "e.g. Not in English, or no measured results"),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}{}", kind.prefix(), n + 1)).monospace().color(theme::muted(ui)));
                let edit = ui.add(egui::TextEdit::singleline(input).hint_text(hint).desired_width(400.0));
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.add_enabled(!input.trim().is_empty(), egui::Button::new(format!("{} Add", icons::PLUS))).clicked() || enter)
                    && !input.trim().is_empty()
                {
                    let r = self.lib.add_criterion(id, kind, input);
                    if self.check(r).is_some() {
                        input.clear();
                        reload = true;
                    }
                }
            });
        }
        if let Some(cid) = remove {
            let r = self.lib.delete_criterion(cid);
            self.check(r);
            reload = true;
        }
        if reload {
            // Keep labels being edited, pick up added or deleted criteria.
            let edited: Vec<(i64, String)> = form.criteria.iter().map(|c| (c.id, c.label.clone())).collect();
            form.criteria = self.lib.criteria(id).unwrap_or_default();
            for c in &mut form.criteria {
                if let Some((_, label)) = edited.iter().find(|(e, _)| *e == c.id) {
                    c.label = label.clone();
                }
            }
        }
    }

    fn edit_cell_dialog(&mut self, ui: &mut Ui, paper: i64, field: i64, label: &str, title: &str, text: &mut String) -> bool {
        let mut open = true;
        let mut save = false;
        let resp = egui::Modal::new(egui::Id::unique("edit-cell")).frame(theme::modal_frame(ui)).show(ui.ctx(), |ui| {
            ui.set_width(620.0);
            ui.label(theme::serif(label, 22.0));
            ui.label(RichText::new(title).color(theme::muted(ui)));
            ui.add_space(8.0);
            ui.add(egui::TextEdit::multiline(text).desired_rows(10).desired_width(f32::INFINITY));
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
                save = true;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button(ui, format!("{} Save", icons::CHECK))).clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    open = false;
                }
            });
        });
        if resp.should_close() {
            open = false;
        }
        if save {
            let r = self.lib.set_note(paper, field, text);
            self.check(r);
            return false;
        }
        open
    }
}

impl Session {
    /// The keyboard shortcuts, from the keyboard button in the sidebar or F1.
    fn shortcuts_dialog(&mut self, ui: &mut Ui) -> bool {
        let ctx = ui.ctx().clone();
        let key = |modifiers, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(modifiers, key));
        let cmd = egui::Modifiers::COMMAND;
        let groups = [
            (
                "Anywhere",
                vec![
                    ("F1".to_string(), "Show this list"),
                    (key(cmd, egui::Key::F), "Search all papers"),
                    (key(cmd, egui::Key::V), "Add copied PDFs or files (with no field selected)"),
                    ("Esc".to_string(), "Go back: paper › its list › Home"),
                ],
            ),
            (
                "Writing notes",
                vec![
                    ("Tab".to_string(), "Next field"),
                    (key(egui::Modifiers::SHIFT, egui::Key::Tab), "Previous field"),
                    (key(cmd, egui::Key::Enter), "Stop editing (notes show formatted)"),
                    ("Esc".to_string(), "Leave the field"),
                ],
            ),
            (
                "A paper",
                vec![
                    (key(cmd | egui::Modifiers::SHIFT, egui::Key::Delete), "Move it to the Trash (asks first)"),
                    ("Esc".to_string(), "Back to the list it was opened from"),
                ],
            ),
            (
                "Dialogs",
                vec![
                    (key(cmd, egui::Key::Enter), "Look up, or save the cell"),
                    ("Esc".to_string(), "Close"),
                ],
            ),
        ];
        let mut close = false;
        let resp = egui::Modal::new(egui::Id::unique("shortcuts")).frame(theme::modal_frame(ui)).show(&ctx, |ui| {
            ui.set_width(460.0);
            ui.horizontal(|ui| {
                theme::icon_badge(ui, icons::KEYBOARD, theme::accent(ui), 40.0);
                ui.label(theme::serif("Keyboard shortcuts", 22.0));
            });
            for (title, rows) in groups {
                ui.add_space(10.0);
                theme::card_title(ui, title);
                egui::Grid::new(("shortcuts", title)).num_columns(2).spacing([16.0, 6.0]).min_col_width(150.0).show(ui, |ui| {
                    for (keys, what) in rows {
                        ui.label(RichText::new(keys).monospace());
                        ui.label(what);
                        ui.end_row();
                    }
                });
            }
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(theme::primary_button(ui, "Close")).clicked() {
                    close = true;
                }
            });
        });
        !(close || resp.should_close())
    }
}

fn choice_label(c: Choice) -> &'static str {
    match c {
        Choice::UseExisting => "Use the existing paper",
        Choice::Merge => "Fill in its missing details",
        Choice::Add => "Import as a new paper",
        Choice::Skip => "Skip",
    }
}

fn colour_picker(ui: &mut Ui, colour: &mut u8) {
    ui.horizontal(|ui| {
        for (i, (name, c)) in theme::LABEL_COLOURS.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
            let selected = *colour as usize == i;
            ui.painter().circle_filled(rect.center(), 9.0, *c);
            if selected {
                ui.painter().circle_stroke(rect.center(), 11.0, egui::Stroke::new(2.0, theme::t(ui).ink));
            }
            if resp.on_hover_text(*name).clicked() {
                *colour = i as u8;
            }
        }
    });
}
