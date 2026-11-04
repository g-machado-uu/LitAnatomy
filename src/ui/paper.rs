//! A paper's page: its anatomy (the notes fields), metadata, links and BibTeX.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use egui::{RichText, Ui};
use egui_commonmark::CommonMarkViewer;

use crate::app::{Lookup, Session, View};
use crate::db::{Library, LinkView, Scope};
use crate::export::bibtex;
use crate::import::{doi, names, parse_year};
use crate::model::{
    Biblio, Field, FieldKind, Metric, Paper, Quote, ReadingStatus, Relation, authors_long, json_or_empty,
    parse_metrics, parse_quotes,
};
use crate::ui::sidebar::plural;
use crate::ui::{icons, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperTab {
    Anatomy,
    Metadata,
    Links,
    Bibtex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Dirty {
    Field(i64),
    Relevance(i64),
}

/// The metadata form's text, saved with an explicit button.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetaForm {
    pub bib: Biblio,
    pub authors: String,
    pub year: String,
    pub doi: String,
    pub key: String,
    pub dirty: bool,
}

impl MetaForm {
    fn from_paper(p: &Paper) -> Self {
        Self {
            bib: p.bib.clone(),
            authors: p
                .bib
                .authors
                .iter()
                .map(|a| if a.given.is_empty() { a.family.clone() } else { format!("{}, {}", a.family, a.given) })
                .collect::<Vec<_>>()
                .join("\n"),
            year: p.bib.year.map(|y| y.to_string()).unwrap_or_default(),
            doi: p.bib.doi.clone().unwrap_or_default(),
            key: p.citekey.clone(),
            dirty: false,
        }
    }

    /// The form as bibliographic data, or what is wrong with it.
    fn to_biblio(&self) -> Result<Biblio> {
        let mut b = self.bib.clone();
        b.title = self.bib.title.split_whitespace().collect::<Vec<_>>().join(" ");
        b.authors = self.authors.lines().map(str::trim).filter(|l| !l.is_empty()).map(names::parse_plain).collect();
        b.year = if self.year.trim().is_empty() {
            None
        } else {
            Some(parse_year(&self.year).ok_or_else(|| anyhow::anyhow!("The year should be four digits, like 2025."))?)
        };
        b.doi = if self.doi.trim().is_empty() {
            None
        } else {
            Some(doi::normalise(&self.doi).ok_or_else(|| anyhow::anyhow!("That doesn't look like a DOI (10.xxxx/…)."))?)
        };
        b.pages = crate::import::normalise_pages(&b.pages);
        if b.title.trim().is_empty() {
            bail!("The title can't be empty.");
        }
        Ok(b)
    }
}

#[derive(Default)]
pub struct LinkForm {
    pub relation: Option<Relation>,
    pub query: String,
    pub target: Option<i64>,
    pub note: String,
}

pub struct PaperPage {
    pub id: i64,
    /// The collection it was opened from: its relevance note and custom fields come first.
    pub context: Option<i64>,
    pub paper: Paper,
    pub tab: PaperTab,
    pub fields: Vec<Field>,
    pub texts: HashMap<i64, String>,
    pub quotes: HashMap<i64, Vec<Quote>>,
    pub metrics: HashMap<i64, Vec<Metric>>,
    /// (collection, why it matters there)
    pub relevance: Vec<(i64, String)>,
    dirty: HashMap<Dirty, f64>,
    pub preview: HashSet<i64>,
    pub meta: MetaForm,
    pub tags: Vec<String>,
    pub tag_input: String,
    pub links: Vec<LinkView>,
    pub link_form: LinkForm,
    pub rating: u8,
}

/// Navigation asked for while drawing a page, done once it's drawn.
enum PageAction {
    Back,
    Open(i64),
    Collection(i64),
}

const ENTRY_TYPES: &[(&str, &str)] = &[
    ("article", "Journal article"),
    ("inproceedings", "Conference paper"),
    ("incollection", "Book chapter"),
    ("book", "Book"),
    ("phdthesis", "PhD thesis"),
    ("mastersthesis", "Master's thesis"),
    ("techreport", "Report"),
    ("online", "Web page / preprint"),
    ("misc", "Other"),
];

pub fn entry_type_label(t: &str) -> &str {
    ENTRY_TYPES.iter().find(|(k, _)| *k == t).map(|(_, l)| *l).unwrap_or(t)
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

impl PaperPage {
    pub fn load(lib: &Library, id: i64, context: Option<i64>) -> Result<Self> {
        let paper = lib.paper(id)?;
        let memberships = lib.memberships(id)?;
        // Only keep the context if the paper is (still) in that collection.
        let context = context.filter(|c| memberships.iter().any(|(m, _)| m == c));
        let mut page = PaperPage {
            id,
            context,
            meta: MetaForm::from_paper(&paper),
            rating: paper.rating,
            paper,
            tab: PaperTab::Anatomy,
            fields: Vec::new(),
            texts: HashMap::new(),
            quotes: HashMap::new(),
            metrics: HashMap::new(),
            relevance: memberships,
            dirty: HashMap::new(),
            preview: HashSet::new(),
            tags: lib.tags_of(id)?,
            tag_input: String::new(),
            links: lib.links_of(id)?,
            link_form: LinkForm::default(),
        };
        page.load_fields(lib)?;
        // Filled-in long fields start as formatted text; empty ones as editors.
        page.preview = page
            .fields
            .iter()
            .filter(|f| f.kind == FieldKind::LongText && page.texts.get(&f.id).is_some_and(|t| !t.trim().is_empty()))
            .map(|f| f.id)
            .collect();
        Ok(page)
    }

    /// The fields for this paper: the library's, plus those of every
    /// collection it is in (the context collection's first).
    fn load_fields(&mut self, lib: &Library) -> Result<()> {
        let mut fields = lib.fields(None)?;
        let mut collections: Vec<i64> = self.relevance.iter().map(|(c, _)| *c).collect();
        collections.sort_by_key(|c| Some(*c) != self.context);
        for c in collections {
            fields.extend(lib.fields(Some(c))?.into_iter().filter(|f| f.collection == Some(c)));
        }
        let notes = lib.notes(self.id)?;
        for f in &fields {
            let stored = notes.get(&f.id).cloned().unwrap_or_default();
            match f.kind {
                FieldKind::Quotes => {
                    self.quotes.entry(f.id).or_insert_with(|| parse_quotes(&stored));
                }
                FieldKind::Metrics => {
                    self.metrics.entry(f.id).or_insert_with(|| parse_metrics(&stored));
                }
                _ => {
                    self.texts.entry(f.id).or_insert(stored);
                }
            }
        }
        self.fields = fields;
        Ok(())
    }

    /// After a change elsewhere: reload the metadata, keep what is being typed.
    pub fn reload_meta(&mut self, lib: &Library) {
        let Ok(paper) = lib.paper(self.id) else { return };
        if !self.meta.dirty {
            self.meta = MetaForm::from_paper(&paper);
        } else {
            self.meta.key = paper.citekey.clone();
        }
        self.rating = paper.rating;
        self.paper = paper;
        if let Ok(memberships) = lib.memberships(self.id) {
            let old = std::mem::take(&mut self.relevance);
            self.relevance = memberships
                .into_iter()
                .map(|(c, stored)| {
                    let typed = old.iter().find(|(o, _)| *o == c).map(|(_, t)| t.clone());
                    let dirty = self.dirty.contains_key(&Dirty::Relevance(c));
                    (c, if dirty { typed.unwrap_or(stored) } else { stored })
                })
                .collect();
        }
        if let Ok(tags) = lib.tags_of(self.id) {
            self.tags = tags;
        }
        if let Ok(links) = lib.links_of(self.id) {
            self.links = links;
        }
        let _ = self.load_fields(lib);
    }

    pub fn has_unsaved(&self) -> bool {
        !self.dirty.is_empty()
    }

    fn touch(&mut self, key: Dirty, now: f64) {
        self.dirty.insert(key, now);
    }

    /// Writes notes changed before `quiet_since` seconds ago (all if None).
    pub fn flush(&mut self, lib: &mut Library, now: Option<f64>) -> Result<()> {
        let due: Vec<Dirty> = self
            .dirty
            .iter()
            .filter(|(_, at)| now.is_none_or(|n| n - **at >= 0.6))
            .map(|(k, _)| *k)
            .collect();
        for key in due {
            match key {
                Dirty::Field(fid) => {
                    let kind = self.fields.iter().find(|f| f.id == fid).map(|f| f.kind).unwrap_or(FieldKind::LongText);
                    let content = match kind {
                        FieldKind::Quotes => {
                            let q: Vec<Quote> = self.quotes.get(&fid).cloned().unwrap_or_default();
                            json_or_empty(&q.into_iter().filter(|q| !q.text.trim().is_empty()).collect::<Vec<_>>())
                        }
                        FieldKind::Metrics => {
                            let m: Vec<Metric> = self.metrics.get(&fid).cloned().unwrap_or_default();
                            json_or_empty(&m.into_iter().filter(|m| !m.name.trim().is_empty()).collect::<Vec<_>>())
                        }
                        _ => self.texts.get(&fid).cloned().unwrap_or_default(),
                    };
                    lib.set_note(self.id, fid, &content)?;
                }
                Dirty::Relevance(cid) => {
                    let text = self.relevance.iter().find(|(c, _)| *c == cid).map(|(_, t)| t.clone()).unwrap_or_default();
                    lib.set_relevance(cid, self.id, &text)?;
                }
            }
            self.dirty.remove(&key);
        }
        Ok(())
    }

    /// Fills the metadata form from a DOI lookup (not saved until Save).
    pub fn apply_lookup(&mut self, b: Biblio) {
        let current = std::mem::take(&mut self.meta.bib);
        let mut merged = b;
        // Keep what the lookup doesn't know about.
        merged.extra = current.extra.clone();
        merged.fill_from(&current);
        let paper = Paper { bib: merged, citekey: self.meta.key.clone(), ..Default::default() };
        let mut form = MetaForm::from_paper(&paper);
        form.dirty = true;
        self.meta = form;
        self.tab = PaperTab::Metadata;
    }
}

impl Session {
    pub(crate) fn paper_ui(&mut self, ui: &mut Ui) {
        let Some(mut page) = self.page.take() else {
            self.go(View::Home);
            return;
        };
        let mut actions = Vec::new();
        self.paper_header(ui, &mut page, &mut actions);
        ui.add_space(6.0);
        let tabs = [
            (PaperTab::Anatomy, format!("{} Anatomy", icons::NOTE_PENCIL)),
            (PaperTab::Metadata, format!("{} Metadata{}", icons::FILE_TEXT, if page.meta.dirty { " •" } else { "" })),
            (PaperTab::Links, format!("{} Links ({})", icons::GRAPH, page.links.len())),
            (PaperTab::Bibtex, format!("{} BibTeX", icons::BRACKETS_CURLY)),
        ];
        if let Some(tab) = theme::segmented(ui, page.tab, &tabs) {
            page.tab = tab;
        }
        ui.add_space(8.0);
        egui::ScrollArea::vertical().id_salt(("paper", page.id, page.tab as u8)).auto_shrink(false).show(ui, |ui| {
            match page.tab {
                PaperTab::Anatomy => self.anatomy_tab(ui, &mut page),
                PaperTab::Metadata => self.metadata_tab(ui, &mut page),
                PaperTab::Links => self.links_tab(ui, &mut page, &mut actions),
                PaperTab::Bibtex => self.bibtex_tab(ui, &page),
            }
            ui.add_space(30.0);
        });
        // Put the page back unless something replaced it meanwhile.
        if self.page.is_none() && self.view == View::Paper(page.id) {
            self.page = Some(page);
        } else {
            let r = page.flush(&mut self.lib, None);
            self.check(r);
        }
        for a in actions {
            match a {
                PageAction::Back => self.go_back(),
                PageAction::Open(id) => self.open_paper(id),
                PageAction::Collection(id) => self.go(View::List(Scope::Collection(id))),
            }
        }
    }

    fn paper_header(&mut self, ui: &mut Ui, page: &mut PaperPage, actions: &mut Vec<PageAction>) {
        let p = page.paper.clone();
        ui.horizontal(|ui| {
            let back_to = match self.back.last() {
                Some(View::List(s)) | Some(View::Matrix(s)) => self.scope_title(*s),
                Some(View::Paper(_)) => "previous paper".into(),
                Some(View::Settings) => "Settings".into(),
                _ => "Home".into(),
            };
            if ui.add(theme::ghost_button(format!("{} {back_to}", icons::ARROW_LEFT))).on_hover_text("Back (Esc)").clicked() {
                actions.push(PageAction::Back);
            }
            if p.deleted {
                theme::chip(ui, format!("{} In the Trash", icons::TRASH), theme::RED);
                if ui.button("Restore").clicked() {
                    let r = self.lib.restore(&[p.id]);
                    self.check(r);
                }
            } else if ui.add(theme::ghost_button(icons::TRASH)).on_hover_text("Move to Trash").clicked() {
                let r = self.lib.trash(&[p.id]);
                if self.check(r).is_some() {
                    self.info("Moved to the Trash. It can be restored from there.");
                    actions.push(PageAction::Back);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.pdf_buttons(ui, &p));
        });
        ui.add_space(4.0);
        ui.add(egui::Label::new(theme::serif(&p.bib.title, 27.0)).wrap());
        let authors = authors_long(&p.bib.authors);
        if !authors.is_empty() {
            ui.add(egui::Label::new(RichText::new(authors).color(theme::t(ui).ink)).wrap());
        }
        let mut venue = Vec::new();
        if !p.bib.container.is_empty() {
            venue.push(p.bib.container.clone());
        }
        if let Some(y) = p.bib.year {
            venue.push(match p.bib.month {
                Some(m) if (1..=12).contains(&m) => format!("{} {y}", &MONTHS[m as usize - 1][..3]),
                _ => y.to_string(),
            });
        }
        if !p.bib.volume.is_empty() {
            let issue = if p.bib.issue.is_empty() { String::new() } else { format!("({})", p.bib.issue) };
            venue.push(format!("vol. {}{issue}", p.bib.volume));
        }
        if !p.bib.pages.is_empty() {
            venue.push(format!("pp. {}", p.bib.pages.replace('-', "–")));
        }
        if !venue.is_empty() {
            ui.add(egui::Label::new(RichText::new(venue.join(" · ")).color(theme::muted(ui))).wrap());
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            theme::chip(ui, entry_type_label(p.bib.entry_type_or_default()), theme::SLATE);
            let pin = if p.key_pinned { format!(" {}", icons::PUSH_PIN) } else { String::new() };
            if theme::chip_button(ui, format!("{} {}{pin}", icons::KEY, p.citekey), theme::accent(ui))
                .on_hover_text(if p.key_pinned {
                    "Citation key (pinned: it won't change). Click to copy \\cite{…}."
                } else {
                    "Citation key (follows the metadata until exported). Click to copy \\cite{…}."
                })
                .clicked()
            {
                ui.ctx().copy_text(format!("\\cite{{{}}}", p.citekey));
                self.info(format!("Copied \\cite{{{}}}.", p.citekey));
            }
            if let Some(d) = &p.bib.doi {
                ui.hyperlink_to(format!("{} {d}", icons::GLOBE), doi::url(d));
            } else if !p.bib.url.is_empty() {
                ui.hyperlink_to(format!("{} Web page", icons::GLOBE), &p.bib.url);
            }
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let options: Vec<(ReadingStatus, String)> = ReadingStatus::ALL
                .iter()
                .map(|s| (*s, format!("{} {}", theme::status_style(*s).0, s.label())))
                .collect();
            if let Some(s) = theme::segmented(ui, p.status, &options) {
                let r = self.lib.set_status(p.id, s);
                self.check(r);
            }
            ui.add_space(10.0);
            self.membership_chips(ui, page, actions);
            ui.add_space(6.0);
            self.tag_chips(ui, page);
        });
    }

    fn pdf_buttons(&mut self, ui: &mut Ui, p: &Paper) {
        match &p.pdf_path {
            Some(stored) => {
                let path = self.pdf_path(stored);
                let missing = !path.exists();
                let menu = ui.button(icons::CARET_DOWN).on_hover_text("More PDF actions");
                let stored = stored.clone();
                egui::Popup::menu(&menu).id(egui::Id::unique(("pdf-menu", p.id))).show(|ui| {
                    if ui.button(format!("{} Show in folder", icons::FOLDER_OPEN)).clicked() {
                        let r = crate::pdf::reveal(&path);
                        self.check(r);
                    }
                    if ui.button(format!("{} Link a different file…", icons::LINK)).clicked() {
                        self.pick_pdf(p.id);
                    }
                    if ui.button(format!("{} Unlink the PDF", icons::LINK_BREAK)).clicked() {
                        let r = self.lib.set_pdf(p.id, None);
                        self.check(r);
                    }
                });
                if missing {
                    if ui.add(theme::primary_button(ui, format!("{} Relink…", icons::LINK))).clicked() {
                        self.pick_pdf(p.id);
                    }
                    theme::chip(ui, format!("{} PDF not found", icons::WARNING), theme::AMBER)
                        .on_hover_text(format!("{} is missing. Relink it, or set the PDF folder in Settings.", path.display()));
                } else if ui
                    .add(theme::primary_button(ui, format!("{} Open PDF", icons::FILE_PDF)))
                    .on_hover_text(path.display().to_string())
                    .clicked()
                {
                    self.open_pdf(&stored);
                }
            }
            None => {
                if ui.button(format!("{} Attach PDF…", icons::FILE_PDF)).on_hover_text("Or drop the PDF onto this window").clicked() {
                    self.pick_pdf(p.id);
                }
            }
        }
    }

    fn pick_pdf(&mut self, id: i64) {
        let mut dialog = rfd::FileDialog::new().set_title("Link a PDF").add_filter("PDF", &["pdf"]);
        if let Some(folder) = &self.settings.pdf_folder {
            dialog = dialog.set_directory(folder);
        }
        if let Some(path) = dialog.pick_file() {
            self.attach_pdf(id, &path);
        }
    }

    fn membership_chips(&mut self, ui: &mut Ui, page: &mut PaperPage, actions: &mut Vec<PageAction>) {
        let collections = self.cache.collections.clone();
        for (cid, _) in page.relevance.clone() {
            let Some(c) = collections.iter().find(|c| c.id == cid) else { continue };
            let resp = theme::chip_button(ui, format!("● {}", c.name), theme::label_colour(c.colour))
                .on_hover_text("Open the collection. Right-click to remove the paper from it.");
            if resp.clicked() {
                actions.push(PageAction::Collection(cid));
            }
            resp.context_menu(|ui| {
                if ui.button(format!("{} Remove from {}", icons::X, c.name)).clicked() {
                    let r = self.lib.remove_from_collection(cid, &[page.id]);
                    self.check(r);
                }
            });
        }
        let add = ui.add(theme::ghost_button(format!("{} Collection", icons::PLUS))).on_hover_text("Add to a collection");
        egui::Popup::menu(&add).id(egui::Id::unique(("add-to", page.id))).show(|ui| {
            let mut any = false;
            for c in collections.iter().filter(|c| !page.relevance.iter().any(|(m, _)| *m == c.id)) {
                any = true;
                if ui.button(RichText::new(format!("●  {}", c.name)).color(theme::fg(ui, theme::label_colour(c.colour)))).clicked() {
                    let r = self.lib.add_to_collection(c.id, &[page.id]);
                    self.check(r);
                }
            }
            if !any {
                ui.label(RichText::new("No other collections").color(theme::muted(ui)));
            }
        });
    }

    fn tag_chips(&mut self, ui: &mut Ui, page: &mut PaperPage) {
        let mut remove = None;
        for t in &page.tags {
            if theme::chip_button(ui, format!("{} {t}", icons::TAG), theme::SLATE).on_hover_text("Click to remove").clicked() {
                remove = Some(t.clone());
            }
        }
        let resp = ui.add(egui::TextEdit::singleline(&mut page.tag_input).hint_text("+ tag").desired_width(80.0));
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut tags = page.tags.clone();
        let mut changed = false;
        if enter && !page.tag_input.trim().is_empty() {
            tags.extend(page.tag_input.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
            page.tag_input.clear();
            changed = true;
        }
        if let Some(t) = remove {
            tags.retain(|x| *x != t);
            changed = true;
        }
        if changed {
            let r = self.lib.set_tags(page.id, &tags);
            self.check(r);
        }
    }

    // ----- Anatomy ---------------------------------------------------------

    fn anatomy_tab(&mut self, ui: &mut Ui, page: &mut PaperPage) {
        let visible: Vec<Field> = page.fields.iter().filter(|f| !f.hidden).cloned().collect();
        let filled = visible
            .iter()
            .filter(|f| f.collection.is_none() && f.kind != FieldKind::Relevance)
            .filter(|f| match f.kind {
                FieldKind::Quotes => page.quotes.get(&f.id).is_some_and(|q| q.iter().any(|q| !q.text.trim().is_empty())),
                FieldKind::Metrics => page.metrics.get(&f.id).is_some_and(|m| m.iter().any(|m| !m.name.trim().is_empty())),
                _ => page.texts.get(&f.id).is_some_and(|t| !t.trim().is_empty()),
            })
            .count();
        let total = visible.iter().filter(|f| f.collection.is_none() && f.kind != FieldKind::Relevance).count();
        ui.horizontal(|ui| {
            theme::progress(ui, if total == 0 { 0.0 } else { filled as f32 / total as f32 }, 120.0, theme::accent(ui));
            ui.label(RichText::new(format!("{filled} of {total} parts written")).small().color(theme::muted(ui)));
            if page.has_unsaved() {
                ui.label(RichText::new(format!("{} saving…", icons::CLOCK)).small().color(theme::muted(ui)));
            } else {
                ui.label(RichText::new(format!("{} saved", icons::CHECK)).small().color(theme::muted(ui)));
            }
        });
        ui.add_space(6.0);
        let two_columns = ui.available_width() > 1050.0;
        if two_columns {
            let (left, right): (Vec<_>, Vec<_>) = visible.iter().enumerate().partition(|(i, _)| i % 2 == 0);
            ui.columns(2, |cols| {
                for (_, f) in left {
                    self.field_card(&mut cols[0], page, f);
                    cols[0].add_space(10.0);
                }
                for (_, f) in right {
                    self.field_card(&mut cols[1], page, f);
                    cols[1].add_space(10.0);
                }
            });
        } else {
            for f in &visible {
                self.field_card(ui, page, f);
                ui.add_space(10.0);
            }
        }
        if !page.abstract_shown() {
            return;
        }
        ui.add_space(4.0);
        theme::card(ui, "Abstract", |ui| {
            ui.add(egui::Label::new(RichText::new(&page.paper.bib.abstract_).color(theme::muted(ui))).wrap());
        });
    }

    fn field_card(&mut self, ui: &mut Ui, page: &mut PaperPage, f: &Field) {
        let now = self.now;
        theme::card_frame(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(theme::semibold(&f.label).size(15.0));
                ui.label(RichText::new(icons::INFO).color(theme::muted(ui))).on_hover_text(&f.prompt);
                if let Some(c) = f.collection.and_then(|c| self.collection(c)) {
                    theme::chip(ui, format!("only in {}", c.name), theme::label_colour(c.colour));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if f.kind == FieldKind::LongText {
                        let previewing = page.preview.contains(&f.id);
                        let (icon, tip) = if previewing { (icons::PENCIL_SIMPLE, "Edit") } else { (icons::EYE, "Show formatted (Markdown)") };
                        if ui.add(theme::ghost_button(icon)).on_hover_text(tip).clicked() {
                            if previewing {
                                page.preview.remove(&f.id);
                            } else {
                                page.preview.insert(f.id);
                            }
                        }
                    }
                    if f.key == "assessment" {
                        let mut rating = page.rating;
                        if theme::star_rating(ui, &mut rating, 16.0) {
                            page.rating = rating;
                            let r = self.lib.set_rating(page.id, rating);
                            self.check(r);
                        }
                    }
                });
            });
            match f.kind {
                FieldKind::LongText => {
                    let text = page.texts.entry(f.id).or_default();
                    if page.preview.contains(&f.id) && !text.trim().is_empty() {
                        let resp = ui
                            .scope(|ui| {
                                CommonMarkViewer::new().show(ui, &mut self.md_cache, text);
                            })
                            .response
                            .interact(egui::Sense::click())
                            .on_hover_text("Double-click to edit");
                        if resp.double_clicked() {
                            page.preview.remove(&f.id);
                        }
                    } else {
                        let resp = ui.add(
                            egui::TextEdit::multiline(text)
                                .hint_text(&f.prompt)
                                .desired_rows(3)
                                .desired_width(f32::INFINITY)
                                .lock_focus(true),
                        );
                        if resp.changed() {
                            page.touch(Dirty::Field(f.id), now);
                        }
                    }
                }
                FieldKind::ShortText => {
                    let text = page.texts.entry(f.id).or_default();
                    if ui.add(egui::TextEdit::singleline(text).hint_text(&f.prompt).desired_width(f32::INFINITY)).changed() {
                        page.touch(Dirty::Field(f.id), now);
                    }
                }
                FieldKind::Metrics => {
                    if metrics_editor(ui, page.metrics.entry(f.id).or_default(), f.id) {
                        page.touch(Dirty::Field(f.id), now);
                    }
                }
                FieldKind::Quotes => {
                    let mut copied = None;
                    if quotes_editor(ui, page.quotes.entry(f.id).or_default(), f.id, &mut copied) {
                        page.touch(Dirty::Field(f.id), now);
                    }
                    if let Some(text) = copied {
                        ui.ctx().copy_text(text);
                        self.info("Quote copied.");
                    }
                }
                FieldKind::Relevance => self.relevance_editor(ui, page, f, now),
            }
        });
    }

    fn relevance_editor(&mut self, ui: &mut Ui, page: &mut PaperPage, f: &Field, now: f64) {
        if page.relevance.is_empty() {
            theme::hint(ui, "Add this paper to a collection (above) to note why it matters for that piece of work.");
            return;
        }
        let mut order: Vec<usize> = (0..page.relevance.len()).collect();
        order.sort_by_key(|i| Some(page.relevance[*i].0) != page.context);
        for i in order {
            let cid = page.relevance[i].0;
            let Some(c) = self.collection(cid).cloned() else { continue };
            ui.horizontal(|ui| {
                theme::dot(ui, theme::label_colour(c.colour), 10.0);
                ui.label(RichText::new(&c.name).small().color(theme::muted(ui)));
            });
            let resp = ui.add(
                egui::TextEdit::multiline(&mut page.relevance[i].1)
                    .hint_text(&f.prompt)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            if resp.changed() {
                page.touch(Dirty::Relevance(cid), now);
            }
        }
    }

    // ----- Metadata --------------------------------------------------------

    fn metadata_tab(&mut self, ui: &mut Ui, page: &mut PaperPage) {
        let m = &mut page.meta;
        let mut changed = false;
        theme::card(ui, "Bibliographic details", |ui| {
            let wide = (ui.available_width() - 190.0).clamp(300.0, 760.0);
            egui::Grid::new("meta-grid").num_columns(2).spacing([14.0, 10.0]).min_col_width(150.0).show(ui, |ui| {
                ui.label("Type");
                let current = entry_type_label(m.bib.entry_type_or_default()).to_string();
                egui::ComboBox::from_id_salt("entry-type").selected_text(current).show_ui(ui, |ui| {
                    for (k, label) in ENTRY_TYPES {
                        if ui.selectable_label(m.bib.entry_type == *k, *label).clicked() {
                            m.bib.entry_type = k.to_string();
                            changed = true;
                        }
                    }
                });
                ui.end_row();
                ui.label("Title");
                changed |= ui.add(egui::TextEdit::multiline(&mut m.bib.title).desired_rows(2).desired_width(wide)).changed();
                ui.end_row();
                ui.label("Authors").on_hover_text("One per line, as Family, Given");
                changed |= ui
                    .add(egui::TextEdit::multiline(&mut m.authors).hint_text("Zhang, Wei\nLi, Na").desired_rows(3).desired_width(wide))
                    .changed();
                ui.end_row();
                ui.label("Year / month");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::TextEdit::singleline(&mut m.year).desired_width(70.0).hint_text("2025")).changed();
                    let month = m.bib.month.filter(|x| (1..=12).contains(x)).map(|x| MONTHS[x as usize - 1]).unwrap_or("—");
                    egui::ComboBox::from_id_salt("month").selected_text(month).show_ui(ui, |ui| {
                        if ui.selectable_label(m.bib.month.is_none(), "—").clicked() {
                            m.bib.month = None;
                            changed = true;
                        }
                        for (i, name) in MONTHS.iter().enumerate() {
                            if ui.selectable_label(m.bib.month == Some(i as u8 + 1), *name).clicked() {
                                m.bib.month = Some(i as u8 + 1);
                                changed = true;
                            }
                        }
                    });
                });
                ui.end_row();
                let container_label = match m.bib.entry_type.as_str() {
                    "inproceedings" => "Proceedings",
                    "incollection" => "Book",
                    "phdthesis" | "mastersthesis" => "University",
                    "techreport" => "Institution",
                    _ => "Journal",
                };
                ui.label(container_label);
                changed |= ui.add(egui::TextEdit::singleline(&mut m.bib.container).desired_width(wide)).changed();
                ui.end_row();
                ui.label("Volume / issue / pages");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::TextEdit::singleline(&mut m.bib.volume).hint_text("vol.").desired_width(70.0)).changed();
                    changed |= ui.add(egui::TextEdit::singleline(&mut m.bib.issue).hint_text("issue").desired_width(70.0)).changed();
                    changed |= ui.add(egui::TextEdit::singleline(&mut m.bib.pages).hint_text("12-19").desired_width(110.0)).changed();
                });
                ui.end_row();
                ui.label("Publisher");
                changed |= ui.add(egui::TextEdit::singleline(&mut m.bib.publisher).desired_width(wide)).changed();
                ui.end_row();
                ui.label("DOI");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::TextEdit::singleline(&mut m.doi).hint_text("10.xxxx/…").desired_width(260.0)).changed();
                    let busy = self.lookups.values().any(|l| *l == Lookup::Refresh(page.id));
                    let can = doi::normalise(&m.doi).is_some() && !busy;
                    let label = if busy { format!("{} Looking up…", icons::CLOCK) } else { format!("{} Fill in from DOI", icons::MAGIC_WAND) };
                    if ui.add_enabled(can, egui::Button::new(label)).on_hover_text("Replace these details with the publisher's record").clicked()
                        && let Some(d) = doi::normalise(&m.doi)
                    {
                        let batch = self.net.fetch(ui.ctx(), vec![d], self.settings.contact_email.clone());
                        self.lookups.insert(batch, Lookup::Refresh(page.id));
                    }
                });
                ui.end_row();
                ui.label("URL");
                changed |= ui.add(egui::TextEdit::singleline(&mut m.bib.url).desired_width(wide)).changed();
                ui.end_row();
                ui.label("ISBN / arXiv");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::TextEdit::singleline(&mut m.bib.isbn).hint_text("ISBN").desired_width(160.0)).changed();
                    changed |= ui.add(egui::TextEdit::singleline(&mut m.bib.arxiv_id).hint_text("arXiv id").desired_width(160.0)).changed();
                });
                ui.end_row();
                ui.label("Keywords");
                changed |= ui.add(egui::TextEdit::singleline(&mut m.bib.keywords).desired_width(wide)).changed();
                ui.end_row();
                ui.label("Abstract");
                changed |= ui.add(egui::TextEdit::multiline(&mut m.bib.abstract_).desired_rows(4).desired_width(wide)).changed();
                ui.end_row();
            });
            if !m.bib.extra.is_empty() {
                ui.add_space(6.0);
                ui.collapsing(format!("Other fields from the import ({})", m.bib.extra.len()), |ui| {
                    for (k, v) in &m.bib.extra {
                        ui.label(RichText::new(format!("{k} = {{{v}}}")).monospace().small());
                    }
                });
            }
        });
        if changed {
            m.dirty = true;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let dirty = page.meta.dirty;
            if ui.add_enabled(dirty, theme::primary_button(ui, format!("{} Save changes", icons::CHECK))).clicked() {
                self.save_meta(page);
            }
            if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                page.meta = MetaForm::from_paper(&page.paper);
            }
        });
        ui.add_space(14.0);
        self.citekey_card(ui, page);
    }

    fn save_meta(&mut self, page: &mut PaperPage) {
        match page.meta.to_biblio() {
            Ok(b) => {
                let r = self.lib.update_biblio(page.id, &b);
                if self.check(r).is_none() {
                    return;
                }
                // An unpinned key follows the metadata.
                let r = self.lib.regenerate_citekey(page.id, &self.settings.citekey_pattern, false);
                self.check(r);
                page.meta.dirty = false;
                page.reload_meta(&self.lib);
                self.info("Saved.");
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    fn citekey_card(&mut self, ui: &mut Ui, page: &mut PaperPage) {
        theme::card(ui, "Citation key", |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut page.meta.key).font(egui::TextStyle::Monospace).desired_width(260.0));
                let changed = page.meta.key.trim() != page.paper.citekey;
                if ui.add_enabled(changed, egui::Button::new(format!("{} Use this key", icons::CHECK))).clicked() {
                    let r = self.lib.set_citekey(page.id, &page.meta.key);
                    if self.check(r).is_some() {
                        self.info("Key changed and pinned.");
                    }
                }
                if ui
                    .button(format!("{} Regenerate", icons::ARROWS_CLOCKWISE))
                    .on_hover_text("Make a new key from the pattern in Settings (unpins it)")
                    .clicked()
                {
                    let r = self.lib.regenerate_citekey(page.id, &self.settings.citekey_pattern, true);
                    if let Some(k) = self.check(r) {
                        page.meta.key = k;
                    }
                }
            });
            let state = if page.paper.key_pinned {
                format!("{} Pinned: this key has been exported or set by hand, so it never changes on its own.", icons::PUSH_PIN)
            } else {
                "Follows the title, author and year until you export it or set it by hand.".to_string()
            };
            theme::hint(ui, state);
        });
    }

    // ----- Links -----------------------------------------------------------

    fn links_tab(&mut self, ui: &mut Ui, page: &mut PaperPage, actions: &mut Vec<PageAction>) {
        theme::card(ui, "How this paper relates to others", |ui| {
            if page.links.is_empty() {
                theme::hint(ui, "No links yet. Note which papers this one builds on, extends, contradicts or compares against.");
            }
            let mut delete = None;
            for l in &page.links {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(l.label()).color(theme::muted(ui)));
                    if ui.link(theme::semibold(&l.other_title)).on_hover_text(&l.other_key).clicked() {
                        actions.push(PageAction::Open(l.other_id));
                    }
                    if !l.link.note.is_empty() {
                        ui.label(RichText::new(format!("— {}", l.link.note)).small().color(theme::muted(ui)));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(theme::ghost_button(icons::X)).on_hover_text("Remove link").clicked() {
                            delete = Some(l.link.id);
                        }
                    });
                });
            }
            if let Some(id) = delete {
                let r = self.lib.delete_link(id);
                self.check(r);
            }
        });
        ui.add_space(10.0);
        theme::card(ui, "Add a link", |ui| {
            let form = &mut page.link_form;
            ui.horizontal(|ui| {
                ui.label("This paper");
                let rel = form.relation.unwrap_or(Relation::BuildsOn);
                egui::ComboBox::from_id_salt("relation").selected_text(rel.label().to_lowercase()).show_ui(ui, |ui| {
                    for r in Relation::ALL {
                        if ui.selectable_label(rel == r, r.label().to_lowercase()).clicked() {
                            form.relation = Some(r);
                        }
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut form.query).hint_text("find a paper by title, author or key").desired_width(300.0));
            });
            let q = form.query.trim().to_lowercase();
            if !q.is_empty() {
                let matches: Vec<_> = self
                    .cache
                    .all
                    .iter()
                    .filter(|r| r.id != page.id)
                    .filter(|r| {
                        r.title.to_lowercase().contains(&q) || r.authors.to_lowercase().contains(&q) || r.citekey.to_lowercase().contains(&q)
                    })
                    .take(8)
                    .cloned()
                    .collect();
                if matches.is_empty() {
                    theme::hint(ui, "No paper matches.");
                }
                for r in matches {
                    let label = format!("{} — {} {}", r.title, r.authors, r.year.map(|y| format!("({y})")).unwrap_or_default());
                    if ui.selectable_label(form.target == Some(r.id), label).clicked() {
                        form.target = Some(r.id);
                    }
                }
            }
            let mut add = false;
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut form.note).hint_text("note (optional): e.g. uses their unit cell").desired_width(360.0));
                let can = form.target.is_some();
                add = ui.add_enabled(can, theme::primary_button(ui, format!("{} Add link", icons::LINK))).clicked();
            });
            if add && let Some(target) = form.target {
                let rel = form.relation.unwrap_or(Relation::BuildsOn);
                let r = self.lib.add_link(page.id, target, rel, &form.note);
                if self.check(r).is_some() {
                    page.link_form = LinkForm { relation: Some(rel), ..Default::default() };
                }
            }
        });
    }

    // ----- BibTeX ----------------------------------------------------------

    fn bibtex_tab(&mut self, ui: &mut Ui, page: &PaperPage) {
        let opts = self.settings.export_options();
        let text = bibtex::entry(&page.paper, opts);
        theme::card(ui, &format!("{} entry", if opts.dialect == bibtex::Dialect::Bibtex { "BibTeX" } else { "BibLaTeX" }), |ui| {
            ui.horizontal(|ui| {
                if ui.button(format!("{} Copy entry", icons::COPY)).clicked() {
                    ui.ctx().copy_text(text.clone());
                    self.info("BibTeX entry copied. Its key is now pinned.");
                    let r = self.lib.pin_citekeys(&[page.id]);
                    self.check(r);
                }
                if ui.button(format!("{} Copy \\cite{{{}}}", icons::QUOTES, page.paper.citekey)).clicked() {
                    ui.ctx().copy_text(format!("\\cite{{{}}}", page.paper.citekey));
                }
                theme::hint(ui, "The dialect is set in Settings.");
            });
            ui.add_space(6.0);
            let mut shown = text.as_str();
            ui.add(egui::TextEdit::multiline(&mut shown).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
        });
        let in_collections = page.relevance.len();
        if in_collections > 0 {
            ui.add_space(6.0);
            theme::hint(ui, format!("Also exported with {}.", plural(in_collections, "collection")));
        }
    }
}

