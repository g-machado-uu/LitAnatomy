use super::*;
use crate::citekey::DEFAULT_PATTERN;
use crate::model::{Author, Biblio, CriterionKind, Decision, FieldKind, NewPaper, ReadingStatus, Relation, Screening};

fn new_paper(family: &str, year: i32, title: &str, doi: Option<&str>) -> NewPaper {
    NewPaper {
        bib: Biblio {
            entry_type: "article".into(),
            title: title.into(),
            authors: vec![Author::new(family, "A.")],
            year: Some(year),
            doi: doi.map(str::to_string),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn new_library_has_default_fields() {
    let lib = Library::open_in_memory().unwrap();
    let fields = lib.fields(None).unwrap();
    assert_eq!(fields.len(), DEFAULT_FIELDS.len());
    assert_eq!(fields[0].label, "Problem & Motivation");
    assert!(fields.iter().all(|f| f.builtin));
    assert!(fields.iter().any(|f| f.kind == FieldKind::Relevance));
}

#[test]
fn reopening_a_file_keeps_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub/lib.litanatomy");
    {
        let mut lib = Library::open(&path).unwrap();
        lib.add_paper(&new_paper("Zhang", 2025, "Wideband Surfaces", None), DEFAULT_PATTERN).unwrap();
    }
    let lib = Library::open(&path).unwrap();
    assert_eq!(lib.rows(Scope::All).unwrap().len(), 1);
    assert_eq!(lib.fields(None).unwrap().len(), DEFAULT_FIELDS.len());
}

#[test]
fn refuses_foreign_and_newer_files() {
    let dir = tempfile::tempdir().unwrap();
    let foreign = dir.path().join("other.db");
    rusqlite::Connection::open(&foreign).unwrap().execute_batch("CREATE TABLE x (a);").unwrap();
    assert!(Library::open(&foreign).is_err());
    let newer = dir.path().join("newer.db");
    rusqlite::Connection::open(&newer).unwrap().execute_batch("PRAGMA user_version = 99;").unwrap();
    let err = Library::open(&newer).err().expect("refused");
    assert!(err.to_string().contains("newer version"), "{err}");
}

#[test]
fn keys_are_unique_and_pinnable() {
    let mut lib = Library::open_in_memory().unwrap();
    let a = lib.add_paper(&new_paper("Zhang", 2025, "Wideband Surfaces", None), DEFAULT_PATTERN).unwrap();
    let b = lib.add_paper(&new_paper("Zhang", 2025, "Wideband Arrays", None), DEFAULT_PATTERN).unwrap();
    assert_eq!(lib.paper(a).unwrap().citekey, "zhang2025Wideband");
    assert_eq!(lib.paper(b).unwrap().citekey, "zhang2025Widebanda");
    assert!(lib.set_citekey(b, "zhang2025wideband").is_err(), "case-insensitive clash");
    assert!(lib.set_citekey(b, "has space").is_err());
    lib.set_citekey(b, "zhangArrays").unwrap();
    let p = lib.paper(b).unwrap();
    assert!(p.key_pinned);
    // Pinned keys survive regeneration unless forced.
    assert_eq!(lib.regenerate_citekey(b, DEFAULT_PATTERN, false).unwrap(), "zhangArrays");
    assert_eq!(lib.regenerate_citekey(b, DEFAULT_PATTERN, true).unwrap(), "zhang2025Widebanda");
    lib.pin_citekeys(&[a]).unwrap();
    assert!(lib.paper(a).unwrap().key_pinned);
}

#[test]
fn duplicates_by_doi_or_title() {
    let mut lib = Library::open_in_memory().unwrap();
    let id = lib
        .add_paper(
            &new_paper("Rocca", 2007, "Efficient Phase-Only Synthesis of Contoured-Beam Patterns", Some("10.1109/x.1")),
            DEFAULT_PATTERN,
        )
        .unwrap();
    let same_doi = new_paper("Other", 2000, "Different", Some("10.1109/X.1"));
    assert_eq!(lib.find_duplicate(&same_doi.bib).unwrap(), Some(id));
    let same_title = new_paper("Rocca", 2007, "Efficient phase-only synthesis of contoured beam\npatterns", None);
    assert_eq!(lib.find_duplicate(&same_title.bib).unwrap(), Some(id));
    let other_year = new_paper("Rocca", 2010, "Efficient Phase-Only Synthesis of Contoured-Beam Patterns", None);
    assert_eq!(lib.find_duplicate(&other_year.bib).unwrap(), None);
    // Merging fills gaps only.
    let mut richer = same_title.bib.clone();
    richer.volume = "55".into();
    richer.title = "Changed".into();
    assert!(lib.merge_into(id, &richer).unwrap());
    let p = lib.paper(id).unwrap();
    assert_eq!(p.bib.volume, "55");
    assert_eq!(p.bib.title, "Efficient Phase-Only Synthesis of Contoured-Beam Patterns");
}

#[test]
fn collections_scopes_and_trash() {
    let mut lib = Library::open_in_memory().unwrap();
    let a = lib.add_paper(&new_paper("Zhang", 2025, "Alpha Surfaces", None), DEFAULT_PATTERN).unwrap();
    let b = lib.add_paper(&new_paper("Li", 2024, "Beta Arrays", None), DEFAULT_PATTERN).unwrap();
    let thesis = lib.create_collection("Thesis", None, 0).unwrap();
    let ch2 = lib.create_collection("Chapter 2", Some(thesis), 1).unwrap();
    assert_eq!(lib.add_to_collection(ch2, &[a, a]).unwrap(), 1);
    lib.add_to_collection(thesis, &[b]).unwrap();
    assert_eq!(lib.rows(Scope::Collection(ch2)).unwrap().len(), 1);
    assert_eq!(lib.rows(Scope::Unfiled).unwrap().len(), 0);
    // Export of a parent includes its sub-collections.
    assert_eq!(lib.export_papers(thesis).unwrap().len(), 2);
    lib.trash(&[a]).unwrap();
    assert_eq!(lib.rows(Scope::All).unwrap().len(), 1);
    assert_eq!(lib.rows(Scope::Trash).unwrap().len(), 1);
    assert_eq!(lib.rows(Scope::Collection(ch2)).unwrap().len(), 0);
    assert_eq!(lib.export_papers(thesis).unwrap().len(), 1);
    lib.restore(&[a]).unwrap();
    assert_eq!(lib.rows(Scope::Collection(ch2)).unwrap().len(), 1);
    // A collection can't be moved under its own child.
    let mut t = lib.collection(thesis).unwrap().unwrap();
    t.parent = Some(ch2);
    lib.update_collection(&t).unwrap();
    assert_eq!(lib.collection(thesis).unwrap().unwrap().parent, None);
    // Deleting a collection keeps the papers.
    lib.delete_collection(thesis).unwrap();
    assert!(lib.collections().unwrap().is_empty(), "sub-collection went too");
    assert_eq!(lib.rows(Scope::Unfiled).unwrap().len(), 2);
    lib.trash(&[a, b]).unwrap();
    assert_eq!(lib.empty_trash().unwrap(), 2);
    assert_eq!(lib.trash_count().unwrap(), 0);
}

#[test]
fn notes_relevance_progress_and_search() {
    let mut lib = Library::open_in_memory().unwrap();
    let a = lib.add_paper(&new_paper("Zhang", 2025, "Wideband Surfaces", None), DEFAULT_PATTERN).unwrap();
    let viva = lib.create_collection("Viva", None, 0).unwrap();
    lib.add_to_collection(viva, &[a]).unwrap();
    let fields = lib.fields(None).unwrap();
    let contribution = fields.iter().find(|f| f.key == "contribution").unwrap().id;
    lib.set_note(a, contribution, "Two PIN diodes give four phase states").unwrap();
    lib.set_relevance(viva, a, "Examiner asked about quantisation lobes").unwrap();
    assert_eq!(lib.notes(a).unwrap()[&contribution], "Two PIN diodes give four phase states");
    assert_eq!(lib.rows(Scope::All).unwrap()[0].filled, 1);
    assert!(lib.search("diode").unwrap().contains(&a));
    assert!(lib.search("quantisation").unwrap().contains(&a));
    assert!(lib.search("zhang wide").unwrap().contains(&a));
    assert!(lib.search("nothing-like-this").unwrap().is_empty());
    assert_eq!(lib.relevance(viva, a).unwrap().as_deref(), Some("Examiner asked about quantisation lobes"));
    // Clearing a note removes it.
    lib.set_note(a, contribution, "  ").unwrap();
    assert!(lib.notes(a).unwrap().is_empty());
    assert!(lib.search("diode").unwrap().is_empty());
    // Status feeds collection progress.
    lib.set_status(a, ReadingStatus::Summarised).unwrap();
    assert_eq!(lib.collection_stats().unwrap()[&viva], (1, 1));
}

#[test]
fn notes_move_the_reading_status_forward() {
    let mut lib = Library::open_in_memory().unwrap();
    let a = lib.add_paper(&new_paper("Zhang", 2025, "Wideband Surfaces", None), DEFAULT_PATTERN).unwrap();
    let viva = lib.create_collection("Viva", None, 0).unwrap();
    lib.add_to_collection(viva, &[a]).unwrap();
    let status = |lib: &Library| lib.paper(a).unwrap().status;
    let fields = lib.fields(None).unwrap();
    let counted: Vec<_> = fields.iter().filter(|f| f.kind != FieldKind::Relevance).collect();
    // Relevance and collection-only fields don't count.
    lib.set_relevance(viva, a, "Why it matters").unwrap();
    let extra = lib.add_field("Frequency range", "", FieldKind::ShortText, Some(viva)).unwrap();
    lib.set_note(a, extra, "26-31 GHz").unwrap();
    assert_eq!(status(&lib), ReadingStatus::ToRead);
    // One part written: Reading.
    lib.set_note(a, counted[0].id, "Something").unwrap();
    assert_eq!(status(&lib), ReadingStatus::Reading);
    // Every part but one: still Reading.
    for f in &counted[1..counted.len() - 1] {
        lib.set_note(a, f.id, "Written").unwrap();
    }
    assert_eq!(status(&lib), ReadingStatus::Reading);
    // A hidden field doesn't count, so hiding the last one completes the paper.
    let mut last = (*counted.last().unwrap()).clone();
    last.hidden = true;
    lib.update_field(&last).unwrap();
    lib.set_note(a, counted[0].id, "Something more").unwrap();
    assert_eq!(status(&lib), ReadingStatus::Summarised);
    // It never moves back: emptying a note keeps Summarised…
    lib.set_note(a, counted[0].id, "").unwrap();
    assert_eq!(status(&lib), ReadingStatus::Summarised);
    // …and a status set by hand stays until the notes say more.
    lib.set_status(a, ReadingStatus::ToRead).unwrap();
    lib.set_note(a, counted[1].id, "Changed").unwrap();
    assert_eq!(status(&lib), ReadingStatus::Reading);
    // Turned off, notes leave the status alone.
    lib.auto_status = false;
    lib.set_status(a, ReadingStatus::ToRead).unwrap();
    lib.set_note(a, counted[0].id, "Again").unwrap();
    assert_eq!(status(&lib), ReadingStatus::ToRead);
}

#[test]
fn custom_fields_per_collection() {
    let mut lib = Library::open_in_memory().unwrap();
    let rf = lib.create_collection("RF surfaces", None, 0).unwrap();
    let other = lib.create_collection("Other", None, 0).unwrap();
    let freq = lib.add_field("Frequency Range", "Operating band", FieldKind::ShortText, Some(rf)).unwrap();
    assert!(lib.fields(Some(rf)).unwrap().iter().any(|f| f.id == freq));
    assert!(!lib.fields(Some(other)).unwrap().iter().any(|f| f.id == freq));
    assert!(!lib.fields(None).unwrap().iter().any(|f| f.id == freq));
    // Built-ins can be renamed and hidden but not deleted.
    let mut builtin = lib.fields(None).unwrap()[0].clone();
    assert!(lib.delete_field(builtin.id).is_err());
    builtin.label = "Motivation".into();
    builtin.hidden = true;
    builtin.kind = FieldKind::Quotes;
    lib.update_field(&builtin).unwrap();
    let after = lib.fields(None).unwrap()[0].clone();
    assert_eq!((after.label.as_str(), after.hidden, after.kind), ("Motivation", true, FieldKind::LongText));
    // Reordering.
    let second = lib.fields(None).unwrap()[1].id;
    lib.move_field(second, true).unwrap();
    assert_eq!(lib.fields(None).unwrap()[0].id, second);
    lib.delete_field(freq).unwrap();
    assert!(lib.fields(Some(rf)).unwrap().iter().all(|f| f.builtin));
}

#[test]
fn links_and_tags() {
    let mut lib = Library::open_in_memory().unwrap();
    let a = lib.add_paper(&new_paper("Zhang", 2025, "Alpha", None), DEFAULT_PATTERN).unwrap();
    let b = lib.add_paper(&new_paper("Li", 2024, "Beta", None), DEFAULT_PATTERN).unwrap();
    lib.add_link(a, b, Relation::BuildsOn, "uses their unit cell").unwrap();
    assert!(lib.add_link(a, a, Relation::Related, "").is_err());
    let from_a = lib.links_of(a).unwrap();
    assert_eq!(from_a.len(), 1);
    assert_eq!(from_a[0].label(), "Builds on");
    assert_eq!(from_a[0].other_id, b);
    let from_b = lib.links_of(b).unwrap();
    assert_eq!(from_b[0].label(), "Built on by");
    lib.delete_link(from_a[0].link.id).unwrap();
    assert!(lib.links_of(a).unwrap().is_empty());

    lib.set_tags(a, &["RIS".into(), "mmWave".into(), "ris".into()]).unwrap();
    assert_eq!(lib.tags_of(a).unwrap(), vec!["mmWave", "RIS"]);
    lib.set_tags(b, &["RIS".into()]).unwrap();
    let tags = lib.tags().unwrap();
    let ris = tags.iter().find(|t| t.1 == "RIS").unwrap();
    assert_eq!(ris.2, 2);
    assert_eq!(lib.papers_with_tag(ris.0).unwrap().len(), 2);
    lib.set_tags(a, &[]).unwrap();
    assert!(lib.tags().unwrap().iter().all(|t| t.1 != "mmWave"), "unused tag removed");
}

#[test]
fn revision_changes_on_writes() {
    let mut lib = Library::open_in_memory().unwrap();
    let r0 = lib.revision();
    lib.create_collection("X", None, 0).unwrap();
    assert!(lib.revision() > r0);
}

#[test]
fn backup_copy() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open(&dir.path().join("a.litanatomy")).unwrap();
    lib.add_paper(&new_paper("Zhang", 2025, "Wideband", None), DEFAULT_PATTERN).unwrap();
    let copy = dir.path().join("b.litanatomy");
    lib.backup_to(&copy).unwrap();
    assert_eq!(Library::open(&copy).unwrap().rows(Scope::All).unwrap().len(), 1);
}

/// A library saved by v0.1 (format 1) opens in v0.2 and keeps its papers.
#[test]
fn upgrades_a_version_1_library() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.litanatomy");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute_batch(
            "INSERT INTO papers (citekey, title, added_at, modified_at) VALUES ('old2020Paper', 'Old Paper', 'x', 'x');
             INSERT INTO collections (name, created_at) VALUES ('Review', 'x');
             INSERT INTO collection_papers (collection_id, paper_id, added_at, relevance) VALUES (1, 1, 'x', 'Why');
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }
    let mut lib = Library::open(&path).unwrap();
    let version: i64 = lib.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(lib.rows(Scope::Collection(1)).unwrap().len(), 1);
    assert_eq!(lib.relevance(1, 1).unwrap().as_deref(), Some("Why"));
    let c = lib.collection(1).unwrap().unwrap();
    assert!(!c.screening);
    assert_eq!(lib.screening(1, 1).unwrap(), Some(Screening::default()));
    assert_eq!(lib.synthesis(1).unwrap(), (String::new(), String::new()));
    // And it is usable: the new tables exist.
    lib.add_theme(1, "Theme").unwrap();
    lib.add_criterion(1, CriterionKind::Exclude, "Not in English").unwrap();
    drop(lib);
    // Opening again is a no-op.
    assert!(Library::open(&path).is_ok());
}

