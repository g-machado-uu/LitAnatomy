# LitAnatomy

A desktop app for PhD students and researchers to organise their literature
review. It holds your references, and it also helps you **dissect** each paper:
what problem it tackles, what it contributes, how, what it found, its limits,
and why it matters for your own work. It runs on Linux, Windows and macOS.

This application was developed with the assistance of Claude Code using Opus 5.5.

## Features

- **Add papers by DOI.** Paste one or many, in any form: `10.1109/…`,
  `DOI: 10.1109/…`, `https://doi.org/…`, `Digital Object Identifier 10.…`.
- **Import references** from BibTeX, BibLaTeX, RIS (EndNote, Mendeley, Zotero,
  Web of Science, Scopus, IEEE Xplore), RefWorks and CSL-JSON. A preview shows
  each paper's new key and any duplicates before importing.
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
  such as "Frequency range". Notes save as you type.

- **Synthesis matrix.** Compare papers side by side, then export the table to
  CSV, Markdown or LaTeX.
- **Readable citation keys** like `zhang2025Wideband`, from a pattern you can
  change. Numeric keys from IEEE Xplore and similar sources are replaced. Once
  exported, a key is pinned so your LaTeX keeps compiling.
- **BibTeX / BibLaTeX export** for a collection or a selection. You can also
  turn on auto-export, which keeps each collection's `.bib` file up to date.
- **PDFs.** Link each paper to its PDF, which opens in your usual PDF viewer.
- **Links between papers**: builds on, extends, contradicts, compares
  against, same group.
- **Reading status, tags and full-text search** across titles, authors and
  notes.
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

DOI lookups go to doi.org, and to api.crossref.org if doi.org has no record.
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
