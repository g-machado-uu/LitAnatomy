//! Home: progress per collection, where you left off, what needs attention.

use egui::{RichText, Ui};

use crate::app::{Session, View};
use crate::db::Scope;
use crate::model::ReadingStatus;
use crate::ui::dialogs::Dialog;
use crate::ui::list::Attention;
use crate::ui::sidebar::{next_colour, plural};
use crate::ui::{icons, theme};

impl Session {
    pub(crate) fn home_ui(&mut self, ui: &mut Ui) {
        let total = self.cache.all.len();
        let counts = self.status_counts();
        let summarised = counts.get(&ReadingStatus::Summarised).copied().unwrap_or(0);
        let subtitle = if total == 0 {
            "Start by adding a paper by its DOI, or import a .bib file.".to_string()
        } else {
            format!(
                "{} · {summarised} summarised · {}",
                plural(total, "paper"),
                plural(self.cache.collections.iter().filter(|c| !c.archived).count(), "collection")
            )
        };
        theme::page_header(ui, &greeting(), &subtitle, |_| {});
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            self.quick_add(ui);
            ui.add_space(14.0);
            self.stat_row(ui, total, &counts);
            ui.add_space(14.0);
            self.collection_cards(ui);
            ui.add_space(14.0);
            let wide = ui.available_width() > 900.0;
            if wide {
                ui.columns(2, |cols| {
                    self.recent_card(&mut cols[0]);
                    self.attention_card(&mut cols[1]);
                });
            } else {
                self.recent_card(ui);
                ui.add_space(14.0);
                self.attention_card(ui);
            }
            ui.add_space(20.0);
        });
    }

    /// Paste an identifier, or drop PDFs on the zone on the right.
    fn quick_add(&mut self, ui: &mut Ui) {
        let dragging = ui.input(|i| !i.raw.hovered_files.is_empty());
        theme::card(ui, "Add a paper", |ui| {
            let zone = egui::vec2((ui.available_width() * 0.32).clamp(220.0, 340.0), 92.0);
            ui.horizontal_top(|ui| {
                // Two gaps between the columns: the space and egui's item spacing.
                let gap = 16.0;
                let left = ui.available_width() - zone.x - gap - 2.0 * ui.spacing().item_spacing.x;
                ui.allocate_ui_with_layout(egui::vec2(left, zone.y), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_min_width(left);
                    ui.add_space(4.0);
                    // The buttons sit next to the box; on a narrow window they go below it.
                    // Look up, Import file… and Zotero… with the spacing between them.
                    let buttons = 345.0;
                    let one_row = left - buttons >= 260.0;
                    let mut go = false;
                    ui.horizontal(|ui| {
                        let width = if one_row { left - buttons } else { left };
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.quick_doi)
                                .hint_text("DOI, arXiv ID, ISBN or PubMed ID")
                                .desired_width(width),
                        );
                        go |= resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if one_row {
                            go |= self.quick_add_buttons(ui);
                        }
                    });
                    if !one_row {
                        ui.horizontal(|ui| go |= self.quick_add_buttons(ui));
                    }
                    if go {
                        let text = std::mem::take(&mut self.quick_doi);
                        if !self.lookup_ids(ui.ctx(), &text, None) {
                            self.quick_doi = text;
                        }
                    }
                    ui.add_space(2.0);
                    theme::hint(ui, "e.g. 10.1109/TAP.2025.3603900, arXiv:2301.12345 or ISBN 978-0-470-63155-3. Several can be pasted at once.");
                });
                ui.add_space(gap);
                let (title, subtitle) = if dragging {
                    ("Drop to add the papers", "PDFs, a folder of PDFs, or a .bib, .ris or .json file")
                } else {
                    ("Drop PDFs or a folder here", "or click to choose PDFs · Ctrl+V pastes copied files")
                };
                if theme::drop_zone(ui, zone, dragging, title, subtitle)
                    .on_hover_text("Each paper's details are found from its PDF, and the PDF stays linked")
                    .clicked()
                {
                    self.pick_pdfs(ui.ctx());
                }
            });
        });
    }

    /// Look up, Import file… and Zotero…; returns whether Look up was pressed.
    fn quick_add_buttons(&mut self, ui: &mut Ui) -> bool {
        let go = ui.add(theme::primary_button(ui, format!("{} Look up", icons::MAGIC_WAND))).clicked();
        if ui.button(format!("{} Import file…", icons::DOWNLOAD_SIMPLE)).on_hover_text("BibTeX, BibLaTeX, RIS, RefWorks or CSL-JSON").clicked() {
            self.pick_import_file();
        }
        if ui.button(format!("{} Zotero…", icons::BOOKS)).on_hover_text("Import the papers whose PDFs Zotero has downloaded").clicked() {
            self.import_zotero();
        }
        go
    }

    fn stat_row(&mut self, ui: &mut Ui, total: usize, counts: &std::collections::HashMap<ReadingStatus, usize>) {
        let stats = [
            (icons::BOOKS, theme::accent(ui), "Papers", total),
            (icons::CIRCLE_DASHED, theme::SLATE, "To read", counts.get(&ReadingStatus::ToRead).copied().unwrap_or(0)),
            (icons::CIRCLE_HALF, theme::AMBER, "Reading", counts.get(&ReadingStatus::Reading).copied().unwrap_or(0)),
            (icons::CHECK_CIRCLE, theme::GREEN, "Summarised", counts.get(&ReadingStatus::Summarised).copied().unwrap_or(0)),
        ];
        ui.columns(4, |cols| {
            for (col, (icon, colour, label, n)) in cols.iter_mut().zip(stats) {
                theme::card_frame(col).show(col, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        theme::icon_badge(ui, icon, colour, 38.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(theme::semibold(n.to_string()).size(22.0));
                            ui.label(RichText::new(label).small().color(theme::muted(ui)));
                        });
                    });
                });
            }
        });
    }

    fn collection_cards(&mut self, ui: &mut Ui) {
        let collections: Vec<_> = self.cache.collections.iter().filter(|c| !c.archived && c.parent.is_none()).cloned().collect();
        theme::card(ui, "Collections", |ui| {
            if collections.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("No collections yet. Make one for each piece of work: a chapter, a paper, your viva.").color(theme::muted(ui)));
                    if ui.button(format!("{} New collection", icons::FOLDER_SIMPLE_PLUS)).clicked() {
                        self.dialog = Some(Dialog::NewCollection { name: String::new(), parent: None, colour: next_colour(&self.cache.collections) });
                    }
                });
                return;
            }
            let card_w = 230.0;
            let per_row = ((ui.available_width() + 12.0) / (card_w + 12.0)).floor().max(1.0) as usize;
            for row in collections.chunks(per_row) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    for c in row {
                        let (total, done) = self.stats_with_children(c.id);
                        let colour = theme::label_colour(c.colour);
                        let resp = egui::Frame::new()
                            .fill(theme::soft(ui, colour))
                            .corner_radius(egui::CornerRadius::same(12))
                            .inner_margin(egui::Margin::same(14))
                            .show(ui, |ui| ui.vertical(|ui| {
                                // Not selectable: a click anywhere opens the collection.
                                ui.style_mut().interaction.selectable_labels = false;
                                ui.set_width(card_w - 28.0);
                                ui.horizontal(|ui| {
                                    theme::dot(ui, colour, 12.0);
                                    ui.add(egui::Label::new(theme::semibold(&c.name)).truncate());
                                });
                                let desc = if c.description.is_empty() { " ".to_string() } else { c.description.clone() };
                                ui.add(egui::Label::new(RichText::new(desc).small().color(theme::muted(ui))).truncate());
                                ui.add_space(4.0);
                                let fraction = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                                theme::progress(ui, fraction, card_w - 28.0, theme::fg(ui, colour));
                                ui.label(RichText::new(format!("{done} of {} summarised", plural(total, "paper"))).small().color(theme::muted(ui)));
                            }))
                            .response
                            .interact(egui::Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            self.go(View::List(Scope::Collection(c.id)));
                        }
                    }
                });
                ui.add_space(8.0);
            }
        });
    }

    /// (papers, summarised) counting sub-collections, without double counting.
    fn stats_with_children(&self, id: i64) -> (usize, usize) {
        let mut ids = vec![id];
        ids.extend(self.lib.descendants(id).unwrap_or_default());
        if ids.len() == 1 {
            return self.cache.stats.get(&id).copied().unwrap_or((0, 0));
        }
        let papers = self.lib.export_papers(id).unwrap_or_default();
        let done = papers.iter().filter(|p| p.status == ReadingStatus::Summarised).count();
        (papers.len(), done)
    }

    fn recent_card(&mut self, ui: &mut Ui) {
        let mut recent = self.cache.all.clone();
        recent.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        recent.truncate(6);
        theme::card(ui, "Continue where you left off", |ui| {
            if recent.is_empty() {
                ui.label(RichText::new("Papers you work on will show here.").color(theme::muted(ui)));
                return;
            }
            for r in recent {
                let resp = theme::clickable_row(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (icon, colour) = theme::status_style(r.status);
                        ui.label(RichText::new(icon).size(16.0).color(theme::fg(ui, colour)));
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add(egui::Label::new(theme::semibold(&r.title)).truncate());
                            let meta = [r.authors.clone(), r.year.map(|y| y.to_string()).unwrap_or_default()]
                                .into_iter()
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                                .join(" · ");
                            ui.add(egui::Label::new(RichText::new(meta).small().color(theme::muted(ui))).truncate());
                        });
                    });
                });
                if resp.clicked() {
                    self.open_paper(r.id);
                }
                ui.add_space(4.0);
            }
        });
    }

    fn attention_card(&mut self, ui: &mut Ui) {
        let all = self.cache.all.clone();
        let empty = all.iter().filter(|r| r.filled == 0).count();
        let no_pdf = all.iter().filter(|r| !r.has_pdf).count();
        let no_doi = all.iter().filter(|r| !r.has_doi).count();
        let reading = all.iter().filter(|r| r.status == ReadingStatus::Reading).count();
        let items = [
            (Attention::NotStarted, icons::NOTE_PENCIL, empty, "not dissected yet (no notes)"),
            (Attention::Reading, icons::CIRCLE_HALF, reading, "being read"),
            (Attention::NoPdf, icons::FILE_PDF, no_pdf, "without a linked PDF"),
            (Attention::NoDoi, icons::LINK_BREAK, no_doi, "without a DOI"),
        ];
        theme::card(ui, "Needs attention", |ui| {
            if all.is_empty() {
                ui.label(RichText::new("Nothing yet.").color(theme::muted(ui)));
                return;
            }
            for (filter, icon, n, text) in items {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon).size(16.0).color(theme::muted(ui)));
                    let label = format!("{} {text}", plural(n, "paper"));
                    if n == 0 {
                        ui.label(RichText::new(label).color(theme::muted(ui)));
                    } else if ui.link(label).clicked() {
                        self.go(View::List(Scope::All));
                        self.list.attention = Some(filter);
                    }
                });
            }
        });
    }
}

fn greeting() -> String {
    use chrono::Timelike;
    let hour = chrono::Local::now().hour();
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
    .to_string()
}
