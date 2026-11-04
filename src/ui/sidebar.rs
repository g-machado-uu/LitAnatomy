//! The left sidebar: brand, add menu, library views, collections and tags.

use egui::{RichText, Ui};

use crate::app::{PaperDrag, Session, View};
use crate::db::Scope;
use crate::model::Collection;
use crate::settings::ThemeMode;
use crate::ui::dialogs::{CollectionForm, Dialog};
use crate::ui::{icons, theme};

enum CollectionAction {
    Open(i64),
    NewChild(i64),
    Edit(i64),
    Move(i64, bool),
    Archive(i64, bool),
    Delete(i64),
    Drop(i64, Vec<i64>),
}

impl Session {
    pub(crate) fn sidebar(&mut self, ui: &mut Ui) {
        let frame = egui::Frame::new()
            .fill(theme::t(ui).sidebar)
            .inner_margin(egui::Margin { left: 12, right: 12, top: 16, bottom: 12 });
        egui::Panel::left("nav").exact_size(250.0).frame(frame).show_separator_line(false).show(ui, |ui| {
            self.brand(ui);
            ui.add_space(12.0);
            self.add_menu(ui);
            ui.add_space(10.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                self.library_nav(ui);
                ui.add_space(12.0);
                self.collections_nav(ui);
                if !self.cache.tags.is_empty() {
                    ui.add_space(12.0);
                    self.tags_nav(ui);
                }
                ui.add_space(70.0);
            });
        });
    }

    fn brand(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, egui::CornerRadius::same(11), theme::t(ui).button);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icons::BOOK_OPEN_TEXT,
                egui::FontId::proportional(21.0),
                egui::Color32::WHITE,
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(theme::serif("LitAnatomy", 22.0));
                ui.label(RichText::new(theme::VERSION).small().color(theme::muted(ui)));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dark = ui.visuals().dark_mode;
                let (icon, tip) = if dark { (icons::SUN, "Light mode") } else { (icons::MOON, "Dark mode") };
                if ui.add(theme::ghost_button(icon)).on_hover_text(tip).clicked() {
                    self.settings.theme_mode = if dark { ThemeMode::Light } else { ThemeMode::Dark };
                    self.settings_form.theme_mode = self.settings.theme_mode;
                    self.save_settings(ui.ctx());
                }
            });
        });
    }

    fn add_menu(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let button = theme::primary_button(ui, format!("{}  Add papers", icons::PLUS)).min_size(egui::vec2(width, 36.0));
        let resp = ui.add(button);
        egui::Popup::menu(&resp).id(egui::Id::unique("add-menu")).show(|ui| {
            ui.set_min_width(220.0);
            if ui.button(format!("{}  By DOI…", icons::MAGIC_WAND)).clicked() {
                self.dialog = Some(Dialog::AddDoi { text: String::new(), target: self.current_collection() });
            }
            if ui.button(format!("{}  Import a file…", icons::DOWNLOAD_SIMPLE)).on_hover_text("BibTeX, BibLaTeX, RIS, RefWorks or CSL-JSON").clicked() {
                self.pick_import_file();
            }
            if ui
                .button(format!("{}  From Zotero…", icons::BOOKS))
                .on_hover_text("The papers whose PDFs Zotero has downloaded, with their details")
                .clicked()
            {
                self.import_zotero();
            }
            if ui.button(format!("{}  Blank entry", icons::FILE_PLUS)).clicked() {
                self.new_blank_paper();
            }
        });
    }

    fn library_nav(&mut self, ui: &mut Ui) {
        section_label(ui, "Library");
        let total = self.cache.all.len();
        let items = [
            (View::Home, icons::HOUSE, "Home", None),
            (View::List(Scope::All), icons::BOOKS, "All papers", Some(total)),
            (View::List(Scope::Unfiled), icons::TRAY, "Unfiled", Some(self.cache.unfiled)),
            (View::Settings, icons::GEAR_SIX, "Settings", None),
        ];
        for (view, icon, label, count) in items {
            let selected = self.selected_view() == view;
            if theme::nav_item(ui, icon, None, label, count, selected, 0.0).clicked() {
                self.go(view);
            }
        }
        let trash_selected = self.selected_view() == View::List(Scope::Trash);
        let trash_count = (self.cache.trash > 0).then_some(self.cache.trash);
        let resp = theme::nav_item(ui, icons::TRASH, None, "Trash", trash_count, trash_selected, 0.0);
        if resp.clicked() {
            self.go(View::List(Scope::Trash));
        }
        if let Some(PaperDrag(ids)) = resp.dnd_release_payload::<PaperDrag>().map(|p| (*p).clone()) {
            let r = self.lib.trash(&ids);
            if self.check(r).is_some() {
                self.list.selected.clear();
                self.info(format!("Moved {} to the Trash.", plural(ids.len(), "paper")));
            }
        }
    }

    /// The view highlighted in the sidebar (a paper highlights where it was opened from).
    fn selected_view(&self) -> View {
        match &self.view {
            View::Paper(_) => self.back.iter().rev().find(|v| !matches!(v, View::Paper(_))).cloned().unwrap_or(View::Home),
            View::Matrix(scope) => View::List(*scope),
            v => v.clone(),
        }
    }

    fn collections_nav(&mut self, ui: &mut Ui) {
        let mut action = None;
        ui.horizontal(|ui| {
            section_label(ui, "Collections");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(theme::ghost_button(icons::PLUS)).on_hover_text("New collection").clicked() {
                    self.dialog = Some(Dialog::NewCollection { name: String::new(), parent: None, colour: next_colour(&self.cache.collections) });
                }
            });
        });
        let all = self.cache.collections.clone();
        let active: Vec<&Collection> = all.iter().filter(|c| !c.archived).collect();
        if active.is_empty() {
            theme::hint(ui, "Group papers by project: a chapter, a paper you are writing, your viva…");
        }
        let selected = self.selected_view();
        self.collection_tree(ui, &all, None, 0, false, &selected, &mut action);
        let archived: Vec<&Collection> = all.iter().filter(|c| c.archived && !c.parent.is_some_and(|p| all.iter().any(|x| x.id == p && x.archived))).collect();
        if !archived.is_empty() {
            ui.add_space(4.0);
            let caret = if self.archived_open { icons::CARET_DOWN } else { icons::CARET_RIGHT };
            if ui
                .add(egui::Label::new(RichText::new(format!("{caret} Archived ({})", archived.len())).small().color(theme::muted(ui))).sense(egui::Sense::click()))
                .clicked()
            {
                self.archived_open = !self.archived_open;
            }
            if self.archived_open {
                for c in archived {
                    self.collection_item(ui, c, 0, &all, &selected, &mut action);
                }
            }
        }
        if let Some(a) = action {
            self.collection_action(a);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn collection_tree(
        &mut self,
        ui: &mut Ui,
        all: &[Collection],
        parent: Option<i64>,
        depth: usize,
        archived: bool,
        selected: &View,
        action: &mut Option<CollectionAction>,
    ) {
        for c in all.iter().filter(|c| c.parent == parent && c.archived == archived) {
            self.collection_item(ui, c, depth, all, selected, action);
            if depth < 6 {
                self.collection_tree(ui, all, Some(c.id), depth + 1, archived, selected, action);
            }
        }
    }

    fn collection_item(
        &mut self,
        ui: &mut Ui,
        c: &Collection,
        depth: usize,
        all: &[Collection],
        selected: &View,
        action: &mut Option<CollectionAction>,
    ) {
        let count = self.cache.stats.get(&c.id).map(|s| s.0).unwrap_or(0);
        let is_selected = *selected == View::List(Scope::Collection(c.id));
        let colour = theme::label_colour(c.colour);
        let resp = theme::nav_item(ui, "", Some(colour), &c.name, Some(count), is_selected, depth as f32 * 14.0);
        if let Some(payload) = resp.dnd_hover_payload::<PaperDrag>() {
            let _ = payload;
            ui.painter().rect_stroke(
                resp.rect,
                egui::CornerRadius::same(9),
                egui::Stroke::new(2.0, theme::accent(ui)),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(payload) = resp.dnd_release_payload::<PaperDrag>() {
            *action = Some(CollectionAction::Drop(c.id, payload.0.clone()));
        }
        if resp.clicked() {
            *action = Some(CollectionAction::Open(c.id));
        }
        let has_siblings = all.iter().filter(|x| x.parent == c.parent && x.archived == c.archived).count() > 1;
        resp.on_hover_text(if c.description.is_empty() { c.name.clone() } else { c.description.clone() })
            .context_menu(|ui| {
                if ui.button(format!("{} Edit…", icons::PENCIL_SIMPLE)).clicked() {
                    *action = Some(CollectionAction::Edit(c.id));
                }
                if ui.button(format!("{} New sub-collection", icons::FOLDER_SIMPLE_PLUS)).clicked() {
                    *action = Some(CollectionAction::NewChild(c.id));
                }
                if has_siblings {
                    if ui.button(format!("{} Move up", icons::CARET_UP)).clicked() {
                        *action = Some(CollectionAction::Move(c.id, true));
                    }
                    if ui.button(format!("{} Move down", icons::CARET_DOWN)).clicked() {
                        *action = Some(CollectionAction::Move(c.id, false));
                    }
                }
                let archive_label = if c.archived { "Restore from archive" } else { "Archive" };
                if ui.button(format!("{} {archive_label}", icons::ARCHIVE)).clicked() {
                    *action = Some(CollectionAction::Archive(c.id, !c.archived));
                }
                ui.separator();
                if ui.button(RichText::new(format!("{} Delete…", icons::TRASH)).color(theme::fg(ui, theme::RED))).clicked() {
                    *action = Some(CollectionAction::Delete(c.id));
                }
            });
    }

    fn collection_action(&mut self, action: CollectionAction) {
        match action {
            CollectionAction::Open(id) => self.go(View::List(Scope::Collection(id))),
            CollectionAction::NewChild(parent) => {
                self.dialog = Some(Dialog::NewCollection {
                    name: String::new(),
                    parent: Some(parent),
                    colour: self.collection(parent).map(|c| c.colour).unwrap_or(0),
                });
            }
            CollectionAction::Edit(id) => {
                if let Some(form) = CollectionForm::load(&self.lib, id) {
                    self.dialog = Some(Dialog::EditCollection(Box::new(form)));
                }
            }
            CollectionAction::Move(id, up) => {
                let r = self.lib.move_collection(id, up);
                self.check(r);
            }
            CollectionAction::Archive(id, archived) => {
                if let Some(mut c) = self.collection(id).cloned() {
                    c.archived = archived;
                    let r = self.lib.update_collection(&c);
                    self.check(r);
                }
            }
            CollectionAction::Delete(id) => {
                if let Some(c) = self.collection(id) {
                    self.dialog = Some(Dialog::ConfirmDeleteCollection { id, name: c.name.clone() });
                }
            }
            CollectionAction::Drop(id, papers) => {
                let r = self.lib.add_to_collection(id, &papers);
                if let Some(added) = self.check(r) {
                    let name = self.collection(id).map(|c| c.name.clone()).unwrap_or_default();
                    if added == 0 {
                        self.info(format!("Already in {name}."));
                    } else {
                        self.info(format!("Added {} to {name}.", plural(added, "paper")));
                    }
                }
            }
        }
    }

    fn tags_nav(&mut self, ui: &mut Ui) {
        section_label(ui, "Tags");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for (id, name, count) in self.cache.tags.clone() {
                if count == 0 {
                    continue;
                }
                let active = self.list.tag == Some(id) && self.view == View::List(Scope::All);
                let colour = if active { theme::accent(ui) } else { theme::SLATE };
                if theme::chip_button(ui, format!("{name} {count}"), colour).clicked() {
                    self.go(View::List(Scope::All));
                    self.list.tag = if active { None } else { Some(id) };
                }
            }
        });
    }
}

fn section_label(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(theme::semibold(text.to_uppercase()).size(11.0).extra_letter_spacing(0.8).color(theme::muted(ui)));
    ui.add_space(2.0);
}

/// The least used colour, so new collections look different.
pub fn next_colour(collections: &[Collection]) -> u8 {
    let mut counts = [0usize; theme::LABEL_COLOURS.len()];
    for c in collections {
        counts[c.colour as usize % counts.len()] += 1;
    }
    counts.iter().enumerate().min_by_key(|(i, n)| (**n, *i)).map(|(i, _)| i as u8).unwrap_or(0)
}

pub fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}
