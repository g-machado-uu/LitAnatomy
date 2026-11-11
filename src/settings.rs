//! Preferences, kept in `settings.toml` in the user's config folder (not in
//! the library, so a library can be shared without them).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::citekey::DEFAULT_PATTERN;
use crate::export::bibtex::Dialect;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Follow the system",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme_mode: ThemeMode,
    /// Name of a palette in `ui::theme::PALETTES`.
    pub palette: String,
    pub ui_scale: f32,
    pub citekey_pattern: String,
    pub bib_dialect: Dialect,
    pub export_abstracts: bool,
    /// Keep each collection's .bib file (set in the collection) up to date.
    pub auto_export: bool,
    /// PDFs inside this folder are stored relative to it, so the library
    /// still finds them on another computer with the folder elsewhere.
    pub pdf_folder: Option<PathBuf>,
    /// The library file; the default is in the user's data folder.
    pub library_path: Option<PathBuf>,
    /// Optional, sent to Crossref with lookups (its "polite pool").
    pub contact_email: String,
    /// Keep readable keys found in imported files (numeric ones are always replaced).
    pub keep_readable_keys: bool,
    /// Linux: a native Wayland window instead of XWayland (sharper when
    /// scaled, but files can't be dropped on it). Read at start-up.
    pub native_wayland: bool,
    /// Writing a paper's notes moves its status to Reading, and to
    /// Summarised once every part is written.
    pub auto_status: bool,
    /// Key Metrics are edited as a plain list, one "Name: value unit" per line,
    /// instead of a table.
    pub metrics_as_list: bool,
    /// The last export of papers' notes: its format and options.
    pub notes_format: crate::export::dissection::Format,
    pub notes_abstracts: bool,
    pub notes_empty: bool,
    pub notes_page_per_paper: bool,
    /// Spreadsheet imports: where each heading's column went last time
    /// (normalised heading → target). Last: TOML writes tables after values.
    pub sheet_columns: std::collections::BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::System,
            palette: "University Blue".into(),
            ui_scale: 1.0,
            citekey_pattern: DEFAULT_PATTERN.into(),
            bib_dialect: Dialect::Bibtex,
            export_abstracts: false,
            auto_export: false,
            pdf_folder: None,
            library_path: None,
            contact_email: String::new(),
            keep_readable_keys: false,
            native_wayland: false,
            auto_status: true,
            metrics_as_list: false,
            notes_format: crate::export::dissection::Format::Pdf,
            notes_abstracts: false,
            notes_empty: false,
            notes_page_per_paper: true,
            sheet_columns: Default::default(),
        }
    }
}

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", "LitAnatomy")
}

pub fn default_settings_path() -> Option<PathBuf> {
    project_dirs().map(|d| d.config_dir().join("settings.toml"))
}

pub fn default_library_path() -> PathBuf {
    project_dirs()
        .map(|d| d.data_dir().join("library.litanatomy"))
        .unwrap_or_else(|| PathBuf::from("library.litanatomy"))
}

impl Settings {
    /// Missing or unreadable settings fall back to the defaults.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        crate::export::write_if_changed(path, &text).with_context(|| format!("Could not save {}", path.display()))?;
        Ok(())
    }

    pub fn library_path(&self) -> PathBuf {
        self.library_path.clone().unwrap_or_else(default_library_path)
    }

    pub fn export_options(&self) -> crate::export::bibtex::Options {
        crate::export::bibtex::Options { dialect: self.bib_dialect, include_abstract: self.export_abstracts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conf/settings.toml");
        assert_eq!(Settings::load(&path), Settings::default());
        let s = Settings { auto_export: true, palette: "Sage".into(), ui_scale: 1.2, ..Default::default() };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        // Unknown or missing keys don't lose the rest.
        std::fs::write(&path, "auto_export = true\nfuture_option = 3\n").unwrap();
        let loaded = Settings::load(&path);
        assert!(loaded.auto_export);
        assert_eq!(loaded.citekey_pattern, DEFAULT_PATTERN);
    }
}
