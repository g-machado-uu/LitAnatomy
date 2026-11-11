//! Drives the real UI headlessly. Set `LITANATOMY_SHOTS=<dir>` to also
//! write screenshots of each screen.

use std::path::{Path, PathBuf};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::app::{LitAnatomyApp, Screen, Session, View};
use crate::db::Scope;
use crate::model::{CriterionKind, Decision, NewPaper, ReadingStatus};
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

/// A file dropped on the window, as the windowing integration would hand it over.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn drop_files(h: &mut Harness<'_, LitAnatomyApp>, paths: &[&Path]) {
    h.input_mut().dropped_files =
        paths.iter().map(|p| std::sync::Arc::new(Dropped(p.to_path_buf())) as egui::DroppedFileHandle).collect();
    h.run_steps(1);
}

/// Clicks a button once it has stopped moving: dialogs grow to their full
/// height over their first frames, and a click aims at the last frame's spot.
fn click_when_still(h: &mut Harness<'_, LitAnatomyApp>, label: &str) {
    let mut last = None;
    for _ in 0..60 {
        h.run_steps(1);
        let rect = h.get_by_label_contains(label).rect();
        if last == Some(rect) {
            break;
        }
        last = Some(rect);
    }
    h.get_by_label_contains(label).click();
    h.run_steps(2);
}

fn import_pending(s: &mut Session) -> bool {
    matches!(&s.dialog, Some(Dialog::Import(state)) if state.pending.is_some())
}

fn add(s: &mut Session, family: &str, year: i32, title: &str) -> i64 {
    let mut p = NewPaper::default();
    p.bib.title = title.into();
    p.bib.year = Some(year);
    p.bib.authors = vec![crate::model::Author::new(family, "A.")];
    s.lib.add_paper(&p, &s.settings.citekey_pattern).unwrap()
}

#[test]
fn pdfs_dropped_become_papers() {
    let dir = tempfile::tempdir().unwrap();
    let papers = dir.path().join("Papers");
    std::fs::create_dir_all(papers.join("chapter2")).unwrap();
    let one = papers.join("zhang2025.pdf");
    std::fs::write(
        &one,
        crate::import::pdf::tests::make_pdf(
            &["IEEE Transactions on Testing", "A Wideband Surface for Testing", "DOI: 10.1234/test.5678"],
            "A Wideband Surface for Testing",
            "Wei Zhang and Na Li",
        ),
    )
    .unwrap();
    let two = papers.join("chapter2/notes_on_lenses.pdf");
    std::fs::write(&two, crate::import::pdf::tests::make_pdf(&["Some text"], "", "")).unwrap();
    std::fs::write(papers.join("chapter2/readme.txt"), "not a pdf").unwrap();

    let settings = Settings { theme_mode: ThemeMode::Light, pdf_folder: Some(papers.clone()), ..Default::default() };
    let mut h = harness(&dir.path().join("library.litanatomy"), settings);
    let s = session(&mut h);
    s.net.offline = true;
    let review = s.lib.create_collection("Review", None, 0).unwrap();
    s.refresh_cache();
    s.go(View::List(Scope::Collection(review)));
    h.run_steps(2);

    // Dragging PDFs over the window says what dropping them does (hovered files persist until cleared).
    h.input_mut().hovered_files = vec![egui::HoveredFile { path: Some(one.clone()), ..Default::default() }];
    h.run_steps(1);
    shot(&mut h, "19_drop_hint");
    h.input_mut().hovered_files.clear();

    // A PDF and a folder (with one more PDF in it) dropped on the window.
    drop_files(&mut h, &[&one, &papers.join("chapter2")]);
    wait_until(&mut h, "PDFs read", |s| !import_pending(s));
    h.run_steps(2);
    assert_eq!(h.query_all_by_label_contains("Could not look it up online").count(), 2, "the preview says why it's unsure");
    shot(&mut h, "13_pdf_import");
    click_when_still(&mut h, "Import 2 new papers");

    let s = session(&mut h);
    let rows = s.lib.rows(Scope::Collection(review)).unwrap();
    assert_eq!(rows.len(), 2, "both PDFs imported into the collection on screen");
    let zhang = rows.iter().find(|r| r.title == "A Wideband Surface for Testing").expect("title from the PDF's properties");
    let p = s.lib.paper(zhang.id).unwrap();
    assert_eq!(p.pdf_path.as_deref(), Some("zhang2025.pdf"), "stored relative to the PDF folder");
    assert_eq!(p.bib.authors.len(), 2);
    let lenses = rows.iter().find(|r| r.id != zhang.id).unwrap();
    assert_eq!(lenses.title, "notes on lenses", "named after the file when the PDF says nothing");
    assert_eq!(s.lib.paper(lenses.id).unwrap().pdf_path.as_deref(), Some("chapter2/notes_on_lenses.pdf"));

    // One PDF dropped on an open paper is linked to it instead.
    let zid = zhang.id;
    s.open_paper(zid);
    h.run_steps(2);
    drop_files(&mut h, &[&two]);
    h.run_steps(2);
    let s = session(&mut h);
    assert_eq!(s.lib.paper(zid).unwrap().pdf_path.as_deref(), Some("chapter2/notes_on_lenses.pdf"));
    assert!(s.dialog.is_none());
}

