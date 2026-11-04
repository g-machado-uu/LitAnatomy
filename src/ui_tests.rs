//! Drives the real UI headlessly. Set `LITANATOMY_SHOTS=<dir>` to also
//! write screenshots of each screen.

use std::path::{Path, PathBuf};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::app::{LitAnatomyApp, Screen, Session, View};
use crate::db::Scope;
use crate::model::ReadingStatus;
use crate::settings::{Settings, ThemeMode};
use crate::ui::dialogs::Dialog;
use crate::ui::{icons, theme};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn harness(library: &Path, settings: Settings) -> Harness<'static, LitAnatomyApp> {
    let path = library.to_path_buf();
    // The harness applies its own theme after the app starts, so pass ours on.
    let theme = if settings.theme_mode == ThemeMode::Dark { egui::Theme::Dark } else { egui::Theme::Light };
    let mut h = Harness::builder().with_size(egui::vec2(1360.0, 880.0)).with_theme(theme).build_eframe(move |cc| {
        theme::install(&cc.egui_ctx, &settings);
        // No settings file: tests never touch the user's real settings.
        LitAnatomyApp::with_settings(settings, None, Some(path))
    });
    h.run_steps(3);
    h
}

fn session<'s>(h: &'s mut Harness<'_, LitAnatomyApp>) -> &'s mut Session {
    match &mut h.state_mut().screen {
        Screen::Ready(s) => s,
        Screen::Failed { error, .. } => panic!("library failed to open: {error}"),
    }
}

fn shot(h: &mut Harness<'_, LitAnatomyApp>, name: &str) {
    if let Ok(dir) = std::env::var("LITANATOMY_SHOTS") {
        h.run_steps(3);
        let img = h.render().expect("render");
        img.save(Path::new(&dir).join(format!("{name}.png"))).unwrap();
    }
}

/// Runs frames (with real time passing) until `done`.
fn wait_until(h: &mut Harness<'_, LitAnatomyApp>, what: &str, done: impl Fn(&mut Session) -> bool) {
    for _ in 0..300 {
        h.run_steps(1);
        if done(session(h)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for: {what}");
}

#[test]
fn import_dissect_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let lib_path = dir.path().join("library.litanatomy");
    let mut h = harness(&lib_path, Settings { theme_mode: ThemeMode::Light, ..Default::default() });
    assert!(h.query_by_label_contains("ADD A PAPER").is_some(), "home screen");
    shot(&mut h, "01_home_empty");

    // A collection, made through its dialog.
    session(&mut h).dialog = Some(Dialog::NewCollection { name: String::new(), parent: None, colour: 0 });
    h.run_steps(2);
    // The home screen's DOI box, then the dialog's name field.
    let input = h.get_all_by_role(Role::TextInput).last().expect("name field");
    input.focus();
    input.type_text("Viva");
    h.run_steps(1);
    h.get_by_label(&format!("{} Create", icons::FOLDER_SIMPLE_PLUS)).click();
    h.run_steps(3);
    let viva = session(&mut h).cache.collections.iter().find(|c| c.name == "Viva").map(|c| c.id).expect("collection made");
    assert_eq!(session(&mut h).view, View::List(Scope::Collection(viva)));

    // Import an IEEE Xplore .bib: numeric keys become readable ones.
    session(&mut h).import_file(&fixture("ieee.bib"), Some(viva));
    h.run_steps(3);
    shot(&mut h, "02_import_preview");
    h.get_by_label_contains("Import 3 new papers").click();
    h.run_steps(3);
    let rows = session(&mut h).lib.rows(Scope::Collection(viva)).unwrap();
    let mut keys: Vec<String> = rows.iter().map(|r| r.citekey.clone()).collect();
    keys.sort();
    assert_eq!(keys, vec!["muller2024Multifunctional", "oliveri2022Synthesis", "zhang2025Wideband"]);
    shot(&mut h, "03_collection_list");

    // Importing the same file again finds the duplicates.
    session(&mut h).import_file(&fixture("ieee.bib"), None);
    h.run_steps(3);
    assert!(h.query_by_label_contains("3 already in your library").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(2);

    // Open a paper and write into its first field.
    let zhang = rows.iter().find(|r| r.citekey == "zhang2025Wideband").unwrap().id;
    session(&mut h).open_paper(zhang);
    h.run_steps(3);
    let editors: Vec<_> = h.get_all_by_role(Role::MultilineTextInput).collect();
    assert!(editors.len() >= 5, "anatomy editors shown: {}", editors.len());
    editors[0].focus();
    editors[0].type_text("Two PIN diodes give four phase states.");
    h.run_steps(2);
    wait_until(&mut h, "note autosaved", |s| {
        let fid = s.lib.fields(None).unwrap()[0].id;
        s.lib.notes(zhang).unwrap().get(&fid).map(String::as_str) == Some("Two PIN diodes give four phase states.")
    });
    let s = session(&mut h);
    // The matrix shows Contribution by default (the field typed into above is Problem & Motivation).
    let contribution = s.lib.fields(None).unwrap().into_iter().find(|f| f.key == "contribution").unwrap().id;
    s.lib.set_note(zhang, contribution, "A 2-bit unit cell with only two PIN diodes.").unwrap();
    s.lib.set_status(zhang, ReadingStatus::Reading).unwrap();
    s.lib.set_rating(zhang, 4).unwrap();
    h.run_steps(3);
    shot(&mut h, "04_paper_anatomy");

    // The other tabs.
    session(&mut h).page.as_mut().unwrap().tab = crate::ui::paper::PaperTab::Metadata;
    h.run_steps(3);
    assert!(h.query_by_label_contains("Save changes").is_some());
    shot(&mut h, "05_paper_metadata");
    session(&mut h).page.as_mut().unwrap().tab = crate::ui::paper::PaperTab::Bibtex;
    h.run_steps(3);
    shot(&mut h, "06_paper_bibtex");

    // Back to the list (Esc), then the matrix.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(session(&mut h).view, View::List(Scope::Collection(viva)));
    session(&mut h).go(View::Matrix(Scope::Collection(viva)));
    h.run_steps(3);
    assert!(h.query_by_label_contains("only two PIN diodes").is_some(), "note shown in the matrix");
    shot(&mut h, "07_matrix");

    // Auto-export keeps the collection's .bib up to date.
    let bib = dir.path().join("thesis/refs.bib");
    std::fs::create_dir_all(bib.parent().unwrap()).unwrap();
    let s = session(&mut h);
    s.settings.auto_export = true;
    let mut c = s.lib.collection(viva).unwrap().unwrap();
    c.bib_export_path = Some(bib.display().to_string());
    s.lib.update_collection(&c).unwrap();
    wait_until(&mut h, "auto-export", |_| bib.exists());
    let text = std::fs::read_to_string(&bib).unwrap();
    assert!(text.contains("@article{zhang2025Wideband,"), "{text}");
    assert!(text.contains("title = {A 2-Bit Wideband {5G} {mm-Wave} {RIS} With Low Sidelobe Levels"), "{text}");
    // Exported keys are pinned.
    assert!(session(&mut h).lib.paper(zhang).unwrap().key_pinned);

    // Home with data, settings, and dark mode.
    session(&mut h).go(View::Home);
    h.run_steps(3);
    assert!(h.query_by_label_contains("CONTINUE WHERE YOU LEFT OFF").is_some());
    shot(&mut h, "08_home");
    session(&mut h).go(View::Settings);
    h.run_steps(3);
    assert!(h.query_by_label_contains("CITATION KEYS").is_some());
    shot(&mut h, "09_settings");
}

#[test]
fn dark_mode_screens() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings { theme_mode: ThemeMode::Dark, ..Default::default() };
    let mut h = harness(&dir.path().join("lib.litanatomy"), settings);
    let s = session(&mut h);
    let rf = s.lib.create_collection("First-year assessment", None, 0).unwrap();
    s.import_file(&fixture("sample.ris"), Some(rf));
    h.run_steps(3);
    h.get_by_label_contains("Import 1 new paper").click();
    h.run_steps(3);
    let id = session(&mut h).lib.rows(Scope::All).unwrap()[0].id;
    assert_eq!(session(&mut h).lib.paper(id).unwrap().citekey, "wang2023Universal");
    shot(&mut h, "10_dark_list");
    session(&mut h).open_paper(id);
    h.run_steps(3);
    shot(&mut h, "11_dark_paper");
}

#[test]
fn unreadable_library_offers_another() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-a-library.litanatomy");
    rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TABLE x (a);").unwrap();
    let p = path.clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_eframe(move |cc| {
            theme::install(&cc.egui_ctx, &Settings::default());
            LitAnatomyApp::with_settings(Settings::default(), None, Some(p))
        });
    h.run_steps(2);
    assert!(matches!(h.state().screen, Screen::Failed { .. }));
    assert!(h.query_by_label_contains("not a LitAnatomy library").is_some());
}

