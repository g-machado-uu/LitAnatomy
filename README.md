# LitAnatomy

A desktop app for PhD students and researchers to organise their literature
review. It holds your references, and it also helps you **dissect** each paper:
what problem it tackles, what it contributes, how, what it found, its limits,
and why it matters for your own work. It runs on Linux, Windows and macOS.

This application was developed with the assistance of Claude Code using Opus 5.5.

## Features

- **Add papers by DOI, arXiv ID, ISBN or PubMed ID.** Paste one or many, in
  any form: `10.1109/…`, `https://doi.org/…`, `arXiv:2301.12345`,
  `ISBN 978-0-470-63155-3`, `PMID: 31452104`.
- **Add papers from their PDFs.** Drop PDFs (or a folder of them) on the
  window or on the drop area on Home, paste files copied in your file manager
  (Ctrl+V), or use Add papers › From PDFs…. LitAnatomy finds the DOI, arXiv ID
  or ISBN in each PDF, looks the paper up, checks that the title matches the
  PDF, and keeps the PDF linked so it opens from the paper. If no identifier
  is found, it searches Crossref by title, and otherwise uses the PDF's own
  details, marked for you to check.
- **Import references** from BibTeX, BibLaTeX, RIS (EndNote, Mendeley, Zotero,
  Web of Science, Scopus, IEEE Xplore), RefWorks and CSL-JSON. A preview shows
  each paper's new key and any duplicates before importing.
- **Import your own review spreadsheet** (Excel, LibreOffice or CSV, a row
  per paper). You choose where each column goes: the paper's details, a part
  of its anatomy, a metric, or a new field named after the column. The
  mapping is guessed from the headings and remembered for next time. DOIs can
  be looked up to fill in the authors, year and journal.
- **Import from Zotero** (Add papers › From Zotero…): the papers whose PDFs
  Zotero has downloaded, with their details, tags and linked PDFs. It reads a
  copy of Zotero's library, so Zotero can stay open and is never changed.
- **Collections** (with sub-collections) for each piece of work: a first-year
  assessment, a paper you are writing, a thesis chapter, your viva. A paper
  can be in several.
- **A paper's anatomy.** Each paper has these fields:
  - Problem & Motivation
  - Contribution
  - Methodology
  - Key Findings
  - Key Metrics
  - Limitations
  - Research Gaps & Future Work
  - Critical Assessment (with a rating)
  - Key Quotes (with pages)
  - Relevance to This Project (one note per collection)
  - Further Notes

  You can rename, reorder or hide the fields. A collection can add its own,
  such as "Frequency range". Notes save as you type. Key Metrics can be
  filled in as a table (name, value, unit) or written as a list, one
  `Name: value unit` per line; each metric's name becomes a matrix column.

- **Synthesis matrix.** Compare papers side by side, then export the table to
  CSV, Markdown or LaTeX.
- **Export the dissections** of a collection, a selection or one paper as a
  PDF (with a list of the papers and bookmarks), a Word document, an Excel
  workbook (which imports again), CSV or Markdown. Useful for supervisor
  meetings and for writing up.