#[test]
fn identifiers_are_recognised() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir.path().join("library.litanatomy"), Settings { theme_mode: ThemeMode::Light, ..Default::default() });
    session(&mut h).net.offline = true;
    session(&mut h).dialog = Some(Dialog::AddDoi {
        text: "arXiv:1706.03762\nISBN 978-0-470-63155-3\nPMID: 31452104\n10.1109/TAP.2025.3603900".into(),
        target: None,
    });
    h.run_steps(3);
    assert!(h.query_by_label("ISBN 9780470631553").is_some());
    shot(&mut h, "18_add_identifiers");
    h.get_by_label_contains("Look up 4 papers").click();
    wait_until(&mut h, "lookups tried", |s| !import_pending(s));
    h.run_steps(2);
    match &session(&mut h).dialog {
        Some(Dialog::Import(state)) => assert_eq!(state.errors.len(), 4, "offline: each lookup fails, and says so"),
        _ => panic!("the import preview stays open with the errors"),
    }
    assert!(h.query_by_label_contains("arXiv:1706.03762: offline").is_some());
}

#[test]
fn screening_a_review() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir.path().join("library.litanatomy"), Settings { theme_mode: ThemeMode::Light, ..Default::default() });
    let s = session(&mut h);
    let review = s.lib.create_collection("Systematic review", None, 2).unwrap();
    let mut c = s.lib.collection(review).unwrap().unwrap();
    c.screening = true;
    s.lib.update_collection(&c).unwrap();
    s.lib.add_criterion(review, CriterionKind::Include, "Peer-reviewed, 2015 or later").unwrap();
    let ec1 = s.lib.add_criterion(review, CriterionKind::Exclude, "Not in English").unwrap();
    s.lib.add_criterion(review, CriterionKind::Exclude, "Simulation only").unwrap();
    let ids = [
        add(s, "Zhang", 2025, "A 2-bit wideband reconfigurable surface"),
        add(s, "Li", 2019, "Varactor-tuned reflectarrays"),
        add(s, "Wu", 2021, "Liquid-crystal metasurfaces at mm-wave"),
    ];
    s.lib.add_to_collection(review, &ids).unwrap();
    s.refresh_cache();
    s.go(View::List(Scope::Collection(review)));
    h.run_steps(3);
    assert!(h.query_by_label_contains("Not screened 3").is_some(), "screening summary");

    // Settings show the criteria.
    let form = crate::ui::dialogs::CollectionForm::load(&session(&mut h).lib, review).unwrap();
    session(&mut h).dialog = Some(Dialog::EditCollection(Box::new(form)));
    h.run_steps(3);
    assert!(h.query_by_label("Exclusion criteria").is_some());
    shot(&mut h, "16_review_settings");
    click_when_still(&mut h, "Cancel");
    assert!(session(&mut h).dialog.is_none(), "Cancel closes the settings");

    // Screen the first paper in the list: ticking a reason to exclude excludes it.
    let first = session(&mut h).visible_rows(Scope::Collection(review))[0].id;
    session(&mut h).open_paper(first);
    h.run_steps(2);
    session(&mut h).page.as_mut().unwrap().tab = crate::ui::paper::PaperTab::Screening;
    h.run_steps(3);
    shot(&mut h, "15_screening_tab");
    h.get_by_label("EC1  Not in English").click();
    h.run_steps(3);
    let s = session(&mut h);
    let screened = s.lib.screening(review, first).unwrap().unwrap();
    assert_eq!((screened.decision, screened.criteria.clone()), (Decision::Exclude, vec![ec1]));

    // On to the next one, still on the Screening tab; Back returns to the list.
    h.get_by_label_contains("Next paper to screen").click();
    h.run_steps(3);
    let s = session(&mut h);
    let View::Paper(next) = s.view.clone() else { panic!("a paper is open") };
    assert_ne!(next, first);
    assert_eq!(s.page.as_ref().unwrap().tab, crate::ui::paper::PaperTab::Screening);
    h.get_by_label(&format!("{} Include", theme::decision_style(Decision::Include).0)).click();
    h.run_steps(2);
    assert_eq!(session(&mut h).lib.screening(review, next).unwrap().unwrap().decision, Decision::Include);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(session(&mut h).view, View::List(Scope::Collection(review)));
    assert!(h.query_by_label_contains("Excluded for: EC1 Not in English (1)").is_some());
    shot(&mut h, "14_screening_list");

    // Filter by decision.
    session(&mut h).list.decision = Some(Decision::Exclude);
    let rows = session(&mut h).visible_rows(Scope::Collection(review));
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![first]);
    let text = session(&mut h).review(review).unwrap().prisma_text("Systematic review");
    assert!(text.contains("Records screened: 2\nNot screened yet: 1\nIncluded: 1\n"), "{text}");
    assert!(text.contains("  EC1 Not in English: 1\n"), "{text}");
}

