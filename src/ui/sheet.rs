//! Importing a review spreadsheet: choose where each column goes, then the
//! usual import preview (duplicates, keys, collection) takes over.

use std::path::{Path, PathBuf};

use egui::{RichText, Ui};

use crate::app::{Lookup, Session};
use crate::import::ident::Identifier;
use crate::import::sheet::{self, Column, Grid, Target};
use crate::model::{Field, FieldKind};
use crate::net::Job;
use crate::ui::dialogs::{Candidate, Dialog};
use crate::ui::sidebar::plural;
use crate::ui::{icons, theme};

pub struct SheetForm {
    pub path: PathBuf,
    pub name: String,
    pub grids: Vec<Grid>,
    pub sheet: usize,
    /// Row of the headings, from 0.
    pub header_row: usize,
    pub columns: Vec<Column>,
    /// The fields a column can go to: the library's and the target collection's.
    pub fields: Vec<Field>,
    pub target: Option<i64>,
    /// Look the DOIs up online for authors, year and journal.
    pub lookup: bool,
}

impl SheetForm {
    fn grid(&self) -> &Grid {
        &self.grids[self.sheet]
    }

    fn has(&self, t: &Target) -> bool {
        self.columns.iter().any(|c| &c.target == t)
    }
}

impl Session {
    pub fn pick_sheet_file(&mut self) {
        let target = self.current_collection();
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Import a review spreadsheet")
            .add_filter("Spreadsheets", sheet::EXTENSIONS)
            .pick_file()
        {
            self.import_sheet(&path, target);
        }
    }

    /// Reads a spreadsheet and asks where its columns go.
    pub fn import_sheet(&mut self, path: &Path, target: Option<i64>) {
        let grids = match sheet::read(path) {
            Ok(g) => g,
            Err(e) => return self.error(format!("{e:#}")),
        };
        // The first sheet with something under its headings.
        let sheet = grids.iter().position(|g| g.rows.len() > g.header_row() + 1).unwrap_or(0);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut form = SheetForm {
            path: path.to_path_buf(),
            name,
            header_row: grids[sheet].header_row(),
            grids,
            sheet,
            columns: Vec::new(),
            fields: Vec::new(),
            target,
            lookup: !self.net.offline,
        };
        self.sheet_fields(&mut form);
        self.sheet_columns(&mut form);
        self.dialog = Some(Dialog::ImportSheet(Box::new(form)));
    }

    fn sheet_fields(&self, form: &mut SheetForm) {
        form.fields = self.lib.fields(form.target).unwrap_or_default().into_iter().filter(|f| !f.hidden).collect();
    }

    /// Guesses (or recalls) every column's target again.
    fn sheet_columns(&self, form: &mut SheetForm) {
        form.columns = sheet::columns(form.grid(), form.header_row, &form.fields, &self.settings.sheet_columns);
    }