#[test]
fn screening_decisions_and_counts() {
    let mut lib = Library::open_in_memory().unwrap();
    let review = lib.create_collection("Review", None, 0).unwrap();
    let mut c = lib.collection(review).unwrap().unwrap();
    c.screening = true;
    lib.update_collection(&c).unwrap();
    assert!(lib.collection(review).unwrap().unwrap().screening);

    let ic = lib.add_criterion(review, CriterionKind::Include, "Peer reviewed").unwrap();
    let ec1 = lib.add_criterion(review, CriterionKind::Exclude, "Not in English").unwrap();
    let ec2 = lib.add_criterion(review, CriterionKind::Exclude, "No measurements").unwrap();
    assert!(lib.add_criterion(review, CriterionKind::Exclude, "  ").is_err());
    let criteria = lib.criteria(review).unwrap();
    assert_eq!(criteria.iter().map(|c| c.id).collect::<Vec<_>>(), vec![ic, ec1, ec2], "inclusion first");
    assert_eq!(crate::model::criterion_code(&criteria, ec2).as_deref(), Some("EC2"));

    let ids: Vec<i64> = (0..4)
        .map(|i| lib.add_paper(&new_paper("Zhang", 2020 + i, &format!("Paper number {i} on surfaces"), None), DEFAULT_PATTERN).unwrap())
        .collect();
    lib.add_to_collection(review, &ids).unwrap();
    lib.set_screening(review, ids[0], &Screening { decision: Decision::Include, criteria: vec![ic], ..Default::default() }).unwrap();
    lib.set_screening(
        review,
        ids[1],
        &Screening { decision: Decision::Exclude, criteria: vec![ec1, ec2], note: "German".into(), ..Default::default() },
    )
    .unwrap();
    lib.set_decision(review, &[ids[2]], Decision::Exclude).unwrap();
    let s = lib.screening(review, ids[1]).unwrap().unwrap();
    assert_eq!((s.decision, s.note.as_str()), (Decision::Exclude, "German"));
    assert!(s.decided_at.is_some());
    // Not a member: no screening, and setting one fails.
    let outside = lib.add_paper(&new_paper("Li", 2021, "Elsewhere entirely", None), DEFAULT_PATTERN).unwrap();
    assert_eq!(lib.screening(review, outside).unwrap(), None);
    assert!(lib.set_screening(review, outside, &Screening::default()).is_err());

    let sum = lib.screening_summary(review).unwrap();
    assert_eq!(
        (sum.count(Decision::Include), sum.count(Decision::Exclude), sum.count(Decision::Undecided), sum.total()),
        (1, 2, 1, 4)
    );
    assert_eq!(sum.excluded_by, vec![(ec1, 1), (ec2, 1)]);
    assert_eq!(sum.excluded_without_reason, 1);

    // Deleting a criterion un-ticks it; trashed papers drop out of the counts.
    lib.delete_criterion(ec1).unwrap();
    assert_eq!(lib.screening(review, ids[1]).unwrap().unwrap().criteria, vec![ec2]);
    lib.trash(&[ids[0]]).unwrap();
    assert_eq!(lib.screening_summary(review).unwrap().count(Decision::Include), 0);
    // Back to "not screened" clears the date.
    lib.set_decision(review, &[ids[1]], Decision::Undecided).unwrap();
    assert_eq!(lib.screening(review, ids[1]).unwrap().unwrap().decided_at, None);
}