#[test]
fn synthesis_outline() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir.path().join("library.litanatomy"), Settings { theme_mode: ThemeMode::Light, ..Default::default() });
    let s = session(&mut h);
    let c = s.lib.create_collection("Thesis chapter 2", None, 1).unwrap();
    let a = add(s, "Zhang", 2025, "A 2-bit wideband reconfigurable surface");
    let b = add(s, "Li", 2019, "Varactor-tuned reflectarrays");
    let d = add(s, "Wu", 2021, "Liquid-crystal metasurfaces at mm-wave");
    s.lib.add_to_collection(c, &[a, b, d]).unwrap();
    let fields = s.lib.fields(None).unwrap();
    let field = |k: &str| fields.iter().find(|f| f.key == k).unwrap().id;
    s.lib.set_note(a, field("contribution"), "Two PIN diodes give four phase states. More detail follows.").unwrap();
    s.lib.set_note(b, field("gaps"), "No measurements above 30 GHz.").unwrap();
    let t = s.lib.add_theme(c, "Tuning elements").unwrap();
    s.lib.update_theme(t, "Tuning elements", "Diodes, varactors or liquid crystals: cost against loss.").unwrap();
    s.lib.add_to_theme(t, &[a, b]).unwrap();
    s.lib.set_theme_note(t, b, "Continuous phase with varactors").unwrap();
    s.refresh_cache();
    s.go(View::List(Scope::Collection(c)));
    h.run_steps(2);
    h.get_by_label_contains("Synthesis").click();
    h.run_steps(3);
    assert_eq!(session(&mut h).view, View::Synthesis(c));
    assert!(h.query_by_label_contains("Not in a theme yet (1)").is_some());

    h.get_by_label_contains("Draft from themes").click();
    h.run_steps(2);
    let outline = session(&mut h).synthesis.as_ref().unwrap().outline.clone();
    assert!(outline.contains("## Tuning elements"), "{outline}");
    assert!(outline.contains("- Zhang (2025) \\cite{zhang2025Wideband}: Two PIN diodes give four phase states.\n"), "{outline}");
    assert!(outline.contains("- Li (2019) \\cite{li2019VaractorTuned}: Continuous phase with varactors\n"), "{outline}");
    assert!(outline.contains("## Other work\n\n- Wu (2021)"), "{outline}");
    assert!(outline.contains("## Open problems\n\n- Li (2019) \\cite{li2019VaractorTuned}: No measurements above 30 GHz."), "{outline}");
    wait_until(&mut h, "outline saved", |s| s.lib.synthesis(c).unwrap().1.contains("## Tuning elements"));
    shot(&mut h, "17_synthesis");

    // The paper shows its theme.
    session(&mut h).open_paper(a);
    h.run_steps(3);
    assert!(h.query_by_label_contains("Tuning elements").is_some());
}


/// Types `text` into whatever has the keyboard focus.
fn type_text(h: &mut Harness<'_, LitAnatomyApp>, text: &str) {
    h.event(egui::Event::Text(text.into()));
    h.run_steps(2);
}

