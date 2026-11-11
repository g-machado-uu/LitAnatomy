//! The dissection as an Excel workbook: a row per paper, a column per part.
//! Its headings are the fields' names, so it can be imported again.

use anyhow::Result;
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook};

use super::dissection::Dissection;

/// Excel refuses longer cells.
const MAX_CELL: usize = 32_000;

pub fn render(d: &Dissection) -> Result<Vec<u8>> {
    let table = d.table();
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("Dissection")?;
    let head = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x001240))
        .set_text_wrap()
        .set_align(FormatAlign::Top)
        .set_border_bottom(FormatBorder::Thin);
    let body = Format::new().set_text_wrap().set_align(FormatAlign::Top);
    let key = Format::new().set_align(FormatAlign::Top).set_font_name("Consolas");
    let number = Format::new().set_align(FormatAlign::Top).set_align(FormatAlign::Left);
    for (c, h) in table.headers.iter().enumerate() {
        sheet.write_string_with_format(0, c as u16, h, &head)?;
        sheet.set_column_width(c as u16, width(h, table.rows.iter().map(|r| r[c].as_str())))?;
    }
    for (r, row) in table.rows.iter().enumerate() {
        let r = r as u32 + 1;
        for (c, cell) in row.iter().enumerate() {
            let c16 = c as u16;
            let header = table.headers[c].as_str();
            if cell.is_empty() {
                continue;
            }
            match header {
                "Year" | "Rating" if cell.parse::<f64>().is_ok() => {
                    sheet.write_number_with_format(r, c16, cell.parse::<f64>()?, &number)?;
                }
                "Key" => {
                    sheet.write_string_with_format(r, c16, cell, &key)?;
                }
                "DOI" => {
                    let url = crate::import::doi::url(cell);
                    sheet.write_url_with_text(r, c16, url.as_str(), cell)?;
                }
                _ => {
                    let text: String = cell.chars().take(MAX_CELL).collect();
                    sheet.write_string_with_format(r, c16, &text, &body)?;
                }
            }
        }
    }
    if !table.rows.is_empty() {
        sheet.autofilter(0, 0, table.rows.len() as u32, table.headers.len().saturating_sub(1) as u16)?;
    }
    // Keep the headings and the key and title in view.
    sheet.set_freeze_panes(1, 2)?;
    Ok(book.save_to_buffer()?)
}

/// A column wide enough for short values, capped for long notes (which wrap).
fn width<'a>(header: &str, cells: impl Iterator<Item = &'a str>) -> f64 {
    let longest = cells.map(|c| c.lines().map(|l| l.chars().count()).max().unwrap_or(0)).max().unwrap_or(0);
    let want = longest.max(header.chars().count()) as f64 + 2.0;
    let cap = match header {
        "Key" => 26.0,
        "Title" => 50.0,
        "Year" | "Rating" => 8.0,
        _ => 60.0,
    };
    want.clamp(8.0, cap)
}