- **Synthesis notes for a collection.** Group its papers into themes, note
  what each theme's papers share and how each paper fits, keep overview notes,
  and draft a related-work outline (a section per theme, with `\cite` keys and
  the open problems from the papers' notes). Export the outline as Markdown or
  LaTeX.
- **Systematic reviews.** Turn on screening for a collection, write its
  inclusion and exclusion criteria, then include, exclude or mark each paper
  "maybe" on its Screening tab, ticking the criteria that apply. The list
  shows how far the screening has got and why papers were excluded, and
  copies the counts for a PRISMA flow diagram.
- **Readable citation keys** like `zhang2025Wideband`, from a pattern you can
  change. Numeric keys from IEEE Xplore and similar sources are replaced. Once
  exported, a key is pinned so your LaTeX keeps compiling.
- **BibTeX / BibLaTeX export** for a collection or a selection. You can also
  turn on auto-export, which keeps each collection's `.bib` file up to date.
- **PDFs.** Link each paper to its PDF, which opens in your usual PDF viewer.
- **Links between papers**: builds on, extends, contradicts, compares
  against, same group.
- **Reading status, tags and full-text search** across titles, authors and
  notes. The status follows your notes: a paper becomes Reading once you write
  one part of it, and Summarised once every part is written (you can turn this
  off in Settings). Click To read, Reading or Summarised on Home to list those
  papers.
- **Keyboard shortcuts.** Tab and Shift+Tab move between a paper's notes,
  Ctrl+Enter (Cmd+Enter on a Mac) stops editing, Esc goes back a level, and
  Ctrl+Shift+Delete moves the open paper to the Trash. F1, or the keyboard
  button at the bottom left, lists them all.
- **Light and dark mode** and six colour themes.
- **One library file.** Easy to back up, or keep it in a synced folder and use
  it on one computer at a time.

## Install

Download the zip for your system from the
[Releases](https://github.com/g-machado-uu/LitAnatomy/releases) page and
unzip it. It contains a single program.

- **Windows:** run `litanatomy.exe`. If SmartScreen warns, choose _More info_ ›
  _Run anyway_.
- **macOS:** right-click `litanatomy` › _Open_ the first time.
- **Linux:** run `./litanatomy`, or build and install it with a launcher (see
  below).

### Build from source

You need Rust 1.95 or newer ([rustup.rs](https://rustup.rs)).

On Linux you also need the GTK 3, xkbcommon and Wayland development packages:

| Distribution  | Command                                                         |
| ------------- | --------------------------------------------------------------- |
| Fedora        | `sudo dnf install gtk3-devel libxkbcommon-devel wayland-devel`  |
| Debian/Ubuntu | `sudo apt install libgtk-3-dev libxkbcommon-dev libwayland-dev` |

Then build:

```sh
cargo build --release
```

On a Wayland desktop, LitAnatomy runs through XWayland, because dropping
files on a native Wayland window doesn't work yet in the windowing library it
uses. Settings › PDFs can switch to a native Wayland window; dropping then
won't work, but pasting copied files (Ctrl+V) still does.

On Linux, `./install.sh` builds LitAnatomy and installs it for your user,
with a launcher and an icon. `./install.sh --uninstall` removes it again.

## Where things are kept

- **Library:** your papers, notes and collections, in one SQLite file.
  - Linux: `~/.local/share/litanatomy/library.litanatomy`
  - macOS: `~/Library/Application Support/LitAnatomy/`
  - Windows: `%APPDATA%\LitAnatomy\data\`

  Settings › Library file can open another library, create a new one or make
  a backup. You can also pass a library path on the command line:
  `litanatomy path/to/library.litanatomy`.

- **Settings:** `settings.toml` in your system's config folder (e.g.
  `~/.config/litanatomy/` on Linux).
- **PDFs** stay where you keep them. Set a _PDF folder_ in Settings and
  LitAnatomy stores links relative to it, so the same library finds the PDFs
  on another computer.

## Privacy

Lookups only happen when you add papers or press "Fill in":

- DOIs go to doi.org, and to api.crossref.org if doi.org has no record.
- arXiv IDs go to export.arxiv.org; ISBNs to openlibrary.org, then Google
  Books; PubMed IDs to NCBI (eutils.ncbi.nlm.nih.gov).
- For a PDF without an identifier, its title (or, failing that, the opening
  words of its first page) is sent to api.crossref.org to search for it.
  The PDF itself never leaves your computer.

Nothing else leaves your computer. If you type a contact email in Settings,
it is sent to Crossref, as their "polite pool" asks.

<!-- ## Development -->
<!---->
<!-- ```sh -->
<!-- cargo test                           # unit and headless UI tests -->
<!-- cargo test -- --ignored              # also a live DOI lookup (needs the internet) -->
<!-- LITANATOMY_SHOTS=/tmp/shots cargo test ui_tests   # screenshots of each screen -->
<!-- ``` -->
<!---->
<!-- See [RELEASING.md](RELEASING.md) for how releases are made. -->