fn note(s: &mut Session, paper: i64, key: &str) -> String {
    s.flush_page(true);
    let fields = s.lib.fields(None).unwrap();
    let id = fields.iter().find(|f| f.key == key).unwrap().id;
    s.lib.notes(paper).unwrap().remove(&id).unwrap_or_default()
}

fn focused(h: &mut Harness<'_, LitAnatomyApp>) -> bool {
    h.ctx.memory(|m| m.focused().is_some())
}

#[test]
fn keyboard_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir.path().join("library.litanatomy"), Settings { theme_mode: ThemeMode::Light, ..Default::default() });
    let s = session(&mut h);
    let c = s.lib.create_collection("Review", None, 0).unwrap();
    let a = add(s, "Zhang", 2025, "A 2-bit wideband reconfigurable surface");
    let b = add(s, "Li", 2019, "Varactor-tuned reflectarrays");
    s.lib.add_to_collection(c, &[a, b]).unwrap();
    s.refresh_cache();
    s.go(View::List(Scope::Collection(c)));
    h.run_steps(2);
    session(&mut h).open_paper(a);
    h.run_steps(3);

    // Tab goes to the first note, Tab again to the next, Shift+Tab back.
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    type_text(&mut h, "Too narrow a band.");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    type_text(&mut h, "Two bits.");
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
    h.run_steps(2);
    type_text(&mut h, " Below 10 GHz.");
    assert_eq!(note(session(&mut h), a, "problem"), "Too narrow a band. Below 10 GHz.");
    assert_eq!(note(session(&mut h), a, "contribution"), "Two bits.");

    // Ctrl+Enter stops editing: the note is saved and shown formatted.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run_steps(2);
    assert!(!focused(&mut h));
    let problem = session(&mut h).lib.fields(None).unwrap().into_iter().find(|f| f.key == "problem").unwrap().id;
    assert!(session(&mut h).page.as_ref().unwrap().preview.contains(&problem));

    // The first Esc in a field only leaves it; the next goes back a level.
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert!(focused(&mut h));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!focused(&mut h));
    assert_eq!(session(&mut h).view, View::Paper(a));

    // Ctrl+Shift+Delete with no field selected asks before trashing the paper.
    h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Delete);
    h.run_steps(2);
    assert!(matches!(session(&mut h).dialog, Some(Dialog::ConfirmTrash(id)) if id == a));
    shot(&mut h, "21_confirm_trash");
    click_when_still(&mut h, "Move to Trash");
    let s = session(&mut h);
    assert!(s.lib.paper(a).unwrap().deleted);
    assert_eq!(s.view, View::List(Scope::Collection(c)), "back to the collection");

    // Esc: from a paper to its collection, from the collection to Home.
    s.open_paper(b);
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(session(&mut h).view, View::List(Scope::Collection(c)));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(session(&mut h).view, View::Home);

    // F1, or the keyboard button at the bottom left, lists the shortcuts.
    h.key_press(egui::Key::F1);
    h.run_steps(3);
    assert!(matches!(session(&mut h).dialog, Some(Dialog::Shortcuts)));
    shot(&mut h, "22_shortcuts");
    h.key_press(egui::Key::F1);
    h.run_steps(2);
    assert!(session(&mut h).dialog.is_none());
    h.get_by_label(icons::KEYBOARD).click();
    h.run_steps(2);
    assert!(matches!(session(&mut h).dialog, Some(Dialog::Shortcuts)));
}

#[test]
fn home_drop_zone_and_paste() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("zhang2025.pdf");
    std::fs::write(&pdf, crate::import::pdf::tests::make_pdf(&["A Wideband Surface for Testing"], "A Wideband Surface for Testing", "Wei Zhang")).unwrap();
    let mut h = harness(&dir.path().join("library.litanatomy"), Settings { theme_mode: ThemeMode::Light, ..Default::default() });
    session(&mut h).net.offline = true;
    assert!(h.query_by_label("Drop PDFs or a folder here").is_some());
    shot(&mut h, "20_home");

    // The zone lights up while files are dragged over the window.
    h.input_mut().hovered_files = vec![egui::HoveredFile { path: Some(pdf.clone()), ..Default::default() }];
    h.run_steps(2);
    assert!(h.query_by_label("Drop to add the papers").is_some());
    shot(&mut h, "20_home_dragging");
    h.input_mut().hovered_files.clear();
    h.run_steps(1);

    // Files copied in a file manager and pasted with Ctrl+V are added too.
    let uri = format!("file://{}", pdf.display().to_string().replace(' ', "%20"));
    h.event(egui::Event::Paste(uri));
    h.run_steps(1);
    assert!(matches!(session(&mut h).dialog, Some(Dialog::Import(_))), "pasting files starts an import");
    wait_until(&mut h, "PDF read", |s| !import_pending(s));
    h.run_steps(2);
    assert!(h.query_by_label_contains("Import 1 new paper").is_some());
}