#[test]
fn zotero_import() {
    let dir = tempfile::tempdir().unwrap();
    let zotero = dir.path().join("Zotero");
    std::fs::create_dir_all(&zotero).unwrap();
    let fixture = crate::import::zotero::tests::library(&zotero);
    fixture.execute("VACUUM INTO ?1", [zotero.join("zotero.sqlite").to_string_lossy()]).unwrap();

    let mut h = harness(&dir.path().join("library.litanatomy"), Settings { theme_mode: ThemeMode::Light, ..Default::default() });
    let s = session(&mut h);
    let thesis = s.lib.create_collection("Thesis", None, 0).unwrap();
    // The article is already in the library, without its PDF.
    let mut known = crate::model::NewPaper::default();
    known.bib.title = "Reconfigurable intelligent surfaces".into();
    known.bib.doi = Some("10.1109/tap.2025.1234567".into());
    let known = s.lib.add_paper(&known, "[auth][year][Title]").unwrap();
    s.refresh_cache();

    session(&mut h).import_zotero_from(&zotero, None);
    h.run_steps(3);
    assert!(h.query_by_label_contains("Left out: 1 other item in Zotero").is_some());
    assert!(h.query_by_label_contains("1 already in your library").is_some());
    // Choosing where they go (the "Add to" menu).
    match &mut session(&mut h).dialog {
        Some(Dialog::Import(state)) => state.target = Some(thesis),
        _ => panic!("import preview not shown"),
    }
    h.run_steps(2);
    shot(&mut h, "12_zotero_preview");
    h.get_by_label_contains("Import 1 new paper").click();
    h.run_steps(3);

    let s = session(&mut h);
    let rows = s.lib.rows(Scope::Collection(thesis)).unwrap();
    assert_eq!(rows.len(), 2, "both papers in the chosen collection");
    let pdf = s.lib.paper(known).unwrap().pdf_path.expect("PDF linked to the paper already in the library");
    assert!(pdf.ends_with("Zhang - 2025.pdf"), "{pdf}");
    let lone = rows.iter().find(|r| r.id != known).unwrap().id;
    assert!(s.lib.paper(lone).unwrap().pdf_path.is_some_and(|p| p.ends_with("viruses_as_power_generators.pdf")));
}