#[test]
fn synthesis_themes_and_outline() {
    let mut lib = Library::open_in_memory().unwrap();
    let c = lib.create_collection("Chapter 2", None, 0).unwrap();
    let a = lib.add_paper(&new_paper("Zhang", 2025, "Wideband Surfaces", None), DEFAULT_PATTERN).unwrap();
    let b = lib.add_paper(&new_paper("Li", 2019, "Narrowband Surfaces", None), DEFAULT_PATTERN).unwrap();
    lib.set_synthesis_notes(c, "Most use PIN diodes.").unwrap();
    lib.set_outline(c, "# Related work").unwrap();
    assert_eq!(lib.synthesis(c).unwrap(), ("Most use PIN diodes.".into(), "# Related work".into()));

    let t1 = lib.add_theme(c, "Unit cells").unwrap();
    let t2 = lib.add_theme(c, "Measurements").unwrap();
    assert!(lib.add_theme(c, " ").is_err());
    assert_eq!(lib.add_to_theme(t1, &[a, b]).unwrap(), 2);
    assert_eq!(lib.add_to_theme(t1, &[a]).unwrap(), 0, "already there");
    lib.set_theme_note(t1, a, "Two diodes").unwrap();
    lib.update_theme(t1, "Unit-cell design", "Cheaper cells").unwrap();
    let themes = lib.themes(c).unwrap();
    assert_eq!(themes[0].name, "Unit-cell design");
    assert_eq!(themes[0].notes, "Cheaper cells");
    // Papers by year: Li (2019) first.
    assert_eq!(themes[0].papers, vec![(b, String::new()), (a, "Two diodes".to_string())]);
    assert_eq!(lib.themes_of(a).unwrap(), vec![(c, "Unit-cell design".to_string())]);

    lib.move_theme(t2, true).unwrap();
    assert_eq!(lib.themes(c).unwrap().iter().map(|t| t.id).collect::<Vec<_>>(), vec![t2, t1]);
    lib.trash(&[b]).unwrap();
    assert_eq!(lib.themes(c).unwrap()[1].papers.len(), 1, "trashed papers are left out");
    lib.remove_from_theme(t1, a).unwrap();
    lib.delete_theme(t2).unwrap();
    assert_eq!(lib.themes(c).unwrap().len(), 1);
    // Deleting the collection deletes its themes.
    lib.delete_collection(c).unwrap();
    assert!(lib.themes_of(b).unwrap().is_empty());
}
