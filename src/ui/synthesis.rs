//! A collection's synthesis: overview notes across its papers, themes that
//! group them, and a draft related-work outline.

use std::collections::HashMap;

use anyhow::Result;
use egui::{RichText, Ui};
use egui_commonmark::CommonMarkViewer;

use crate::app::{Session, View, safe_file_name};
use crate::db::{Library, Scope};
use crate::export::outline::{self, OutlinePaper, OutlineTheme};
use crate::model::{Decision, PaperRow, Theme};
use crate::ui::sidebar::plural;
use crate::ui::{icons, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Dirty {
    Notes,
    Outline,
    /// A theme's name or notes.
    Theme(i64),
    /// How a paper fits a theme: (theme, paper).
    PaperNote(i64, i64),
}

pub struct SynthesisPage {
    pub collection: i64,
    pub notes: String,
    pub outline: String,
    pub themes: Vec<Theme>,
    pub new_theme: String,
    pub preview_notes: bool,
    confirm_draft: bool,
    dirty: HashMap<Dirty, f64>,
    /// The collection's papers (screened-out ones left out), cached per library revision.
    rows: (u64, Vec<PaperRow>),
}

/// Asked for while drawing, done afterwards.
enum Action {
    Open(i64),
    AddTheme,
    DeleteTheme(i64),
    MoveTheme(i64, bool),
    AddToTheme(i64, i64),
    RemoveFromTheme(i64, i64),
    Draft,
}

impl SynthesisPage {
    pub fn load(lib: &Library, collection: i64) -> Result<Self> {
        let (notes, outline) = lib.synthesis(collection)?;
        Ok(Self {
            collection,
            preview_notes: !notes.trim().is_empty(),
            notes,
            outline,
            themes: lib.themes(collection)?,
            new_theme: String::new(),
            confirm_draft: false,
            dirty: HashMap::new(),
            rows: (0, Vec::new()),
        })
    }

    pub fn has_unsaved(&self) -> bool {
        !self.dirty.is_empty()
    }

    fn touch(&mut self, key: Dirty, now: f64) {
        self.dirty.insert(key, now);
    }

    /// Writes what was typed more than a moment ago (everything if `now` is None).
    pub fn flush(&mut self, lib: &mut Library, now: Option<f64>) -> Result<()> {
        let due: Vec<Dirty> =
            self.dirty.iter().filter(|(_, at)| now.is_none_or(|n| n - **at >= 0.6)).map(|(k, _)| *k).collect();
        for key in due {
            match key {
                Dirty::Notes => lib.set_synthesis_notes(self.collection, &self.notes)?,
                Dirty::Outline => lib.set_outline(self.collection, &self.outline)?,
                Dirty::Theme(id) => {
                    if let Some(t) = self.themes.iter().find(|t| t.id == id)
                        && !t.name.trim().is_empty()
                    {
                        lib.update_theme(id, &t.name, &t.notes)?;
                    }
                }
                Dirty::PaperNote(tid, pid) => {
                    let note = self.themes.iter().find(|t| t.id == tid).and_then(|t| t.papers.iter().find(|(p, _)| *p == pid));
                    if let Some((_, note)) = note {
                        lib.set_theme_note(tid, pid, note)?;
                    }
                }
            }
            self.dirty.remove(&key);
        }
        Ok(())
    }

    /// Reloads the themes after adding or removing some, keeping what is being typed.
    fn reload_themes(&mut self, lib: &Library) -> Result<()> {
        let old = std::mem::take(&mut self.themes);
        self.themes = lib.themes(self.collection)?;
        for t in &mut self.themes {
            let Some(o) = old.iter().find(|o| o.id == t.id) else { continue };
            if self.dirty.contains_key(&Dirty::Theme(t.id)) {
                t.name = o.name.clone();
                t.notes = o.notes.clone();
            }
            for (pid, note) in &mut t.papers {
                if self.dirty.contains_key(&Dirty::PaperNote(t.id, *pid))
                    && let Some((_, typed)) = o.papers.iter().find(|(p, _)| p == pid)
                {
                    *note = typed.clone();
                }
            }
        }
        Ok(())
    }
}

/// "Zhang et al. (2025)"
fn short_label(r: &PaperRow) -> String {
    match (r.authors.is_empty(), r.year) {
        (true, _) => r.title.chars().take(40).collect(),
        (false, Some(y)) => format!("{} ({y})", r.authors),
        (false, None) => r.authors.clone(),
    }
}

impl Session {
    pub fn flush_synthesis(&mut self, all: bool) {
        let now = self.now;
        if let Some(page) = &mut self.synthesis
            && let Err(e) = page.flush(&mut self.lib, if all { None } else { Some(now) })
        {
            self.error(format!("Could not save the synthesis: {e:#}"));
        }
    }

    /// Saves the synthesis and forgets it, so it is read afresh next time.
    pub fn leave_synthesis(&mut self) {
        self.flush_synthesis(true);
        self.synthesis = None;
    }

    /// The papers a synthesis covers: the collection's, without those excluded by screening.
    fn synthesis_rows(&mut self, page: &mut SynthesisPage) -> Vec<PaperRow> {
        let rev = self.lib.revision();
        if page.rows.0 != rev {
            let mut rows = self.lib.rows(Scope::Collection(page.collection)).unwrap_or_default();
            if self.collection(page.collection).is_some_and(|c| c.screening) {
                let screened = self.lib.screenings(page.collection).unwrap_or_default();
                rows.retain(|r| screened.get(&r.id).is_none_or(|s| s.decision != Decision::Exclude));
            }
            rows.sort_by(|a, b| a.year.cmp(&b.year).then_with(|| a.authors.cmp(&b.authors)));
            page.rows = (rev, rows);
        }
        page.rows.1.clone()
    }

    pub(crate) fn synthesis_ui(&mut self, ui: &mut Ui, id: i64) {
        if self.synthesis.as_ref().is_none_or(|p| p.collection != id) {
            self.flush_synthesis(true);
            match SynthesisPage::load(&self.lib, id) {
                Ok(p) => self.synthesis = Some(p),
                Err(e) => {
                    self.error(format!("Could not open the synthesis: {e:#}"));
                    self.go(View::List(Scope::Collection(id)));
                    return;
                }
            }
        }
        let Some(mut page) = self.synthesis.take() else { return };
        let rows = self.synthesis_rows(&mut page);
        let mut actions = Vec::new();
        let name = self.scope_title(Scope::Collection(id));
        let screened = self.collection(id).is_some_and(|c| c.screening);
        let subtitle = format!(
            "{}{} · group them into themes, note what they share, and draft the related-work section.",
            plural(rows.len(), "paper"),
            if screened { " (excluded ones left out)" } else { "" }
        );
        theme::page_header(ui, &format!("Synthesis · {name}"), &subtitle, |ui| {
            let resp = ui.button(format!("{} Export outline {}", icons::EXPORT, icons::CARET_DOWN));
            egui::Popup::menu(&resp).id(egui::Id::unique("outline-export")).show(|ui| {
                let file = format!("{}-related-work", safe_file_name(&name));
                if ui.button("Markdown…").clicked() {
                    self.save_text(&format!("{file}.md"), "md", &page.outline);
                }
                if ui.button("LaTeX…").clicked() {
                    self.save_text(&format!("{file}.tex"), "tex", &outline::to_latex(&page.outline));
                }
                ui.separator();
                if ui.button(format!("{} Copy as LaTeX", icons::COPY)).clicked() {
                    ui.ctx().copy_text(outline::to_latex(&page.outline));
                    self.info("Outline copied as LaTeX.");
                }
            });
            if ui.button(format!("{} Matrix", icons::TABLE)).clicked() {
                self.go(View::Matrix(Scope::Collection(id)));
            }
            if ui.add(theme::ghost_button(format!("{} Back to list", icons::ARROW_LEFT))).clicked() {
                self.go(View::List(Scope::Collection(id)));
            }
        });
        egui::ScrollArea::vertical().id_salt(("synthesis", id)).auto_shrink(false).show(ui, |ui| {
            if ui.available_width() > 1100.0 {
                ui.columns(2, |cols| {
                    self.themes_card(&mut cols[0], &mut page, &rows, &mut actions);
                    self.notes_card(&mut cols[1], &mut page);
                    cols[1].add_space(12.0);
                    self.outline_card(&mut cols[1], &mut page, &mut actions);
                });
            } else {
                self.themes_card(ui, &mut page, &rows, &mut actions);
                ui.add_space(12.0);
                self.notes_card(ui, &mut page);
                ui.add_space(12.0);
                self.outline_card(ui, &mut page, &mut actions);
            }
            ui.add_space(30.0);
        });
        let mut open = None;
        let mut reload = false;
        for a in actions {
            match a {
                Action::Open(pid) => open = Some(pid),
                Action::AddTheme => {
                    let r = self.lib.add_theme(id, &page.new_theme);
                    if self.check(r).is_some() {
                        page.new_theme.clear();
                        reload = true;
                    }
                }
                Action::DeleteTheme(tid) => {
                    page.dirty.retain(|k, _| !matches!(k, Dirty::Theme(t) | Dirty::PaperNote(t, _) if *t == tid));
                    let r = self.lib.delete_theme(tid);
                    self.check(r);
                    reload = true;
                }
                Action::MoveTheme(tid, up) => {
                    // Save names first: the reload keeps only what is still unsaved.
                    let r = page.flush(&mut self.lib, None);
                    self.check(r);
                    let r = self.lib.move_theme(tid, up);
                    self.check(r);
                    reload = true;
                }
                Action::AddToTheme(tid, pid) => {
                    let r = self.lib.add_to_theme(tid, &[pid]);
                    self.check(r);
                    reload = true;
                }
                Action::RemoveFromTheme(tid, pid) => {
                    page.dirty.remove(&Dirty::PaperNote(tid, pid));
                    let r = self.lib.remove_from_theme(tid, pid);
                    self.check(r);
                    reload = true;
                }
                Action::Draft => {
                    page.outline = self.draft_outline(&page, &rows);
                    page.touch(Dirty::Outline, self.now);
                    page.confirm_draft = false;
                }
            }
        }
        if reload {
            let r = page.reload_themes(&self.lib);
            self.check(r);
        }
        if self.view == View::Synthesis(id) && self.synthesis.is_none() {
            self.synthesis = Some(page);
        } else {
            let r = page.flush(&mut self.lib, None);
            self.check(r);
        }
        if let Some(pid) = open {
            self.open_paper(pid);
        }
    }

    fn themes_card(&mut self, ui: &mut Ui, page: &mut SynthesisPage, rows: &[PaperRow], actions: &mut Vec<Action>) {
        let now = self.now;
        let label_of = |pid: i64| rows.iter().find(|r| r.id == pid);
        theme::card(ui, "Themes", |ui| {
            if page.themes.is_empty() {
                theme::hint(ui, "A theme is an idea several papers share: a method, a finding, a debate. Add one, then add its papers.");
            }
            let count = page.themes.len();
            for (i, t) in page.themes.iter_mut().enumerate() {
                ui.push_id(("theme", t.id), |ui| {
                    egui::Frame::new()
                        .fill(theme::t(ui).raised)
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icons::LIGHTBULB).color(theme::accent(ui)));
                                let buttons = 1.0 + f32::from(u8::from(i > 0)) + f32::from(u8::from(i + 1 < count));
                                let w = (ui.available_width() - buttons * (ui.spacing().interact_size.x + ui.spacing().item_spacing.x) - 8.0).max(120.0);
                                if ui.add(egui::TextEdit::singleline(&mut t.name).font(theme::semibold_font(15.0)).desired_width(w)).changed() {
                                    page.dirty.insert(Dirty::Theme(t.id), now);
                                }
                                if i > 0 && ui.add(theme::ghost_button(icons::CARET_UP)).on_hover_text("Move up").clicked() {
                                    actions.push(Action::MoveTheme(t.id, true));
                                }
                                if i + 1 < count && ui.add(theme::ghost_button(icons::CARET_DOWN)).on_hover_text("Move down").clicked() {
                                    actions.push(Action::MoveTheme(t.id, false));
                                }
                                if ui.add(theme::ghost_button(icons::TRASH)).on_hover_text("Delete the theme (its papers stay)").clicked() {
                                    actions.push(Action::DeleteTheme(t.id));
                                }
                            });
                            let notes = ui.add(
                                egui::TextEdit::multiline(&mut t.notes)
                                    .hint_text("What do these papers share? Where do they agree or disagree?")
                                    .desired_rows(2)
                                    .desired_width(f32::INFINITY),
                            );
                            if notes.changed() {
                                page.dirty.insert(Dirty::Theme(t.id), now);
                            }
                            for (pid, note) in &mut t.papers {
                                let Some(r) = label_of(*pid) else { continue };
                                ui.horizontal(|ui| {
                                    if ui.link(theme::semibold(short_label(r))).on_hover_text(&r.title).clicked() {
                                        actions.push(Action::Open(r.id));
                                    }
                                    // Room for the remove button (buttons are at least `interact_size` wide).
                                    let w = (ui.available_width() - ui.spacing().interact_size.x - 2.0 * ui.spacing().item_spacing.x).max(120.0);
                                    if ui.add(egui::TextEdit::singleline(note).hint_text("how it fits the theme").desired_width(w)).changed() {
                                        page.dirty.insert(Dirty::PaperNote(t.id, *pid), now);
                                    }
                                    if ui.add(theme::ghost_button(icons::X)).on_hover_text("Remove from the theme").clicked() {
                                        actions.push(Action::RemoveFromTheme(t.id, *pid));
                                    }
                                });
                            }
                            let resp = ui.add(theme::ghost_button(format!("{} Add papers {}", icons::PLUS, icons::CARET_DOWN)));
                            egui::Popup::menu(&resp).id(egui::Id::unique(("theme-add", t.id))).show(|ui| {
                                ui.set_max_width(460.0);
                                let left: Vec<&PaperRow> = rows.iter().filter(|r| !t.papers.iter().any(|(p, _)| *p == r.id)).collect();
                                if left.is_empty() {
                                    ui.label(RichText::new("Every paper is in this theme.").color(theme::muted(ui)));
                                }
                                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                    for r in left {
                                        let text = format!("{} · {}", short_label(r), r.title);
                                        if ui.add(egui::Button::new(text).truncate()).clicked() {
                                            actions.push(Action::AddToTheme(t.id, r.id));
                                        }
                                    }
                                });
                            });
                        });
                });
                ui.add_space(8.0);
            }
            ui.horizontal(|ui| {
                let edit = ui.add(egui::TextEdit::singleline(&mut page.new_theme).hint_text("New theme, e.g. Low-cost unit cells").desired_width(280.0));
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let can = !page.new_theme.trim().is_empty();
                if (ui.add_enabled(can, egui::Button::new(format!("{} Add theme", icons::PLUS))).clicked() || enter) && can {
                    actions.push(Action::AddTheme);
                }
            });
            let themed: Vec<i64> = page.themes.iter().flat_map(|t| t.papers.iter().map(|(p, _)| *p)).collect();
            let unthemed: Vec<&PaperRow> = rows.iter().filter(|r| !themed.contains(&r.id)).collect();
            if !page.themes.is_empty() && !unthemed.is_empty() {
                ui.add_space(10.0);
                ui.label(theme::semibold(format!("Not in a theme yet ({})", unthemed.len())).color(theme::muted(ui)));
                for r in unthemed {
                    ui.horizontal(|ui| {
                        let resp = ui.add(theme::ghost_button(format!("{} {}", icons::PLUS, icons::CARET_DOWN))).on_hover_text("Add to a theme");
                        egui::Popup::menu(&resp).id(egui::Id::unique(("unthemed", r.id))).show(|ui| {
                            for t in &page.themes {
                                if ui.button(format!("{} {}", icons::LIGHTBULB, t.name)).clicked() {
                                    actions.push(Action::AddToTheme(t.id, r.id));
                                }
                            }
                        });
                        if ui.link(short_label(r)).clicked() {
                            actions.push(Action::Open(r.id));
                        }
                        ui.add(egui::Label::new(RichText::new(&r.title).small().color(theme::muted(ui))).truncate());
                    });
                }
            }
        });
    }

    fn notes_card(&mut self, ui: &mut Ui, page: &mut SynthesisPage) {
        let now = self.now;
        theme::card_frame(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::card_title(ui, "Overview notes");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (icon, tip) = if page.preview_notes { (icons::PENCIL_SIMPLE, "Edit") } else { (icons::EYE, "Show formatted (Markdown)") };
                    if ui.add(theme::ghost_button(icon)).on_hover_text(tip).clicked() {
                        page.preview_notes = !page.preview_notes;
                    }
                });
            });
            if page.preview_notes && !page.notes.trim().is_empty() {
                let resp = ui
                    .scope(|ui| {
                        CommonMarkViewer::new().show(ui, &mut self.md_cache, &page.notes);
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_text("Double-click to edit");
                if resp.double_clicked() {
                    page.preview_notes = false;
                }
            } else if ui
                .add(
                    egui::TextEdit::multiline(&mut page.notes)
                        .hint_text("Patterns across the papers: what most agree on, where they clash, which methods dominate, what nobody has tried.")
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                page.touch(Dirty::Notes, now);
            }
        });
    }

    fn outline_card(&mut self, ui: &mut Ui, page: &mut SynthesisPage, actions: &mut Vec<Action>) {
        let now = self.now;
        theme::card(ui, "Related-work outline", |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add(theme::primary_button(ui, format!("{} Draft from themes", icons::SPARKLE)))
                    .on_hover_text("A section per theme with its papers and \\cite keys, then the open problems from the papers' notes")
                    .clicked()
                {
                    if page.outline.trim().is_empty() {
                        actions.push(Action::Draft);
                    } else {
                        page.confirm_draft = true;
                    }
                }
                if ui.button(format!("{} Copy", icons::COPY)).clicked() {
                    ui.ctx().copy_text(page.outline.clone());
                    self.info("Outline copied.");
                }
            });
            if page.confirm_draft {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} Replace the outline below with a new draft?", icons::WARNING)).color(theme::fg(ui, theme::AMBER)));
                    if ui.add(theme::danger_button("Replace")).clicked() {
                        actions.push(Action::Draft);
                    }
                    if ui.button("Cancel").clicked() {
                        page.confirm_draft = false;
                    }
                });
            }
            theme::hint(ui, "Markdown. Edit it freely: drafting again replaces it. Export it as LaTeX for your thesis.");
            ui.add_space(4.0);
            if ui
                .add(
                    egui::TextEdit::multiline(&mut page.outline)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("# Related work\n\n## A theme\n- Zhang et al. (2025) \\cite{zhang2025Wideband}: …")
                        .desired_rows(16)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                page.touch(Dirty::Outline, now);
            }
        });
    }

    /// The outline drafted from the themes and the papers' Contribution and
    /// Research Gaps notes.
    fn draft_outline(&self, page: &SynthesisPage, rows: &[PaperRow]) -> String {
        let fields = self.lib.fields(None).unwrap_or_default();
        let field = |key: &str| fields.iter().find(|f| f.key == key).map(|f| f.id);
        let (contribution, gaps) = (field("contribution"), field("gaps"));
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        let notes = self.lib.notes_for(&ids).unwrap_or_default();
        let note = |pid: i64, fid: Option<i64>| fid.and_then(|f| notes.get(&(pid, f)).cloned()).unwrap_or_default();
        let paper = |r: &PaperRow, fits: &str| OutlinePaper {
            citekey: r.citekey.clone(),
            label: short_label(r),
            title: r.title.clone(),
            summary: if fits.trim().is_empty() { note(r.id, contribution) } else { fits.to_string() },
            gaps: note(r.id, gaps),
        };
        let themes: Vec<OutlineTheme> = page
            .themes
            .iter()
            .map(|t| OutlineTheme {
                name: t.name.clone(),
                notes: t.notes.clone(),
                papers: t.papers.iter().filter_map(|(pid, fits)| rows.iter().find(|r| r.id == *pid).map(|r| paper(r, fits))).collect(),
            })
            .collect();
        let themed: Vec<i64> = page.themes.iter().flat_map(|t| t.papers.iter().map(|(p, _)| *p)).collect();
        let unthemed: Vec<OutlinePaper> = rows.iter().filter(|r| !themed.contains(&r.id)).map(|r| paper(r, "")).collect();
        let all: Vec<OutlinePaper> = rows.iter().map(|r| paper(r, "")).collect();
        let name = self.scope_title(Scope::Collection(page.collection));
        outline::draft(&name, &themes, &unthemed, &all)
    }
}