impl PaperPage {
    fn abstract_shown(&self) -> bool {
        !self.paper.bib.abstract_.trim().is_empty()
    }
}

/// Name / value / unit rows. Returns true if anything changed.
fn metrics_editor(ui: &mut Ui, metrics: &mut Vec<Metric>, id: i64) -> bool {
    let mut changed = false;
    let mut remove = None;
    if !metrics.is_empty() {
        egui::Grid::new(("metrics", id)).num_columns(4).spacing([6.0, 6.0]).show(ui, |ui| {
            ui.label(RichText::new("Name").small().color(theme::muted(ui)));
            ui.label(RichText::new("Value").small().color(theme::muted(ui)));
            ui.label(RichText::new("Unit").small().color(theme::muted(ui)));
            ui.end_row();
            for (i, m) in metrics.iter_mut().enumerate() {
                changed |= ui.add(egui::TextEdit::singleline(&mut m.name).hint_text("Gain").desired_width(150.0)).changed();
                changed |= ui.add(egui::TextEdit::singleline(&mut m.value).hint_text("30.8").desired_width(110.0)).changed();
                changed |= ui.add(egui::TextEdit::singleline(&mut m.unit).hint_text("dB").desired_width(60.0)).changed();
                if ui.add(theme::ghost_button(icons::X)).on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    }
    if let Some(i) = remove {
        metrics.remove(i);
        changed = true;
    }
    if ui.add(theme::ghost_button(format!("{} Add metric", icons::PLUS))).clicked() {
        metrics.push(Metric::default());
    }
    changed
}

/// Verbatim quotes with pages. Returns true if anything changed.
fn quotes_editor(ui: &mut Ui, quotes: &mut Vec<Quote>, id: i64, copied: &mut Option<String>) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (i, q) in quotes.iter_mut().enumerate() {
        ui.push_id((id, i), |ui| {
            egui::Frame::new()
                .fill(theme::t(ui).raised)
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin { left: 12, right: 8, top: 8, bottom: 8 })
                .show(ui, |ui| {
                    let rect = ui.max_rect();
                    ui.painter().vline(rect.left() - 6.0, rect.y_range(), egui::Stroke::new(3.0, theme::accent(ui)));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icons::QUOTES).color(theme::accent(ui)));
                        ui.label(RichText::new("p.").small().color(theme::muted(ui)));
                        changed |= ui.add(egui::TextEdit::singleline(&mut q.page).hint_text("12").desired_width(50.0)).changed();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(theme::ghost_button(icons::X)).on_hover_text("Remove").clicked() {
                                remove = Some(i);
                            }
                            if ui.add(theme::ghost_button(icons::COPY)).on_hover_text("Copy with page").clicked() {
                                let page = if q.page.trim().is_empty() { String::new() } else { format!(" (p. {})", q.page.trim()) };
                                *copied = Some(format!("“{}”{page}", q.text.trim()));
                            }
                        });
                    });
                    changed |= ui
                        .add(egui::TextEdit::multiline(&mut q.text).hint_text("The exact words…").desired_rows(2).desired_width(f32::INFINITY))
                        .changed();
                });
        });
        ui.add_space(4.0);
    }
    if let Some(i) = remove {
        quotes.remove(i);
        changed = true;
    }
    if ui.add(theme::ghost_button(format!("{} Add quote", icons::PLUS))).clicked() {
        quotes.push(Quote::default());
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Author;

    #[test]
    fn meta_form_round_trip() {
        let p = Paper {
            citekey: "k".into(),
            bib: Biblio {
                title: "T".into(),
                authors: vec![Author::new("Zhang", "Wei"), Author::new("IEEE", "")],
                year: Some(2025),
                doi: Some("10.1109/x.1".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let form = MetaForm::from_paper(&p);
        assert_eq!(form.authors, "Zhang, Wei\nIEEE");
        let b = form.to_biblio().unwrap();
        assert_eq!(b.authors, p.bib.authors);
        assert_eq!(b.year, Some(2025));
        let mut bad = form.clone();
        bad.doi = "nope".into();
        assert!(bad.to_biblio().is_err());
        let mut bad = form.clone();
        bad.year = "20".into();
        assert!(bad.to_biblio().is_err());
        let mut bad = form;
        bad.bib.title = "  ".into();
        assert!(bad.to_biblio().is_err());
    }
}