    pub(crate) fn sheet_dialog(&mut self, ui: &mut Ui, form: &mut SheetForm) -> bool {
        let mut open = true;
        let mut go = false;
        let mut regroup = false;
        let collections = self.cache.collections.clone();
        let rows = sheet::rows(form.grid(), form.header_row, &form.columns);
        let resp = egui::Modal::new(egui::Id::unique("import-sheet")).frame(theme::modal_frame(ui)).show(ui.ctx(), |ui| {
            let width = (ui.ctx().content_rect().width() - 120.0).clamp(560.0, 1000.0);
            ui.set_width(width);
            ui.label(theme::serif("Import a review spreadsheet", 24.0));
            ui.label(RichText::new(format!("{} · {}", form.name, plural(form.grid().rows.len().saturating_sub(form.header_row + 1), "row"))).color(theme::muted(ui)));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if form.grids.len() > 1 {
                    ui.label("Sheet");
                    let current = form.grid().name.clone();
                    egui::ComboBox::from_id_salt("sheet-pick").selected_text(current).show_ui(ui, |ui| {
                        for (i, g) in form.grids.iter().enumerate() {
                            if ui.selectable_label(form.sheet == i, &g.name).clicked() && form.sheet != i {
                                form.sheet = i;
                                regroup = true;
                            }
                        }
                    });
                    ui.add_space(12.0);
                }
                ui.label("Headings in row");
                let mut row = form.header_row + 1;
                let max = form.grid().rows.len().max(1);
                if ui.add(egui::DragValue::new(&mut row).range(1..=max)).changed() {
                    form.header_row = row - 1;
                    regroup = true;
                }
                ui.add_space(12.0);
                ui.label("Add to");
                let current = form.target.and_then(|t| collections.iter().find(|c| c.id == t)).map(|c| c.name.clone()).unwrap_or_else(|| "No collection".into());
                let before = form.target;
                egui::ComboBox::from_id_salt("sheet-target").selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut form.target, None, "No collection");
                    for c in &collections {
                        ui.selectable_value(&mut form.target, Some(c.id), &c.name);
                    }
                });
                if form.target != before {
                    self.sheet_fields(form);
                    // Columns going to the old collection's own fields are left out.
                    for c in &mut form.columns {
                        if let Target::Field(id) = c.target
                            && !form.fields.iter().any(|f| f.id == id)
                        {
                            c.target = Target::Ignore;
                        }
                    }
                }
            });
            ui.add_space(4.0);
            theme::hint(
                ui,
                "Choose where each column goes. Details such as the title and DOI describe the paper; the other columns become its notes. \
                 A column can also become a metric, or a new field named after its heading. Your choices are remembered for next time.",
            );
            ui.add_space(8.0);
            let max_h = (ui.ctx().content_rect().height() - 360.0).max(180.0);
            egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| {
                egui::Grid::new("sheet-columns").num_columns(4).striped(true).spacing([14.0, 8.0]).show(ui, |ui| {
                    for h in ["", "Heading", "First value", "Goes to"] {
                        ui.label(RichText::new(h).small().color(theme::muted(ui)));
                    }
                    ui.end_row();
                    let fields = form.fields.clone();
                    // Letter, heading, first value, menu (240 + its button), and the gaps.
                    let heading_w = (width * 0.24).clamp(150.0, 240.0);
                    let sample_w = (width - heading_w - 30.0 - 270.0 - 3.0 * 14.0 - 24.0).max(160.0);
                    for c in &mut form.columns {
                        ui.label(RichText::new(&c.letter).monospace().color(theme::muted(ui)));
                        ui.vertical(|ui| {
                            ui.set_width(heading_w);
                            if c.header.is_empty() {
                                ui.label(RichText::new("(no heading)").italics().color(theme::muted(ui)));
                            } else {
                                let header = sheet::clean_header(&c.header);
                                ui.add(egui::Label::new(theme::semibold(&header)).truncate()).on_hover_text(&header);
                            }
                        });
                        ui.vertical(|ui| {
                            ui.set_width(sample_w);
                            let first: String = c.sample.split_whitespace().collect::<Vec<_>>().join(" ");
                            let short: String = first.chars().take(120).collect();
                            let short = if short.len() < first.len() { format!("{short}…") } else { short };
                            let resp = ui.add(egui::Label::new(RichText::new(short).color(theme::muted(ui))).truncate());
                            if first.chars().count() > 120 {
                                resp.on_hover_text(&c.sample);
                            }
                            ui.label(RichText::new(format!("{} filled in", plural(c.filled, "row"))).small().color(theme::muted(ui)));
                        });
                        target_menu(ui, c, &fields);
                        ui.end_row();
                    }
                });
            });
            ui.add_space(10.0);
            let has_title = form.has(&Target::Title);
            let has_doi = form.has(&Target::Doi);
            if !has_title && !has_doi {
                ui.label(RichText::new(format!("{} Choose the column with each paper's title or DOI.", icons::WARNING)).color(theme::fg(ui, theme::AMBER)));
            } else {
                let mut summary = format!("{} to import", plural(rows.papers.len(), "paper"));
                if !rows.skipped.is_empty() {
                    summary.push_str(&format!(" · {} without a title or DOI left out", plural(rows.skipped.len(), "row")));
                }
                ui.label(summary);
            }
            if has_doi {
                ui.checkbox(&mut form.lookup, "Look each DOI up online, to fill in the authors, year and journal")
                    .on_hover_text("The spreadsheet's own details are kept where the lookup has none.");
            }
            if form.has(&Target::Field(form.fields.iter().find(|f| f.kind == FieldKind::Relevance).map(|f| f.id).unwrap_or(-1))) && form.target.is_none() {
                theme::hint(ui, "Relevance is noted per collection: choose one above, or it goes into Further Notes.");
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = (has_title || has_doi) && !rows.papers.is_empty();
                if ui.add_enabled(ready, theme::primary_button(ui, format!("{} Continue", icons::ARROW_RIGHT))).clicked() {
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
        if regroup {
            self.sheet_columns(form);
        }
        if go {
            self.start_sheet_import(ui.ctx(), form, rows);
            return false;
        }
        open
    }

    /// Remembers the mapping, then opens the import preview with the rows
    /// (looking their DOIs up first, if asked).
    fn start_sheet_import(&mut self, ctx: &egui::Context, form: &SheetForm, rows: sheet::Rows) {
        for c in form.columns.iter().filter(|c| !c.header.trim().is_empty()) {
            self.settings.sheet_columns.insert(sheet::normalise(&c.header), c.target.code(&form.fields));
        }
        self.save_settings(ctx);
        let source = format!("{} (spreadsheet)", form.name);
        let mut state = crate::ui::dialogs::ImportState::new(source, form.path.parent().map(Path::to_path_buf), form.target, self.settings.keep_readable_keys);
        state.errors = rows.skipped;
        state.candidates = rows.papers.into_iter().map(|(paper, notes)| Candidate { sheet: notes, ..Candidate::new(paper) }).collect();
        state.refresh(&self.lib, &self.settings.citekey_pattern);
        if form.lookup {
            let jobs: Vec<Job> = state.candidates.iter().filter_map(|c| c.paper.bib.doi.clone()).map(|d| Job::Id(Identifier::Doi(d))).collect();
            if !jobs.is_empty() {
                let count = jobs.len();
                let batch = self.net.run(ctx, jobs, self.settings.contact_email.clone());
                self.lookups.insert(batch, Lookup::Fill);
                state.pending = Some((batch, count));
            }
        }
        self.dialog = Some(Dialog::Import(state));
    }
}