#[test]
fn pasted_file_lists() {
    use crate::app::file_paths;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a b.pdf");
    let b = dir.path().join("ü.pdf");
    std::fs::write(&a, "").unwrap();
    std::fs::write(&b, "").unwrap();
    let a_uri = format!("file://{}", a.display().to_string().replace(' ', "%20"));
    let b_uri = format!("file://{}", b.display().to_string().replace('ü', "%C3%BC"));
    assert_eq!(file_paths(&format!("{a_uri}\r\n{b_uri}\n")), vec![a.clone(), b.clone()]);
    assert_eq!(file_paths(&format!("\"{}\"", a.display())), vec![a.clone()]);
    assert!(file_paths("10.1109/TAP.2025.3603900").is_empty(), "a DOI isn't a file");
    assert!(file_paths(&format!("{}\nsome words", a.display())).is_empty(), "everything must be a file");
    assert!(file_paths(&format!("{}", dir.path().join("missing.pdf").display())).is_empty());
}

#[test]
fn metrics_list_status_and_home_counts() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(&dir.path().join("library.litanatomy"), Settings { theme_mode: ThemeMode::Light, ..Default::default() });
    let s = session(&mut h);
    let a = add(s, "Zhang", 2025, "A 2-bit wideband reconfigurable surface");
    add(s, "Li", 2019, "Varactor-tuned reflectarrays");
    add(s, "Wu", 2021, "Liquid-crystal phase shifters");
    s.refresh_cache();
    s.go(View::Home);
    h.run_steps(3);

    // The status cards on Home list their papers.
    h.get_by_role_and_label(Role::Button, "To read").click();
    h.run_steps(3);
    let s = session(&mut h);
    assert_eq!(s.view, View::List(Scope::All));
    assert_eq!(s.list.status, Some(ReadingStatus::ToRead));
    assert_eq!(s.visible_rows(Scope::All).len(), 3);
    assert!(h.query_by_role_and_label(Role::Button, "Reading").is_none(), "no card to click for an empty status");

    // Key Metrics written as a list, one per line.
    session(&mut h).open_paper(a);
    h.run_steps(3);
    h.get_by_label_contains("List").click();
    h.run_steps(2);
    assert!(session(&mut h).settings.metrics_as_list);
    // Problem, Contribution, Methodology, Key Findings, then Key Metrics.
    for _ in 0..5 {
        h.key_press(egui::Key::Tab);
        h.run_steps(1);
    }
    type_text(&mut h, "Gain: 30.8 dB");
    h.key_press(egui::Key::Enter);
    type_text(&mut h, "- Bandwidth = 26–31 GHz");
    shot(&mut h, "30_metrics_list");
    let metrics = crate::model::parse_metrics(&note(session(&mut h), a, "metrics"));
    let pairs: Vec<(String, String)> = metrics.iter().map(|m| (m.name.clone(), m.value_with_unit())).collect();
    assert_eq!(pairs, [("Gain".to_string(), "30.8 dB".to_string()), ("Bandwidth".to_string(), "26–31 GHz".to_string())]);

    // Writing the first part made the paper Reading, and said so.
    assert_eq!(session(&mut h).lib.paper(a).unwrap().status, ReadingStatus::Reading);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Marked as Reading").is_some());

    // Back to the table, which shows the same metrics.
    h.get_by_label_contains("Table").click();
    h.run_steps(2);
    shot(&mut h, "31_metrics_table");
    assert!(!session(&mut h).settings.metrics_as_list);

    // Every part written: Summarised, and Home's Summarised card lists it.
    let s = session(&mut h);
    for f in s.lib.fields(None).unwrap().iter().filter(|f| f.kind != crate::model::FieldKind::Relevance && f.key != "metrics") {
        s.lib.set_note(a, f.id, "Written").unwrap();
    }
    assert_eq!(s.lib.paper(a).unwrap().status, ReadingStatus::Summarised);
    s.go(View::Home);
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Summarised").click();
    h.run_steps(3);
    let s = session(&mut h);
    assert_eq!(s.list.status, Some(ReadingStatus::Summarised));
    assert_eq!(s.visible_rows(Scope::All).iter().map(|r| r.id).collect::<Vec<_>>(), [a]);
}

