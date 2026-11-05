//! The synthesis matrix: papers as rows, chosen fields and metrics as columns.

use std::collections::HashMap;

use egui::{RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use crate::app::{Session, View, safe_file_name};
use crate::db::Scope;
use crate::export::{self, matrix};
use crate::model::{Criterion, Decision, Field, FieldKind, PaperRow, Screening, criterion_code, parse_metrics, parse_quotes};
use crate::ui::dialogs::Dialog;
use crate::ui::{icons, theme};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Col {
    Field(i64),
    /// One named metric across papers ("Gain").
    Metric(String),
    /// The collection's relevance notes.
    Relevance,
    /// The screening decision, criteria and note (review collections).
    Screening,
}

/// What the matrix shows, rebuilt when the library changes.
#[derive(Default, Clone)]
struct Data {
    rows: Vec<PaperRow>,
    fields: Vec<Field>,
    notes: HashMap<(i64, i64), String>,
    relevance: HashMap<i64, String>,
    metric_names: Vec<String>,
    /// Some for collections that screen.
    screening: Option<(HashMap<i64, Screening>, Vec<Criterion>)>,
}

#[derive(Default)]
pub struct MatrixState {
    chosen: HashMap<Scope, Vec<Col>>,
    data: Option<(u64, Scope, Data)>,
}

impl Data {
    fn col_label(&self, col: &Col) -> String {
        match col {
            Col::Field(id) => self.fields.iter().find(|f| f.id == *id).map(|f| f.label.clone()).unwrap_or_default(),
            Col::Metric(name) => name.clone(),
            Col::Relevance => "Relevance".into(),
            Col::Screening => "Screening".into(),
        }
    }

    fn cell(&self, paper: i64, col: &Col) -> String {
        match col {
            Col::Field(id) => {
                let raw = self.notes.get(&(paper, *id)).cloned().unwrap_or_default();
                match self.fields.iter().find(|f| f.id == *id).map(|f| f.kind) {
                    Some(FieldKind::Metrics) => parse_metrics(&raw)
                        .iter()
                        .map(|m| format!("{}: {}", m.name, m.value_with_unit()))
                        .collect::<Vec<_>>()
                        .join("; "),
                    Some(FieldKind::Quotes) => parse_quotes(&raw)
                        .iter()
                        .map(|q| if q.page.is_empty() { format!("“{}”", q.text) } else { format!("“{}” (p. {})", q.text, q.page) })
                        .collect::<Vec<_>>()
                        .join(" "),
                    _ => raw,
                }
            }
            Col::Metric(name) => self
                .fields
                .iter()
                .filter(|f| f.kind == FieldKind::Metrics)
                .flat_map(|f| parse_metrics(self.notes.get(&(paper, f.id)).map(String::as_str).unwrap_or("")))
                .find(|m| m.name.trim().eq_ignore_ascii_case(name))
                .map(|m| m.value_with_unit())
                .unwrap_or_default(),
            Col::Relevance => self.relevance.get(&paper).cloned().unwrap_or_default(),
            Col::Screening => {
                let Some((all, criteria)) = &self.screening else { return String::new() };
                let s = all.get(&paper).cloned().unwrap_or_default();
                let codes: Vec<String> = s.criteria.iter().filter_map(|c| criterion_code(criteria, *c)).collect();
                let mut text = s.decision.label().to_string();
                if !codes.is_empty() {
                    text.push_str(&format!(" ({})", codes.join(", ")));
                }
                if !s.note.trim().is_empty() {
                    text.push_str(&format!(": {}", s.note.trim()));
                }
                text
            }
        }
    }

    fn available(&self, scope: Scope) -> Vec<Col> {
        let mut cols: Vec<Col> = self.fields.iter().filter(|f| !f.hidden && f.kind != FieldKind::Relevance).map(|f| Col::Field(f.id)).collect();
        if matches!(scope, Scope::Collection(_)) {
            cols.push(Col::Relevance);
        }
        if self.screening.is_some() {
            cols.push(Col::Screening);
        }
        cols.extend(self.metric_names.iter().cloned().map(Col::Metric));
        cols
    }

    fn defaults(&self) -> Vec<Col> {
        let mut cols: Vec<Col> = self
            .fields
            .iter()
            .filter(|f| {
                !f.hidden
                    && (matches!(f.key.as_str(), "contribution" | "methodology" | "findings" | "limitations")
                        || (f.collection.is_some() && f.kind == FieldKind::ShortText))
            })
            .map(|f| Col::Field(f.id))
            .collect();
        if self.screening.is_some() {
            cols.insert(0, Col::Screening);
        }
        cols.extend(self.metric_names.iter().take(3).cloned().map(Col::Metric));
        cols
    }

    fn label(r: &PaperRow) -> String {
        match r.year {
            Some(y) => format!("{} ({y})", r.authors),
            None => r.authors.clone(),
        }
    }

    fn table(&self, cols: &[Col]) -> matrix::Table {
        matrix::Table {
            headers: cols.iter().map(|c| self.col_label(c)).collect(),
            rows: self
                .rows
                .iter()
                .map(|r| matrix::Row {
                    label: if r.authors.is_empty() { r.title.clone() } else { Self::label(r) },
                    citekey: r.citekey.clone(),
                    cells: cols.iter().map(|c| self.cell(r.id, c)).collect(),
                })
                .collect(),
        }
    }
}

impl Session {
    fn matrix_data(&mut self, scope: Scope) -> Data {
        let rev = self.lib.revision();
        if let Some((r, s, d)) = &self.matrix.data
            && *r == rev
            && *s == scope
        {
            return d.clone();
        }
        let mut rows = self.lib.rows(scope).unwrap_or_default();
        rows.sort_by(|a, b| a.year.cmp(&b.year).then_with(|| a.authors.cmp(&b.authors)));
        let fields = match scope {
            Scope::Collection(c) => self.lib.fields(Some(c)).unwrap_or_default(),
            _ => self.lib.fields(None).unwrap_or_default(),
        };
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        let notes = self.lib.notes_for(&ids).unwrap_or_default();
        let relevance = match scope {
            Scope::Collection(c) => ids
                .iter()
                .filter_map(|id| self.lib.relevance(c, *id).ok().flatten().map(|t| (*id, t)))
                .collect(),
            _ => HashMap::new(),
        };
        let mut metric_names: Vec<String> = Vec::new();
        for f in fields.iter().filter(|f| f.kind == FieldKind::Metrics) {
            for id in &ids {
                for m in parse_metrics(notes.get(&(*id, f.id)).map(String::as_str).unwrap_or("")) {
                    let name = m.name.trim().to_string();
                    if !name.is_empty() && !metric_names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                        metric_names.push(name);
                    }
                }
            }
        }
        let screening = match scope {
            Scope::Collection(c) if self.collection(c).is_some_and(|c| c.screening) => {
                Some((self.lib.screenings(c).unwrap_or_default(), self.lib.criteria(c).unwrap_or_default()))
            }
            _ => None,
        };
        // In a review, excluded papers go last.
        if let Some((all, _)) = &screening {
            rows.sort_by_key(|r| all.get(&r.id).is_some_and(|s| s.decision == Decision::Exclude));
        }
        let data = Data { rows, fields, notes, relevance, metric_names, screening };
        self.matrix.data = Some((rev, scope, data.clone()));
        data
    }

    pub(crate) fn matrix_ui(&mut self, ui: &mut Ui, scope: Scope) {
        let data = self.matrix_data(scope);
        let available = data.available(scope);
        let chosen: Vec<Col> = self
            .matrix
            .chosen
            .get(&scope)
            .cloned()
            .unwrap_or_else(|| data.defaults())
            .into_iter()
            .filter(|c| available.contains(c))
            .collect();
        let title = format!("Synthesis matrix · {}", self.scope_title(scope));
        let mut new_chosen = None;
        theme::page_header(ui, &title, "Compare papers side by side. Double-click a cell to edit it.", |ui| {
            let resp = ui.button(format!("{} Export {}", icons::EXPORT, icons::CARET_DOWN));
            egui::Popup::menu(&resp).id(egui::Id::unique("matrix-export")).show(|ui| {
                let table = data.table(&chosen);
                let name = safe_file_name(&self.scope_title(scope));
                for (label, ext, text) in [
                    ("CSV (Excel, LibreOffice)…", "csv", matrix::to_csv(&table)),
                    ("Markdown…", "md", matrix::to_markdown(&table)),
                    ("LaTeX table…", "tex", matrix::to_latex(&table)),
                ] {
                    if ui.button(label).clicked() {
                        self.save_text(&format!("{name}-matrix.{ext}"), ext, &text);
                    }
                }
                ui.separator();
                if ui.button(format!("{} Copy as LaTeX", icons::COPY)).clicked() {
                    ui.ctx().copy_text(matrix::to_latex(&table));
                    self.info("LaTeX table copied.");
                }
            });
            let resp = ui.button(format!("{} Columns {}", icons::COLUMNS, icons::CARET_DOWN));
            egui::Popup::menu(&resp)
                .id(egui::Id::unique("matrix-columns"))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    let mut cols = chosen.clone();
                    for col in &available {
                        let mut on = cols.contains(col);
                        let label = match col {
                            Col::Metric(n) => format!("{} {n}", icons::TABLE),
                            other => data.col_label(other),
                        };
                        if ui.checkbox(&mut on, label).changed() {
                            if on {
                                cols.push(col.clone());
                                // Keep the order of `available`.
                                cols.sort_by_key(|c| available.iter().position(|a| a == c));
                            } else {
                                cols.retain(|c| c != col);
                            }
                            new_chosen = Some(cols.clone());
                        }
                    }
                    if data.metric_names.is_empty() {
                        theme::hint(ui, "Metrics typed in a paper's Key Metrics (e.g. Gain) become columns here.");
                    }
                });
            if ui.add(theme::ghost_button(format!("{} Back to list", icons::ARROW_LEFT))).clicked() {
                self.go(View::List(scope));
            }
        });
        if let Some(cols) = new_chosen {
            self.matrix.chosen.insert(scope, cols);
        }
        if data.rows.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("No papers to compare yet.").color(theme::muted(ui))));
            return;
        }
        let mut open = None;
        let mut edit = None;
        egui::ScrollArea::horizontal().auto_shrink(false).show(ui, |ui| {
            let mut table = TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::TOP))
                .column(Column::initial(220.0).at_least(160.0).resizable(true).clip(true));
            for _ in &chosen {
                table = table.column(Column::initial(250.0).at_least(110.0).resizable(true).clip(true));
            }
            table
                .header(34.0, |mut header| {
                    header.col(|ui| {
                        ui.label(theme::semibold("PAPER").size(11.0).extra_letter_spacing(0.6).color(theme::muted(ui)));
                    });
                    for col in &chosen {
                        header.col(|ui| {
                            ui.add(
                                egui::Label::new(theme::semibold(data.col_label(col).to_uppercase()).size(11.0).extra_letter_spacing(0.6).color(theme::muted(ui)))
                                    .truncate(),
                            );
                        });
                    }
                })
                .body(|body| {
                    body.rows(104.0, data.rows.len(), |mut row| {
                        let r = &data.rows[row.index()];
                        row.col(|ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                if ui.link(theme::semibold(Data::label(r))).on_hover_text("Open the paper").clicked() {
                                    open = Some(r.id);
                                }
                                ui.add(egui::Label::new(RichText::new(&r.title).small().color(theme::muted(ui))).wrap());
                            });
                        });
                        for col in &chosen {
                            row.col(|ui| {
                                let text = data.cell(r.id, col);
                                let shown = if text.is_empty() { RichText::new("—").color(theme::muted(ui)) } else { RichText::new(text.clone()).small() };
                                let resp = ui.add(egui::Label::new(shown).wrap().sense(Sense::click()));
                                let resp = if text.len() > 120 { resp.on_hover_text(&text) } else { resp };
                                if resp.double_clicked()
                                    && let Col::Field(fid) = col
                                    && let Some(f) = data.fields.iter().find(|f| f.id == *fid)
                                    && matches!(f.kind, FieldKind::LongText | FieldKind::ShortText)
                                {
                                    edit = Some((r.id, f.id, f.label.clone(), r.title.clone(), text.clone()));
                                }
                            });
                        }
                    });
                });
        });
        if let Some((paper, field, label, title, text)) = edit {
            self.dialog = Some(Dialog::EditCell { paper, field, label, title, text });
        }
        if let Some(id) = open {
            self.open_paper(id);
        }
    }

    /// Asks where to save a text export, then writes it.
    pub fn save_text(&mut self, suggested: &str, ext: &str, text: &str) {
        let Some(path) = rfd::FileDialog::new().set_file_name(suggested).add_filter(ext.to_uppercase(), &[ext]).save_file() else {
            return;
        };
        match export::write_if_changed(&path, text) {
            Ok(_) => self.info(format!("Saved {}.", path.display())),
            Err(e) => self.error(format!("{e:#}")),
        }
    }
}
