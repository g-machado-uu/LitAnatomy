//! A list of papers: all, unfiled, the Trash or one collection.

use std::collections::{BTreeSet, HashMap, HashSet};

use egui::{RichText, Sense, Ui};
use egui_extras::{Column, TableBuilder};

use crate::app::{PaperDrag, Session, View};
use crate::db::{Scope, ScreeningSummary};
use crate::model::{Criterion, Decision, PaperRow, ReadingStatus, Screening, criterion_code};
use crate::ui::dialogs::{CollectionForm, Dialog};
use crate::ui::sidebar::plural;
use crate::ui::{icons, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    NotStarted,
    Reading,
    NoPdf,
    NoDoi,
}

impl Attention {
    fn label(self) -> &'static str {
        match self {
            Self::NotStarted => "Not dissected yet",
            Self::Reading => "Being read",
            Self::NoPdf => "No PDF",
            Self::NoDoi => "No DOI",
        }
    }

    fn keeps(self, r: &PaperRow) -> bool {
        match self {
            Self::NotStarted => r.filled == 0,
            Self::Reading => r.status == ReadingStatus::Reading,
            Self::NoPdf => !r.has_pdf,
            Self::NoDoi => !r.has_doi,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Added,
    Title,
    Year,
    Status,
    Progress,
}

#[derive(Default)]
pub struct ListState {
    pub search: String,
    pub status: Option<ReadingStatus>,
    pub tag: Option<i64>,
    pub attention: Option<Attention>,
    pub sort: SortKey,
    pub ascending: bool,
    pub selected: BTreeSet<i64>,
    pub focus_search: bool,
    /// Screening filter, in collections that screen.
    pub decision: Option<Decision>,
    rows: Option<(u64, Scope, Vec<PaperRow>)>,
    hits: Option<(u64, String, HashSet<i64>)>,
    review: Option<(u64, i64, Review)>,
}

/// A review collection's screening, cached per library revision.
#[derive(Clone, Default)]
pub struct Review {
    pub screenings: HashMap<i64, Screening>,
    pub criteria: Vec<Criterion>,
    pub summary: ScreeningSummary,
}

impl Review {
    /// "Excluded · EC1, EC3" style codes of a paper's ticked criteria.
    fn codes(&self, s: &Screening) -> String {
        s.criteria.iter().filter_map(|c| criterion_code(&self.criteria, *c)).collect::<Vec<_>>().join(", ")
    }

    /// The counts for a PRISMA flow diagram, as plain text.
    pub fn prisma_text(&self, name: &str) -> String {
        let s = &self.summary;
        let mut out = format!("Screening of {name}\n");
        out.push_str(&format!("Records screened: {}\n", s.total() - s.count(Decision::Undecided)));
        out.push_str(&format!("Not screened yet: {}\n", s.count(Decision::Undecided)));
        out.push_str(&format!("Included: {}\n", s.count(Decision::Include)));
        out.push_str(&format!("Undecided (maybe): {}\n", s.count(Decision::Maybe)));
        out.push_str(&format!("Excluded: {}\n", s.count(Decision::Exclude)));
        for (cid, n) in &s.excluded_by {
            let label = self.criteria.iter().find(|c| c.id == *cid).map(|c| c.label.as_str()).unwrap_or_default();
            let code = criterion_code(&self.criteria, *cid).unwrap_or_default();
            out.push_str(&format!("  {code} {label}: {n}\n"));
        }
        if s.excluded_without_reason > 0 {
            out.push_str(&format!("  No criterion given: {}\n", s.excluded_without_reason));
        }
        out
    }
}

impl Session {
    /// The screening of a review collection (None if the collection doesn't screen).
    pub(crate) fn review(&mut self, collection: i64) -> Option<Review> {
        if !self.collection(collection).is_some_and(|c| c.screening) {
            return None;
        }
        let rev = self.lib.revision();
        if !matches!(&self.list.review, Some((r, c, _)) if *r == rev && *c == collection) {
            let review = Review {
                screenings: self.lib.screenings(collection).unwrap_or_default(),
                criteria: self.lib.criteria(collection).unwrap_or_default(),
                summary: self.lib.screening_summary(collection).unwrap_or_default(),
            };
            self.list.review = Some((rev, collection, review));
        }
        self.list.review.as_ref().map(|(_, _, r)| r.clone())
    }

    /// The rows of `scope` after search, filters and sorting.
    pub(crate) fn visible_rows(&mut self, scope: Scope) -> Vec<PaperRow> {
        let rev = self.lib.revision();
        if !matches!(&self.list.rows, Some((r, s, _)) if *r == rev && *s == scope) {
            let rows = self.lib.rows(scope);
            let rows = self.check(rows).unwrap_or_default();
            self.list.rows = Some((rev, scope, rows));
        }
        let query = self.list.search.trim().to_string();
        if !query.is_empty() && !matches!(&self.list.hits, Some((r, q, _)) if *r == rev && *q == query) {
            let hits = self.lib.search(&query).unwrap_or_default();
            self.list.hits = Some((rev, query.clone(), hits));
        }
        let tagged = self.list.tag.map(|t| self.papers_with_tag(t));
        let review = match (scope, self.list.decision) {
            (Scope::Collection(c), Some(_)) => self.review(c),
            _ => None,
        };
        let l = &self.list;
        let mut rows: Vec<PaperRow> = l.rows.as_ref().map(|(_, _, r)| r.clone()).unwrap_or_default();
        rows.retain(|r| {
            (query.is_empty() || l.hits.as_ref().is_some_and(|(_, _, h)| h.contains(&r.id)))
                && l.status.is_none_or(|s| r.status == s)
                && tagged.as_ref().is_none_or(|t| t.contains(&r.id))
                && l.attention.is_none_or(|a| a.keeps(r))
                && match (&review, l.decision) {
                    (Some(rv), Some(d)) => rv.screenings.get(&r.id).map(|s| s.decision).unwrap_or_default() == d,
                    _ => true,
                }
        });
        let key = |r: &PaperRow| r.title.to_lowercase();
        match l.sort {
            SortKey::Added => {}
            SortKey::Title => rows.sort_by_key(key),
            SortKey::Year => rows.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| key(a).cmp(&key(b)))),
            SortKey::Status => rows.sort_by(|a, b| b.status.to_i64().cmp(&a.status.to_i64()).then_with(|| key(a).cmp(&key(b)))),
            SortKey::Progress => rows.sort_by(|a, b| b.filled.cmp(&a.filled).then_with(|| key(a).cmp(&key(b)))),
        }
        if l.ascending {
            rows.reverse();
        }
        rows
    }

    pub(crate) fn list_ui(&mut self, ui: &mut Ui, scope: Scope) {
        let rows = self.visible_rows(scope);
        let total = self.list.rows.as_ref().map(|(_, _, r)| r.len()).unwrap_or(0);
        let title = self.scope_title(scope);
        let subtitle = match scope {
            Scope::Collection(id) => {
                let desc = self.collection(id).map(|c| c.description.clone()).unwrap_or_default();
                let stats = self.cache.stats.get(&id).copied().unwrap_or((0, 0));
                let progress = format!("{} · {} summarised", plural(stats.0, "paper"), stats.1);
                if desc.is_empty() { progress } else { format!("{desc} · {progress}") }
            }
            Scope::Trash => "Papers here can be restored, or deleted for good.".into(),
            Scope::Unfiled => "Papers that are not in any collection yet.".into(),
            Scope::All => format!("{} in your library", plural(total, "paper")),
        };
        theme::page_header(ui, &title, &subtitle, |ui| {
            if scope == Scope::Trash {
                if total > 0 && ui.add(theme::danger_button(format!("{} Empty Trash", icons::TRASH))).clicked() {
                    self.dialog = Some(Dialog::ConfirmEmptyTrash);
                }
                return;
            }
            if let Scope::Collection(id) = scope
                && ui.button(icons::GEAR_SIX).on_hover_text("Collection settings: name, custom fields, auto-export").clicked()
                && let Some(form) = CollectionForm::load(&self.lib, id)
            {
                self.dialog = Some(Dialog::EditCollection(Box::new(form)));
            }
            if ui.button(format!("{} Export .bib", icons::EXPORT)).on_hover_text("Export these papers as BibTeX").clicked() {
                let ids: Vec<i64> = match scope {
                    Scope::Collection(id) => self.lib.export_papers(id).unwrap_or_default().iter().map(|p| p.id).collect(),
                    _ => rows.iter().map(|r| r.id).collect(),
                };
                self.export_bib(&ids, &title);
            }
            if ui
                .button(format!("{} Export notes…", icons::FILE_TEXT))
                .on_hover_text("Save these papers' dissections as a PDF, Word, Excel, CSV or Markdown file")
                .clicked()
            {
                let ids: Vec<i64> = if self.list.selected.is_empty() {
                    rows.iter().map(|r| r.id).collect()
                } else {
                    rows.iter().map(|r| r.id).filter(|id| self.list.selected.contains(id)).collect()
                };
                let context = if let Scope::Collection(id) = scope { Some(id) } else { None };
                self.export_notes(ids, context, title.clone());
            }
            if ui.button(format!("{} Matrix", icons::TABLE)).on_hover_text("Compare these papers side by side").clicked() {
                self.go(View::Matrix(scope));
            }
            if let Scope::Collection(id) = scope
                && ui
                    .button(format!("{} Synthesis", icons::TREE_STRUCTURE))
                    .on_hover_text("Themes across these papers, and a draft related-work outline")
                    .clicked()
            {
                self.go(View::Synthesis(id));
            }
        });
        let review = match scope {
            Scope::Collection(id) => self.review(id),
            _ => None,
        };
        self.toolbar(ui);
        if let (Some(review), Scope::Collection(id)) = (&review, scope) {
            self.screening_bar(ui, id, review);
        }
        if !self.list.selected.is_empty() {
            self.selection_bar(ui, scope, &rows, review.is_some());
        }
        if rows.is_empty() {
            self.empty_list(ui, scope, total);
            return;
        }
        self.paper_table(ui, &rows, scope, review.as_ref());
    }

    /// A review's progress: papers per decision (click to filter), and why papers were excluded.
    fn screening_bar(&mut self, ui: &mut Ui, collection: i64, review: &Review) {
        let s = &review.summary;
        egui::Frame::new()
            .fill(theme::t(ui).raised)
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::semibold(format!("{} Screening", icons::FUNNEL)));
                    theme::progress(ui, if s.total() == 0 { 0.0 } else { 1.0 - s.count(Decision::Undecided) as f32 / s.total() as f32 }, 80.0, theme::accent(ui));
                    for d in Decision::ALL {
                        let (icon, colour) = theme::decision_style(d);
                        let active = self.list.decision == Some(d);
                        let text = format!("{icon} {} {}", d.label(), s.count(d));
                        let resp = theme::chip_button(ui, text, if active { theme::accent(ui) } else { colour });
                        if resp.on_hover_text(if active { "Show all papers" } else { "Show only these" }).clicked() {
                            self.list.decision = if active { None } else { Some(d) };
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(theme::ghost_button(format!("{} PRISMA counts", icons::COPY)))
                            .on_hover_text("Copy the numbers for a PRISMA flow diagram")
                            .clicked()
                        {
                            let name = self.scope_title(Scope::Collection(collection));
                            ui.ctx().copy_text(review.prisma_text(&name));
                            self.info("Screening counts copied.");
                        }
                    });
                });
                let reasons: Vec<String> = s
                    .excluded_by
                    .iter()
                    .filter(|(_, n)| *n > 0)
                    .filter_map(|(c, n)| {
                        let crit = review.criteria.iter().find(|x| x.id == *c)?;
                        Some(format!("{} {} ({n})", criterion_code(&review.criteria, *c)?, crit.label))
                    })
                    .chain((s.excluded_without_reason > 0).then(|| format!("no criterion ({})", s.excluded_without_reason)))
                    .collect();
                if !reasons.is_empty() {
                    ui.add(egui::Label::new(RichText::new(format!("Excluded for: {}", reasons.join(" · "))).small().color(theme::muted(ui))).wrap());
                }
                if review.criteria.is_empty() {
                    theme::hint(ui, "No criteria yet: add inclusion and exclusion criteria in the collection's settings (the gear button).");
                }
            });
        ui.add_space(6.0);
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.list.search)
                    .hint_text(format!("{}  Search titles, authors, notes…", icons::MAGNIFYING_GLASS))
                    .desired_width(320.0),
            );
            if std::mem::take(&mut self.list.focus_search) {
                search.request_focus();
            }
            let status_text = self.list.status.map(|s| s.label()).unwrap_or("Any status");
            egui::ComboBox::from_id_salt("status-filter").selected_text(status_text).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.list.status, None, "Any status");
                for s in ReadingStatus::ALL {
                    ui.selectable_value(&mut self.list.status, Some(s), s.label());
                }
            });
            if !self.cache.tags.is_empty() {
                let tag_text = self
                    .list
                    .tag
                    .and_then(|t| self.cache.tags.iter().find(|x| x.0 == t))
                    .map(|t| format!("{} {}", icons::TAG, t.1))
                    .unwrap_or_else(|| "Any tag".into());
                egui::ComboBox::from_id_salt("tag-filter").selected_text(tag_text).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.list.tag, None, "Any tag");
                    for (id, name, _) in &self.cache.tags {
                        ui.selectable_value(&mut self.list.tag, Some(*id), name);
                    }
                });
            }
            if let Some(a) = self.list.attention
                && theme::chip_button(ui, format!("{}  {}", a.label(), icons::X), theme::AMBER).on_hover_text("Clear this filter").clicked()
            {
                self.list.attention = None;
            }
            let filtered = !self.list.search.is_empty()
                || self.list.status.is_some()
                || self.list.tag.is_some()
                || self.list.attention.is_some()
                || self.list.decision.is_some();
            if filtered && ui.add(theme::ghost_button(format!("{} Clear filters", icons::X))).clicked() {
                self.list.search.clear();
                self.list.status = None;
                self.list.tag = None;
                self.list.attention = None;
                self.list.decision = None;
            }
        });
        ui.add_space(6.0);
    }

    fn selection_bar(&mut self, ui: &mut Ui, scope: Scope, rows: &[PaperRow], screening: bool) {
        let ids: Vec<i64> = self.list.selected.iter().copied().collect();
        egui::Frame::new()
            .fill(theme::t(ui).soft)
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::semibold(format!("{} selected", ids.len())));
                    ui.add_space(8.0);
                    if scope == Scope::Trash {
                        if ui.button(format!("{} Restore", icons::ARROW_COUNTER_CLOCKWISE)).clicked() {
                            let r = self.lib.restore(&ids);
                            self.check(r);
                            self.list.selected.clear();
                        }
                        if ui.add(theme::danger_button(format!("{} Delete for good…", icons::TRASH))).clicked() {
                            self.dialog = Some(Dialog::ConfirmPurge(ids.clone()));
                        }
                    } else {
                        let resp = ui.button(format!("{} Add to collection {}", icons::FOLDER_SIMPLE_PLUS, icons::CARET_DOWN));
                        let collections = self.cache.collections.clone();
                        egui::Popup::menu(&resp).id(egui::Id::unique("add-to-collection")).show(|ui| {
                            if collections.is_empty() {
                                ui.label(RichText::new("No collections yet").color(theme::muted(ui)));
                            }
                            for c in &collections {
                                let label = RichText::new(format!("●  {}", c.name)).color(theme::fg(ui, theme::label_colour(c.colour)));
                                if ui.button(label).clicked() {
                                    let r = self.lib.add_to_collection(c.id, &ids);
                                    if let Some(n) = self.check(r) {
                                        self.info(format!("Added {} to {}.", plural(n, "paper"), c.name));
                                    }
                                }
                            }
                        });
                        if let Scope::Collection(id) = scope
                            && ui.button(format!("{} Remove from collection", icons::X)).clicked()
                        {
                            let r = self.lib.remove_from_collection(id, &ids);
                            self.check(r);
                            self.list.selected.clear();
                        }
                        let resp = ui.button(format!("Set status {}", icons::CARET_DOWN));
                        egui::Popup::menu(&resp).id(egui::Id::unique("set-status")).show(|ui| {
                            for s in ReadingStatus::ALL {
                                let (icon, colour) = theme::status_style(s);
                                if ui.button(RichText::new(format!("{icon}  {}", s.label())).color(theme::fg(ui, colour))).clicked() {
                                    for id in &ids {
                                        let r = self.lib.set_status(*id, s);
                                        self.check(r);
                                    }
                                }
                            }
                        });
                        if let (Scope::Collection(id), true) = (scope, screening) {
                            let resp = ui.button(format!("{} Set decision {}", icons::FUNNEL, icons::CARET_DOWN));
                            egui::Popup::menu(&resp).id(egui::Id::unique("set-decision")).show(|ui| {
                                for d in Decision::ALL {
                                    let (icon, colour) = theme::decision_style(d);
                                    if ui.button(RichText::new(format!("{icon}  {}", d.verb())).color(theme::fg(ui, colour))).clicked() {
                                        let r = self.lib.set_decision(id, &ids, d);
                                        if self.check(r).is_some() {
                                            self.info(format!("{}: {}.", d.label(), plural(ids.len(), "paper")));
                                        }
                                    }
                                }
                            });
                        }
                        if ui.button(format!("{} Export .bib", icons::EXPORT)).clicked() {
                            let name = format!("{}-selection", self.scope_title(scope));
                            self.export_bib(&ids, &name);
                        }
                        if ui
                            .button(format!("{} Export notes…", icons::FILE_TEXT))
                            .on_hover_text("Save the selected papers' dissections as a PDF, Word, Excel, CSV or Markdown file")
                            .clicked()
                        {
                            // In the order the list shows them.
                            let ordered: Vec<i64> = rows.iter().map(|r| r.id).filter(|id| ids.contains(id)).collect();
                            let context = if let Scope::Collection(id) = scope { Some(id) } else { None };
                            let title = self.scope_title(scope);
                            self.export_notes(ordered, context, title);
                        }
                        if ui.button(format!("{} Move to Trash", icons::TRASH)).clicked() {
                            let r = self.lib.trash(&ids);
                            if self.check(r).is_some() {
                                self.info(format!("Moved {} to the Trash.", plural(ids.len(), "paper")));
                            }
                            self.list.selected.clear();
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(theme::ghost_button("Clear selection")).clicked() {
                            self.list.selected.clear();
                        }
                        let visible: Vec<i64> = rows.iter().map(|r| r.id).collect();
                        if visible.iter().any(|id| !self.list.selected.contains(id)) && ui.add(theme::ghost_button("Select all")).clicked() {
                            self.list.selected.extend(visible);
                        }
                    });
                });
            });
        ui.add_space(6.0);
    }

    fn empty_list(&mut self, ui: &mut Ui, scope: Scope, total: usize) {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            if total > 0 {
                ui.label(RichText::new(icons::MAGNIFYING_GLASS).size(36.0).color(theme::muted(ui)));
                ui.label(theme::semibold("No papers match these filters."));
                return;
            }
            match scope {
                Scope::Trash => {
                    ui.label(RichText::new(icons::TRASH).size(36.0).color(theme::muted(ui)));
                    ui.label(theme::semibold("The Trash is empty."));
                }
                Scope::Unfiled => {
                    ui.label(RichText::new(icons::CHECK_CIRCLE).size(36.0).color(theme::muted(ui)));
                    ui.label(theme::semibold("Every paper is in a collection."));
                }
                _ => {
                    ui.label(RichText::new(icons::BOOKS).size(40.0).color(theme::muted(ui)));
                    ui.label(theme::semibold("No papers here yet"));
                    let hint = if matches!(scope, Scope::Collection(_)) {
                        "Add papers by DOI, from their PDFs or from a file, or drag papers here from All papers."
                    } else {
                        "Add papers by DOI, from their PDFs, or import a BibTeX, RIS, RefWorks or CSL-JSON file."
                    };
                    ui.label(RichText::new(hint).color(theme::muted(ui)));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 470.0).max(0.0) / 2.0);
                        if ui.add(theme::primary_button(ui, format!("{} Add by DOI", icons::MAGIC_WAND))).clicked() {
                            self.dialog = Some(Dialog::AddDoi { text: String::new(), target: self.current_collection() });
                        }
                        if ui.button(format!("{} From PDFs…", icons::FILE_PDF)).clicked() {
                            self.pick_pdfs(ui.ctx());
                        }
                        if ui.button(format!("{} Import a file…", icons::DOWNLOAD_SIMPLE)).clicked() {
                            self.pick_import_file();
                        }
                    });
                }
            }
        });
    }

    fn paper_table(&mut self, ui: &mut Ui, rows: &[PaperRow], scope: Scope, review: Option<&Review>) {
        let mut open = None;
        let mut toggle = None;
        let all_selected = rows.iter().all(|r| self.list.selected.contains(&r.id));
        let mut select_all = None;
        let mut sort_clicked = None;
        let mut headers = vec![
            (None, ""),
            (None, ""),
            (Some(SortKey::Title), "Paper"),
            (Some(SortKey::Year), "Year"),
            (Some(SortKey::Status), "Status"),
        ];
        if review.is_some() {
            headers.push((None, "Screening"));
        }
        headers.extend([(Some(SortKey::Progress), "Dissected"), (None, "PDF")]);
        let (sort, ascending) = (self.list.sort, self.list.ascending);
        let mut table = TableBuilder::new(ui)
            .striped(false)
            .sense(Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(26.0))
            .column(Column::exact(18.0))
            .column(Column::remainder().at_least(260.0).clip(true))
            .column(Column::exact(56.0))
            .column(Column::exact(124.0));
        if review.is_some() {
            table = table.column(Column::exact(150.0).clip(true));
        }
        table
            .column(Column::exact(112.0))
            .column(Column::exact(40.0))
            .header(30.0, |mut header| {
                for (i, (key, label)) in headers.iter().enumerate() {
                    header.col(|ui| {
                        if i == 0 {
                            let mut all = all_selected;
                            if ui.checkbox(&mut all, "").on_hover_text("Select all").changed() {
                                select_all = Some(all);
                            }
                            return;
                        }
                        let arrow = match key {
                            Some(k) if *k == sort => {
                                if ascending { format!(" {}", icons::SORT_ASCENDING) } else { format!(" {}", icons::SORT_DESCENDING) }
                            }
                            _ => String::new(),
                        };
                        let text = theme::semibold(format!("{}{arrow}", label.to_uppercase())).size(11.0).extra_letter_spacing(0.6).color(theme::muted(ui));
                        match key {
                            Some(k) => {
                                if ui.add(egui::Label::new(text).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    sort_clicked = Some(*k);
                                }
                            }
                            None => {
                                ui.label(text);
                            }
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(52.0, rows.len(), |mut row| {
                    let r = &rows[row.index()];
                    let selected = self.list.selected.contains(&r.id);
                    row.set_selected(selected);
                    row.col(|ui| {
                        let mut on = selected;
                        if ui.checkbox(&mut on, "").changed() {
                            toggle = Some(r.id);
                        }
                    });
                    row.col(|ui| {
                        let handle = ui
                            .add(egui::Label::new(RichText::new(icons::DOTS_SIX_VERTICAL).color(theme::muted(ui))).sense(Sense::drag()))
                            .on_hover_text("Drag onto a collection in the sidebar")
                            .on_hover_cursor(egui::CursorIcon::Grab);
                        let ids = if selected { self.list.selected.iter().copied().collect() } else { vec![r.id] };
                        let count = ids.len();
                        handle.dnd_set_drag_payload(PaperDrag(ids));
                        if handle.dragged() {
                            drag_label(ui, if count == 1 { r.title.clone() } else { plural(count, "paper") });
                        }
                    });
                    row.col(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.add(egui::Label::new(theme::semibold(&r.title)).truncate().selectable(false));
                            let meta = [r.authors.as_str(), r.container.as_str()]
                                .into_iter()
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                                .join(" · ");
                            ui.add(
                                egui::Label::new(RichText::new(format!("{meta}   {}", r.citekey)).small().color(theme::muted(ui)))
                                    .truncate()
                                    .selectable(false),
                            );
                        });
                    });
                    row.col(|ui| {
                        ui.label(r.year.map(|y| y.to_string()).unwrap_or_default());
                    });
                    row.col(|ui| {
                        theme::status_chip(ui, r.status);
                    });
                    if let Some(review) = review {
                        row.col(|ui| {
                            let s = review.screenings.get(&r.id).cloned().unwrap_or_default();
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                let resp = theme::decision_chip(ui, s.decision);
                                let codes = review.codes(&s);
                                if !codes.is_empty() {
                                    ui.label(RichText::new(codes).small().color(theme::muted(ui)));
                                }
                                if !s.note.is_empty() {
                                    resp.on_hover_text(&s.note);
                                }
                            });
                        });
                    }
                    row.col(|ui| {
                        let f = if r.field_total == 0 { 0.0 } else { r.filled as f32 / r.field_total as f32 };
                        theme::progress(ui, f, 64.0, theme::accent(ui));
                        ui.label(RichText::new(format!("{}/{}", r.filled, r.field_total)).small().color(theme::muted(ui)));
                    });
                    row.col(|ui| {
                        if r.has_pdf {
                            ui.label(RichText::new(icons::FILE_PDF).size(17.0).color(theme::fg(ui, theme::RED))).on_hover_text("PDF linked");
                        }
                    });
                    if row.response().clicked() {
                        open = Some(r.id);
                    }
                });
            });
        if let Some(k) = sort_clicked {
            if self.list.sort == k {
                self.list.ascending = !self.list.ascending;
            } else {
                self.list.sort = k;
                self.list.ascending = false;
            }
        }
        if let Some(all) = select_all {
            if all {
                self.list.selected.extend(rows.iter().map(|r| r.id));
            } else {
                self.list.selected.clear();
            }
        }
        if let Some(id) = toggle
            && !self.list.selected.remove(&id)
        {
            self.list.selected.insert(id);
        }
        if let Some(id) = open {
            let _ = scope;
            self.open_paper(id);
        }
    }
}

/// The label that follows the pointer while dragging papers.
pub fn drag_label(ui: &Ui, text: String) {
    if let Some(pos) = ui.ctx().pointer_interact_pos() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        egui::Area::new(egui::Id::unique("paper-drag"))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .fixed_pos(pos + egui::vec2(14.0, 10.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(320.0);
                    ui.add(egui::Label::new(format!("{} {text}", icons::BOOK_OPEN_TEXT)).truncate());
                });
            });
    }
}
