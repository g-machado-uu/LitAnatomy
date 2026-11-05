//! Settings: appearance, citation keys, BibTeX export, PDFs, the library
//! file, online lookups and the anatomy fields.

use egui::{RichText, Ui};

use crate::app::Session;
use crate::citekey;
use crate::db::Scope;
use crate::export::bibtex::Dialect;
use crate::model::{Author, Biblio, Field, FieldKind};
use crate::settings::{Settings, ThemeMode};
use crate::ui::sidebar::plural;
use crate::ui::{icons, theme};

pub struct SettingsForm {
    pub theme_mode: ThemeMode,
    pub pattern: String,
    pub email: String,
    pub scale: f32,
    fields: Vec<Field>,
    fields_rev: u64,
    new_field: String,
    new_kind: FieldKind,
}

impl From<&Settings> for SettingsForm {
    fn from(s: &Settings) -> Self {
        Self {
            theme_mode: s.theme_mode,
            pattern: s.citekey_pattern.clone(),
            email: s.contact_email.clone(),
            scale: s.ui_scale,
            fields: Vec::new(),
            fields_rev: 0,
            new_field: String::new(),
            new_kind: FieldKind::LongText,
        }
    }
}

fn examples() -> [Biblio; 3] {
    [
        Biblio {
            title: "A Wideband 1-bit Reconfigurable Metasurface With Switchable Bidirectional Transmission".into(),
            authors: vec![Author::new("Zhang", "Wei"), Author::new("Li", "Na")],
            year: Some(2025),
            container: "IEEE Transactions on Antennas and Propagation".into(),
            ..Default::default()
        },
        Biblio {
            title: "On the Design of mm-Wave Electromagnetic Skins".into(),
            authors: vec![Author::new("van der Berg", "Anna")],
            year: Some(2019),
            container: "Proceedings of EuCAP".into(),
            ..Default::default()
        },
        Biblio {
            title: "Über Antennen für 6G".into(),
            authors: vec![Author::new("Müller", "Jörg")],
            year: Some(2024),
            ..Default::default()
        },
    ]
}