#[test]
fn spreadsheet_import_and_notes_export() {
    let dir = tempfile::tempdir().unwrap();
    // A review sheet like a student keeps: no heading over the DOIs, questions as headings.
    let sheet = dir.path().join("Lit Review.xlsx");
    {
        let mut book = rust_xlsxwriter::Workbook::new();
        let ws = book.add_worksheet();
        let rows = [
            ["", "Paper Name", "What did they do", "How did they do", "Frequency Range", "Why it is relevant for me", "Additional Points"],
            ["DOI: 10.1109/TAP.2025.1234", "A 2-bit reconfigurable\nsurface", "Two diodes per cell", "Full-wave simulation", "26-31 GHz", "Same band as mine", "Ask about bias"],
            ["", "Varactor reflectarrays", "Varactor tuning", "", "", "", ""],
        ];
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                ws.write_string(r as u32, c as u16, *cell).unwrap();
            }
        }
        book.save(&sheet).unwrap();
    }
    let mut h = harness(&dir.path().join("library.litanatomy"), Settings { theme_mode: ThemeMode::Light, ..Default::default() });
    let s = session(&mut h);
    s.net.offline = true;
    let viva = s.lib.create_collection("Viva", None, 0).unwrap();
    s.refresh_cache();
    s.import_file(&sheet, Some(viva));
    h.run_steps(3);
    assert!(matches!(session(&mut h).dialog, Some(Dialog::ImportSheet(_))), "the mapping dialog opens");
    assert!(h.query_by_label_contains("2 papers to import").is_some());
    shot(&mut h, "40_sheet_mapping");
    // Make "Additional Points" a new field instead of Further Notes.
    if let Some(Dialog::ImportSheet(form)) = &mut session(&mut h).dialog {
        form.columns.iter_mut().find(|c| c.header == "Additional Points").unwrap().target = crate::import::sheet::Target::NewField;
    }
    click_when_still(&mut h, "Continue");
    assert!(matches!(session(&mut h).dialog, Some(Dialog::Import(_))), "then the usual preview");
    h.run_steps(3);
    shot(&mut h, "41_sheet_preview");
    assert!(h.query_by_label_contains("notes from the spreadsheet").is_some());
    click_when_still(&mut h, "Import 2 new papers");
    let s = session(&mut h);
    let rows = s.lib.rows(Scope::Collection(viva)).unwrap();
    assert_eq!(rows.len(), 2);
    let surface = rows.iter().find(|r| r.title == "A 2-bit reconfigurable surface").expect("title joined onto one line");
    let paper = s.lib.paper(surface.id).unwrap();
    assert_eq!(paper.bib.doi.as_deref(), Some("10.1109/tap.2025.1234"));
    assert_eq!(paper.status, ReadingStatus::Reading, "notes written: Reading");
    assert_eq!(note(s, surface.id, "contribution"), "Two diodes per cell");
    assert_eq!(note(s, surface.id, "methodology"), "Full-wave simulation");
    assert!(note(s, surface.id, "metrics").contains("Frequency Range"));
    assert_eq!(s.lib.relevance(viva, surface.id).unwrap().as_deref(), Some("Same band as mine"));
    let extra = s.lib.fields(None).unwrap().into_iter().find(|f| f.label == "Additional Points").expect("new field");
    assert_eq!(s.lib.notes(surface.id).unwrap()[&extra.id], "Ask about bias");
    // The mapping is remembered.
    assert_eq!(s.settings.sheet_columns.get("additional points").map(String::as_str), Some("newfield"));

    // Export the collection's notes: the dialog, then each format.
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    s.export_notes(ids, Some(viva), "Viva");
    h.run_steps(3);
    assert!(h.query_by_label_contains("Save as PDF").is_some());
    shot(&mut h, "42_export_notes");
    let s = session(&mut h);
    let Some(Dialog::ExportNotes(form)) = s.dialog.take() else { panic!("export dialog") };
    let mut form = *form;
    for f in crate::export::dissection::Format::ALL {
        form.format = f;
        let path = dir.path().join(crate::export::dissection::file_name("Viva", f));
        s.export_notes_to(&form, &path).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 100);
    }
    let md = std::fs::read_to_string(dir.path().join("Viva – dissection.md")).unwrap();
    assert!(md.contains("Two diodes per cell") && md.contains("### Additional Points"));
}