/// The menu of where a column can go.
fn target_menu(ui: &mut Ui, c: &mut Column, fields: &[Field]) {
    let label = c.target.label(fields, &c.header);
    let shown = if c.target == Target::Ignore { RichText::new(label).color(theme::muted(ui)) } else { RichText::new(label) };
    egui::ComboBox::from_id_salt(("sheet-target", c.index)).width(240.0).selected_text(shown).show_ui(ui, |ui| {
        ui.selectable_value(&mut c.target, Target::Ignore, "Leave out");
        ui.separator();
        ui.label(RichText::new("The paper's details").small().color(theme::muted(ui)));
        for t in Target::DETAILS {
            let text = t.label(fields, &c.header);
            ui.selectable_value(&mut c.target, t, text);
        }
        ui.separator();
        ui.label(RichText::new("Its notes").small().color(theme::muted(ui)));
        for f in fields {
            let mut text = f.label.clone();
            if f.collection.is_some() {
                text.push_str("  (this collection)");
            }
            ui.selectable_value(&mut c.target, Target::Field(f.id), text);
        }
        if !c.header.trim().is_empty() {
            ui.separator();
            let metric = Target::Metric.label(fields, &c.header);
            ui.selectable_value(&mut c.target, Target::Metric, format!("{metric} in Key Metrics"));
            let new = Target::NewField.label(fields, &c.header);
            ui.selectable_value(&mut c.target, Target::NewField, new);
        }
    });
}