impl Session {
    pub(crate) fn settings_ui(&mut self, ui: &mut Ui) {
        theme::page_header(ui, "Settings", "Saved as you change them.", |_| {});
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(860.0);
            self.appearance_settings(ui);
            ui.add_space(12.0);
            self.key_settings(ui);
            ui.add_space(12.0);
            self.export_settings(ui);
            ui.add_space(12.0);
            self.field_settings(ui);
            ui.add_space(12.0);
            self.pdf_settings(ui);
            ui.add_space(12.0);
            self.library_settings(ui);
            ui.add_space(12.0);
            self.lookup_settings(ui);
            ui.add_space(12.0);
            theme::card(ui, "About", |ui| {
                ui.label(format!("LitAnatomy {}: dissect, summarise and cite the papers of your literature review.", theme::VERSION));
                ui.hyperlink_to(format!("{} github.com/g-machado-uu/LitAnatomy", icons::GLOBE), "https://github.com/g-machado-uu/LitAnatomy");
            });
            ui.add_space(30.0);
        });
    }

    fn appearance_settings(&mut self, ui: &mut Ui) {
        let mut changed = false;
        theme::card(ui, "Appearance", |ui| {
            let modes: Vec<(ThemeMode, String)> = ThemeMode::ALL.iter().map(|m| (*m, m.label().to_string())).collect();
            if let Some(m) = theme::segmented(ui, self.settings.theme_mode, &modes) {
                self.settings.theme_mode = m;
                self.settings_form.theme_mode = m;
                changed = true;
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Colour theme").color(theme::muted(ui)));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                let dark = ui.visuals().dark_mode;
                for p in theme::PALETTES {
                    let tones = if dark { &p.dark } else { &p.light };
                    let selected = self.settings.palette == p.name;
                    let resp = egui::Frame::new()
                        .fill(tones.bg)
                        .stroke(egui::Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { tones.accent } else { tones.line }))
                        .corner_radius(egui::CornerRadius::same(12))
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| ui.vertical(|ui| {
                            ui.set_width(118.0);
                            ui.horizontal(|ui| {
                                for c in [tones.button, tones.sidebar, tones.card] {
                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                                    ui.painter().circle(rect.center(), 8.0, c, egui::Stroke::new(1.0, tones.line));
                                }
                            });
                            ui.label(RichText::new(p.name).color(tones.ink).small());
                        }))
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if resp.clicked() && !selected {
                        self.settings.palette = p.name.to_string();
                        changed = true;
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Text size");
                let resp = ui.add(egui::Slider::new(&mut self.settings_form.scale, 0.8..=1.5).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                if (resp.drag_stopped() || (resp.changed() && !resp.dragged())) && (self.settings.ui_scale - self.settings_form.scale).abs() > 0.001 {
                    self.settings.ui_scale = self.settings_form.scale;
                    changed = true;
                }
            });
        });
        if changed {
            self.save_settings(ui.ctx());
        }
    }

    fn key_settings(&mut self, ui: &mut Ui) {
        let mut changed = false;
        theme::card(ui, "Citation keys", |ui| {
            ui.horizontal(|ui| {
                ui.label("Pattern");
                ui.add(egui::TextEdit::singleline(&mut self.settings_form.pattern).font(egui::TextStyle::Monospace).desired_width(300.0));
                let valid = citekey::validate(&self.settings_form.pattern);
                let differs = self.settings_form.pattern != self.settings.citekey_pattern;
                if ui.add_enabled(valid.is_ok() && differs, egui::Button::new(format!("{} Use this pattern", icons::CHECK))).clicked() {
                    self.settings.citekey_pattern = self.settings_form.pattern.clone();
                    changed = true;
                }
                if self.settings_form.pattern != citekey::DEFAULT_PATTERN && ui.add(theme::ghost_button("Default")).clicked() {
                    self.settings_form.pattern = citekey::DEFAULT_PATTERN.into();
                }
                if let Err(e) = valid {
                    ui.label(RichText::new(e).small().color(theme::fg(ui, theme::RED)));
                }
            });
            ui.add_space(4.0);
            egui::Grid::new("key-examples").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                for b in examples() {
                    ui.label(RichText::new(format!("{} ({})", crate::model::authors_short(&b.authors), b.year.unwrap_or_default())).small().color(theme::muted(ui)));
                    ui.label(RichText::new(citekey::generate(&self.settings_form.pattern, &b)).monospace());
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            theme::hint(
                ui,
                "Parts: [auth] first author, [auth2] first two, [year], [shortyear], [title:N] first N significant title words, [venue:N]. \
                 Add :lower or :upper to change case, e.g. [auth:lower][year][title:1]. Clashes get a, b, c… at the end.",
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut self.settings.keep_readable_keys, "When importing, keep keys that are already readable").changed() {
                    changed = true;
                }
            });
            theme::hint(ui, "Numeric or database keys, such as IEEE Xplore's 10234847, are always replaced.");
            ui.add_space(6.0);
            if ui
                .button(format!("{} Regenerate every unpinned key", icons::ARROWS_CLOCKWISE))
                .on_hover_text("Keys that were exported or set by hand are pinned and keep their key.")
                .clicked()
            {
                let ids: Vec<i64> = self.lib.rows(Scope::All).unwrap_or_default().iter().map(|r| r.id).collect();
                let mut n = 0;
                for id in ids {
                    if let Ok(p) = self.lib.paper(id)
                        && !p.key_pinned
                    {
                        let r = self.lib.regenerate_citekey(id, &self.settings.citekey_pattern, false);
                        if let Some(k) = self.check(r)
                            && k != p.citekey
                        {
                            n += 1;
                        }
                    }
                }
                self.info(format!("{} changed.", plural(n, "key")));
            }
        });
        if changed {
            self.save_settings(ui.ctx());
        }
    }

    fn export_settings(&mut self, ui: &mut Ui) {
        let mut changed = false;
        theme::card(ui, "BibTeX export", |ui| {
            let dialects = [(Dialect::Bibtex, Dialect::Bibtex.label().to_string()), (Dialect::Biblatex, Dialect::Biblatex.label().to_string())];
            if let Some(d) = theme::segmented(ui, self.settings.bib_dialect, &dialects) {
                self.settings.bib_dialect = d;
                changed = true;
            }
            changed |= ui.checkbox(&mut self.settings.export_abstracts, "Include abstracts").changed();
            ui.add_space(8.0);
            changed |= ui
                .checkbox(&mut self.settings.auto_export, theme::semibold("Keep .bib files up to date automatically"))
                .changed();
            theme::hint(
                ui,
                "Each collection can have its own .bib file (e.g. in your thesis folder). When on, it is rewritten a moment after any change \
                 to the collection, its sub-collections or their papers. Exported keys are pinned so your document keeps compiling.",
            );
            if self.settings.auto_export {
                ui.add_space(6.0);
                let collections = self.cache.collections.clone();
                egui::Grid::new("export-paths").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                    for c in collections {
                        ui.horizontal(|ui| {
                            theme::dot(ui, theme::label_colour(c.colour), 10.0);
                            ui.label(&c.name);
                        });
                        let path = c.bib_export_path.clone().unwrap_or_default();
                        ui.add(egui::Label::new(RichText::new(if path.is_empty() { "not exported".into() } else { path.clone() }).small().color(theme::muted(ui))).truncate());
                        ui.horizontal(|ui| {
                            if ui.button(format!("{} Choose…", icons::FOLDER_OPEN)).clicked()
                                && let Some(p) = rfd::FileDialog::new()
                                    .set_file_name(format!("{}.bib", crate::app::safe_file_name(&c.name)))
                                    .add_filter("BibTeX", &["bib"])
                                    .save_file()
                            {
                                let mut c2 = c.clone();
                                c2.bib_export_path = Some(p.display().to_string());
                                let r = self.lib.update_collection(&c2);
                                self.check(r);
                            }
                            if !path.is_empty() && ui.add(theme::ghost_button(icons::X)).on_hover_text("Stop exporting").clicked() {
                                let mut c2 = c.clone();
                                c2.bib_export_path = None;
                                let r = self.lib.update_collection(&c2);
                                self.check(r);
                            }
                        });
                        ui.end_row();
                    }
                });
                if self.cache.collections.is_empty() {
                    theme::hint(ui, "Make a collection first.");
                }
                ui.add_space(4.0);
                if ui.button(format!("{} Export now", icons::EXPORT)).clicked() {
                    let r = self.run_auto_export();
                    if let Some(n) = self.check(r) {
                        self.info(format!("{} updated.", plural(n, "file")));
                    }
                    self.auto.done_rev = self.lib.revision();
                }
                if let Some(e) = self.auto.last_error.clone() {
                    ui.label(RichText::new(e).small().color(theme::fg(ui, theme::RED)));
                }
            }
        });
        if changed {
            self.save_settings(ui.ctx());
        }
    }

    fn field_settings(&mut self, ui: &mut Ui) {
        let rev = self.lib.revision();
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if self.settings_form.fields_rev != rev && !typing {
            self.settings_form.fields = self.lib.fields(None).unwrap_or_default();
            self.settings_form.fields_rev = rev;
        }
        theme::card(ui, "Anatomy fields", |ui| {
            theme::hint(ui, "The parts every paper is dissected into. Rename them, change their prompts, reorder or hide them. Fields for one collection only are set in that collection's settings.");
            ui.add_space(6.0);
            let mut update = None;
            let mut moved = None;
            let mut delete = None;
            for f in self.settings_form.fields.iter_mut() {
                ui.horizontal(|ui| {
                    let eye = if f.hidden { icons::EYE } else { icons::CHECK_CIRCLE };
                    let tip = if f.hidden { "Hidden: click to show" } else { "Shown: click to hide" };
                    let colour = if f.hidden { theme::muted(ui) } else { theme::accent(ui) };
                    if ui.add(theme::ghost_button(RichText::new(eye).color(colour).text())).on_hover_text(tip).clicked() {
                        f.hidden = !f.hidden;
                        update = Some(f.clone());
                    }
                    let label = ui.add(egui::TextEdit::singleline(&mut f.label).desired_width(220.0));
                    let prompt = ui.add(egui::TextEdit::singleline(&mut f.prompt).desired_width(320.0).hint_text("prompt"));
                    if (label.lost_focus() || prompt.lost_focus()) && !f.label.trim().is_empty() {
                        update = Some(f.clone());
                    }
                    ui.label(RichText::new(f.kind.label()).small().color(theme::muted(ui)));
                    if ui.add(theme::ghost_button(icons::CARET_UP)).clicked() {
                        moved = Some((f.id, true));
                    }
                    if ui.add(theme::ghost_button(icons::CARET_DOWN)).clicked() {
                        moved = Some((f.id, false));
                    }
                    if !f.builtin && ui.add(theme::ghost_button(icons::TRASH)).on_hover_text("Delete the field and everything written in it").clicked() {
                        delete = Some(f.id);
                    }
                });
            }
            if let Some(f) = update {
                let r = self.lib.update_field(&f);
                self.check(r);
            }
            if let Some((id, up)) = moved {
                let r = self.lib.move_field(id, up);
                self.check(r);
            }
            if let Some(id) = delete {
                let r = self.lib.delete_field(id);
                self.check(r);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.settings_form.new_field).hint_text("New field for every paper").desired_width(220.0));
                egui::ComboBox::from_id_salt("new-field-kind").selected_text(self.settings_form.new_kind.label()).show_ui(ui, |ui| {
                    for k in FieldKind::USER_KINDS {
                        ui.selectable_value(&mut self.settings_form.new_kind, k, k.label());
                    }
                });
                if ui.add_enabled(!self.settings_form.new_field.trim().is_empty(), egui::Button::new(format!("{} Add field", icons::PLUS))).clicked() {
                    let r = self.lib.add_field(&self.settings_form.new_field, "", self.settings_form.new_kind, None);
                    if self.check(r).is_some() {
                        self.settings_form.new_field.clear();
                    }
                }
            });
        });
    }

    fn pdf_settings(&mut self, ui: &mut Ui) {
        let mut changed = false;
        theme::card(ui, "PDFs", |ui| {
            ui.horizontal(|ui| {
                ui.label("PDF folder");
                let shown = self.settings.pdf_folder.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "not set".into());
                ui.label(RichText::new(shown).monospace().small());
                if ui.button(format!("{} Choose…", icons::FOLDER_OPEN)).clicked()
                    && let Some(dir) = rfd::FileDialog::new().set_title("Folder where you keep your papers").pick_folder()
                {
                    self.settings.pdf_folder = Some(dir);
                    changed = true;
                }
                if self.settings.pdf_folder.is_some() && ui.add(theme::ghost_button(icons::X)).clicked() {
                    self.settings.pdf_folder = None;
                    changed = true;
                }
            });
            theme::hint(
                ui,
                "PDFs inside this folder are remembered relative to it, so the library still finds them on another computer \
                 (or another operating system) where the folder lives elsewhere. PDFs open in your system's PDF viewer.",
            );
            if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() {
                ui.add_space(6.0);
                if ui
                    .checkbox(&mut self.settings.native_wayland, "Native Wayland window")
                    .on_hover_text("Takes effect when LitAnatomy is next started.")
                    .changed()
                {
                    changed = true;
                }
                theme::hint(
                    ui,
                    "On Wayland, PDFs and files can only be dropped on the window when it runs through XWayland (the default). \
                     A native Wayland window can look sharper with display scaling, but nothing can be dropped on it: paste \
                     copied files with Ctrl+V instead, or use Add papers › From PDFs…. Restart LitAnatomy after changing this.",
                );
            }
        });
        if changed {
            self.save_settings(ui.ctx());
        }
    }

    fn library_settings(&mut self, ui: &mut Ui) {
        let mut switch_to = None;
        theme::card(ui, "Library file", |ui| {
            let path = self.lib.path().map(|p| p.display().to_string()).unwrap_or_else(|| "(in memory)".into());
            ui.label(RichText::new(path).monospace().small());
            theme::hint(ui, "Everything (papers, notes, collections) is in this one file. Keep it in a synced folder to use it on several computers, one at a time.");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(format!("{} Open another…", icons::FOLDER_OPEN)).clicked() {
                    switch_to = rfd::FileDialog::new().add_filter("LitAnatomy library", &["litanatomy"]).pick_file();
                }
                if ui.button(format!("{} New library…", icons::PLUS)).clicked() {
                    switch_to = rfd::FileDialog::new()
                        .set_file_name("library.litanatomy")
                        .add_filter("LitAnatomy library", &["litanatomy"])
                        .save_file();
                }
                if ui.button(format!("{} Back up…", icons::DATABASE)).clicked() {
                    let name = format!("LitAnatomy-backup-{}.litanatomy", chrono::Local::now().format("%Y-%m-%d"));
                    if let Some(dest) = rfd::FileDialog::new().set_file_name(name).add_filter("LitAnatomy library", &["litanatomy"]).save_file() {
                        let r = self.lib.backup_to(&dest);
                        if self.check(r).is_some() {
                            self.info(format!("Backed up to {}.", dest.display()));
                        }
                    }
                }
                if let Some(dir) = self.lib.path().and_then(|p| p.parent()).map(|p| p.to_path_buf())
                    && ui.button(format!("{} Show folder", icons::ARROW_SQUARE_OUT)).clicked()
                {
                    let r = open::that_detached(&dir).map_err(anyhow::Error::from);
                    self.check(r);
                }
            });
        });
        if let Some(path) = switch_to {
            self.switch_library(ui.ctx(), path);
        }
    }

    fn lookup_settings(&mut self, ui: &mut Ui) {
        let mut changed = false;
        theme::card(ui, "Online lookups", |ui| {
            ui.horizontal(|ui| {
                ui.label("Contact email (optional)");
                let resp = ui.add(egui::TextEdit::singleline(&mut self.settings_form.email).hint_text("you@university.ac.uk").desired_width(260.0));
                if resp.lost_focus() && self.settings_form.email != self.settings.contact_email {
                    self.settings.contact_email = self.settings_form.email.trim().to_string();
                    changed = true;
                }
            });
            theme::hint(
                ui,
                "DOIs are looked up at doi.org and Crossref, arXiv IDs at arXiv, ISBNs at Open Library (then Google Books) and \
                 PubMed IDs at NCBI. A PDF without an identifier is searched for on Crossref by its title or opening words; the \
                 PDF itself never leaves your computer. Crossref asks heavy users for a contact address and serves them faster; \
                 it is only sent if you type one here.",
            );
        });
        if changed {
            self.save_settings(ui.ctx());
        }
    }
}
