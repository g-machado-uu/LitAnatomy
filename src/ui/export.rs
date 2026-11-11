//! Saving papers' dissections as a PDF, Word document, Excel workbook, CSV
//! or Markdown file.

use std::path::Path;

use anyhow::Result;
use egui::{RichText, Ui};

use crate::app::Session;
use crate::export::dissection::{self, Dissection, Format, Options};
use crate::ui::dialogs::Dialog;
use crate::ui::sidebar::plural;
use crate::ui::{icons, theme};

pub struct ExportForm {
    pub ids: Vec<i64>,
    /// The collection they are shown in: its relevance notes and own fields.
    pub context: Option<i64>,
    pub title: String,
    pub format: Format,
    pub options: Options,
    pub open_after: bool,
}

impl Session {
    /// Asks how to export the notes of papers `ids`.
    pub fn export_notes(&mut self, ids: Vec<i64>, context: Option<i64>, title: impl Into<String>) {
        if ids.is_empty() {
            self.error("There are no papers to export.");
            return;
        }
        // Notes being typed are part of it.
        self.flush_page(true);
        let s = &self.settings;
        let options = Options { abstracts: s.notes_abstracts, empty: s.notes_empty, page_per_paper: s.notes_page_per_paper };
        let form = ExportForm { ids, context, title: title.into(), format: s.notes_format, options, open_after: true };
        self.dialog = Some(Dialog::ExportNotes(Box::new(form)));
    }

    /// Writes the export to `path`.
    pub fn export_notes_to(&mut self, form: &ExportForm, path: &Path) -> Result<()> {
        self.flush_page(true);
        Dissection::load(&self.lib, &form.ids, form.context, &form.title, form.options)?.save(form.format, path)
    }

    pub(crate) fn export_notes_dialog(&mut self, ui: &mut Ui, form: &mut ExportForm) -> bool {
        let mut open = true;
        let mut save = false;
        let resp = egui::Modal::new(egui::Id::unique("export-notes")).frame(theme::modal_frame(ui)).show(ui.ctx(), |ui| {
            ui.set_width(520.0);
            ui.label(theme::serif("Export the dissections", 24.0));
            ui.label(RichText::new(format!("{} · {}", plural(form.ids.len(), "paper"), form.title)).color(theme::muted(ui)));
            ui.add_space(10.0);
            for f in Format::ALL {
                let selected = form.format == f;
                let resp = theme::clickable_row(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mark = if selected { icons::CHECK_CIRCLE } else { icons::CIRCLE_DASHED };
                        let colour = if selected { theme::accent(ui) } else { theme::muted(ui) };
                        ui.label(RichText::new(mark).size(18.0).color(colour));
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(theme::semibold(format!("{}  (.{})", f.label(), f.extension())));
                            ui.add(egui::Label::new(RichText::new(f.describe()).small().color(theme::muted(ui))).wrap());
                        });
                    });
                });
                if resp.clicked() {
                    form.format = f;
                }
                ui.add_space(2.0);
            }
            ui.add_space(8.0);
            ui.checkbox(&mut form.options.abstracts, "Include the abstracts");
            let document = form.format.paged() || form.format == Format::Markdown;
            ui.add_enabled(document, egui::Checkbox::new(&mut form.options.empty, "Show the parts not written yet"));
            ui.add_enabled(form.format.paged(), egui::Checkbox::new(&mut form.options.page_per_paper, "Start each paper on a new page"));
            ui.checkbox(&mut form.open_after, "Open the file once it is saved");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button(ui, format!("{} Save as {}…", icons::EXPORT, form.format.label()))).clicked() {
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
            self.remember_export(ui.ctx(), form);
            let ext = form.format.extension();
            let picked = rfd::FileDialog::new()
                .set_title("Save the dissections")
                .set_file_name(dissection::file_name(&form.title, form.format))
                .add_filter(form.format.label(), &[ext])
                .save_file();
            let Some(mut path) = picked else { return true };
            if path.extension().is_none() {
                path.set_extension(ext);
            }
            match self.export_notes_to(form, &path) {
                Ok(()) => {
                    self.info(format!("Saved {}.", path.display()));
                    if form.open_after
                        && let Err(e) = open::that_detached(&path)
                    {
                        self.error(format!("Saved, but could not open it: {e}"));
                    }
                    return false;
                }
                Err(e) => self.error(format!("Could not export: {e:#}")),
            }
        }
        open
    }

    fn remember_export(&mut self, ctx: &egui::Context, form: &ExportForm) {
        let s = &mut self.settings;
        s.notes_format = form.format;
        s.notes_abstracts = form.options.abstracts;
        s.notes_empty = form.options.empty;
        s.notes_page_per_paper = form.options.page_per_paper;
        self.save_settings(ctx);
    }
}
