//! Look & feel: colour palettes (light and dark), bundled fonts and icons,
//! and the small painted components shared by the screens.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use egui::{
    Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Pos2, Response,
    RichText, Sense, Shadow, Stroke, TextStyle, Theme, ThemePreference, Ui, Vec2, Visuals, WidgetInfo,
};

use crate::model::{Decision, ReadingStatus};
use crate::settings::{Settings, ThemeMode};
use crate::ui::icons;

/// The colours of one palette in one mode.
#[derive(Clone, Copy, Debug)]
pub struct Tones {
    /// Links, icons, focus and selected text.
    pub accent: Color32,
    /// Fill of primary buttons (white text on it).
    pub button: Color32,
    /// Selection and soft highlights.
    pub soft: Color32,
    pub bg: Color32,
    pub card: Color32,
    pub sidebar: Color32,
    pub raised: Color32,
    pub line: Color32,
    pub hover: Color32,
    pub ink: Color32,
    pub muted: Color32,
}

pub struct Palette {
    pub name: &'static str,
    pub light: Tones,
    pub dark: Tones,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

#[allow(clippy::too_many_arguments)]
const fn tones(
    accent: u32,
    button: u32,
    soft: u32,
    bg: u32,
    card: u32,
    sidebar: u32,
    raised: u32,
    line: u32,
    hover: u32,
    ink: u32,
    muted: u32,
) -> Tones {
    Tones {
        accent: hex(accent),
        button: hex(button),
        soft: hex(soft),
        bg: hex(bg),
        card: hex(card),
        sidebar: hex(sidebar),
        raised: hex(raised),
        line: hex(line),
        hover: hex(hover),
        ink: hex(ink),
        muted: hex(muted),
    }
}

pub const PALETTES: &[Palette] = &[
    // Utrecht University dark blue (#001240); the tints share its hue.
    Palette {
        name: "University Blue",
        light: tones(0x001240, 0x001240, 0xDCE4F7, 0xF5F7FB, 0xFFFFFF, 0xE6EBF9, 0xEEF1F8, 0xDDE2EE, 0xE3E8F4, 0x1C2233, 0x5A6277),
        dark: tones(0x7F9EEE, 0x1F3B85, 0x23325A, 0x11141D, 0x191E2A, 0x141925, 0x222837, 0x2A3041, 0x262C3D, 0xE6E9F2, 0x9AA2B6),
    },
    Palette {
        name: "Sage",
        light: tones(0x3F7A57, 0x3F7A57, 0xDDEBE1, 0xF5F7F4, 0xFFFFFF, 0xE6EEE7, 0xEDF2EC, 0xDCE4DB, 0xE2EAE1, 0x1F2A22, 0x5B675D),
        dark: tones(0x8CC7A2, 0x2F6B47, 0x24402F, 0x121714, 0x1A211C, 0x161D18, 0x232C25, 0x2B352D, 0x27302A, 0xE5ECE6, 0x9DAA9F),
    },
    Palette {
        name: "Terracotta",
        light: tones(0xA84E2E, 0xA84E2E, 0xF5E0D6, 0xFAF6F3, 0xFFFFFF, 0xF3E7DF, 0xF5EDE7, 0xEADDD3, 0xEFE3DA, 0x2E2420, 0x6E5F57),
        dark: tones(0xEBA386, 0xA84E2E, 0x4A2C20, 0x181311, 0x211A17, 0x1D1714, 0x2C2420, 0x382D28, 0x322823, 0xF0E7E2, 0xB3A39A),
    },
    Palette {
        name: "Plum",
        light: tones(0x74428F, 0x74428F, 0xEADDF2, 0xF8F5FA, 0xFFFFFF, 0xEEE6F3, 0xF2ECF6, 0xE3DAEA, 0xE9E0EF, 0x271F2D, 0x66596F),
        dark: tones(0xC7A2DB, 0x7A4796, 0x3A2848, 0x151218, 0x1D1922, 0x19151E, 0x28222E, 0x332B3A, 0x2D2634, 0xEDE6F2, 0xABA0B4),
    },
    Palette {
        name: "Graphite",
        light: tones(0x3D4B5C, 0x3D4B5C, 0xDFE4EA, 0xF5F6F7, 0xFFFFFF, 0xE9EBEE, 0xEFF1F3, 0xDEE1E5, 0xE4E7EB, 0x1E2226, 0x5F666E),
        dark: tones(0xB4C0CE, 0x4A5A6E, 0x2F3742, 0x131416, 0x1B1D20, 0x17191B, 0x25282C, 0x303338, 0x2A2D31, 0xE8EAED, 0xA2A8B0),
    },
    Palette {
        name: "High contrast",
        light: tones(0x0B3D91, 0x0B3D91, 0xCFE0FF, 0xFFFFFF, 0xFFFFFF, 0xF0F0F0, 0xF2F2F2, 0x5A5A5A, 0xE4E4E4, 0x000000, 0x2E2E2E),
        dark: tones(0x9CC8FF, 0x0B3D91, 0x0F2E5C, 0x000000, 0x0A0A0A, 0x050505, 0x1A1A1A, 0xA0A0A0, 0x262626, 0xFFFFFF, 0xDDDDDD),
    },
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

pub fn palette_index(name: &str) -> usize {
    PALETTES.iter().position(|p| p.name == name).unwrap_or(0)
}

pub fn current_palette() -> &'static Palette {
    &PALETTES[CURRENT.load(Ordering::Relaxed).min(PALETTES.len() - 1)]
}

pub fn tones_for(dark: bool) -> &'static Tones {
    let p = current_palette();
    if dark { &p.dark } else { &p.light }
}

pub fn t(ui: &Ui) -> &'static Tones {
    tones_for(ui.visuals().dark_mode)
}

// Status colours, tuned for text on white; lightened in dark mode by `fg`.
pub const GREEN: Color32 = hex(0x3E7F4F);
pub const AMBER: Color32 = hex(0xA8611B);
pub const RED: Color32 = hex(0xB3404A);
pub const SLATE: Color32 = hex(0x5B6B7F);

/// Colours a collection can have.
pub const LABEL_COLOURS: [(&str, Color32); 8] = [
    ("Blue", hex(0x2F6FBE)),
    ("Teal", hex(0x23807F)),
    ("Green", hex(0x4C8A3F)),
    ("Ochre", hex(0xB07A12)),
    ("Orange", hex(0xC2582A)),
    ("Rose", hex(0xB84458)),
    ("Plum", hex(0x8352A1)),
    ("Slate", hex(0x5B6B7F)),
];

pub fn label_colour(i: u8) -> Color32 {
    LABEL_COLOURS[i as usize % LABEL_COLOURS.len()].1
}

pub fn status_style(s: ReadingStatus) -> (&'static str, Color32) {
    match s {
        ReadingStatus::ToRead => (icons::CIRCLE_DASHED, SLATE),
        ReadingStatus::Reading => (icons::CIRCLE_HALF, AMBER),
        ReadingStatus::Summarised => (icons::CHECK_CIRCLE, GREEN),
    }
}

const SEMIBOLD: &str = "semibold";
const SERIF: &str = "serif";

/// The app version shown in the sidebar, e.g. "v0.1.0".
/// Icon and colour of a screening decision.
pub fn decision_style(d: Decision) -> (&'static str, Color32) {
    match d {
        Decision::Undecided => (icons::CIRCLE_DASHED, SLATE),
        Decision::Include => (icons::CHECK_CIRCLE, GREEN),
        Decision::Maybe => (icons::QUESTION, AMBER),
        Decision::Exclude => (icons::X_CIRCLE, RED),
    }
}

pub fn decision_chip(ui: &mut Ui, d: Decision) -> Response {
    let (icon, colour) = decision_style(d);
    chip(ui, format!("{icon} {}", d.label()), colour)
}

pub const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// Inter for text, DM Serif Display for titles and Phosphor for icons, in
/// front of egui's own fonts (kept as fallbacks for other scripts).
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("inter", &include_bytes!("../../assets/fonts/Inter-Regular.ttf")[..]),
        ("inter-semibold", include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
        ("dm-serif", include_bytes!("../../assets/fonts/DMSerifDisplay-Regular.ttf")),
        ("phosphor", include_bytes!("../../assets/fonts/Phosphor.ttf")),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    let fallbacks = fonts.families[&FontFamily::Proportional].clone();
    // Phosphor maps a–z to blank ligature glyphs, so it must come after a text font.
    let family = |first: &str| {
        let mut keys = vec![first.to_string()];
        if first != "inter" {
            keys.push("inter".into());
        }
        keys.push("phosphor".into());
        keys.extend(fallbacks.iter().cloned());
        keys
    };
    fonts.families.insert(FontFamily::Proportional, family("inter"));
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), family("inter-semibold"));
    fonts.families.insert(FontFamily::Name(SERIF.into()), family("dm-serif"));
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.push("phosphor".into());
    }
    fonts
}

/// One-off style setup at start-up.
pub fn install(ctx: &egui::Context, settings: &Settings) {
    ctx.set_fonts(fonts());
    ctx.all_styles_mut(|style| {
        use egui::FontFamily::{Monospace, Proportional};
        style.text_styles = [
            (TextStyle::Heading, FontId::new(28.0, FontFamily::Name(SERIF.into()))),
            (TextStyle::Body, FontId::new(14.0, Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, Monospace)),
            (TextStyle::Button, FontId::new(14.0, Proportional)),
            (TextStyle::Small, FontId::new(12.0, Proportional)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 5.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.window_margin = Margin::same(20);
        style.spacing.menu_margin = Margin::same(8);
    });
    apply(ctx, settings);
}

/// Applies the palette, light/dark mode and scale from the settings.
pub fn apply(ctx: &egui::Context, settings: &Settings) {
    CURRENT.store(palette_index(&settings.palette), Ordering::Relaxed);
    for theme in [Theme::Light, Theme::Dark] {
        ctx.set_visuals_of(theme, visuals(theme == Theme::Dark));
    }
    ctx.set_theme(match settings.theme_mode {
        ThemeMode::System => ThemePreference::System,
        ThemeMode::Light => ThemePreference::Light,
        ThemeMode::Dark => ThemePreference::Dark,
    });
    let scale = settings.ui_scale.clamp(0.75, 1.75);
    if (ctx.zoom_factor() - scale).abs() > f32::EPSILON {
        ctx.set_zoom_factor(scale);
    }
}

fn visuals(dark: bool) -> Visuals {
    let c = tones_for(dark);
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    let radius = CornerRadius::same(8);
    v.panel_fill = c.bg;
    v.window_fill = c.card;
    v.faint_bg_color = c.raised;
    v.extreme_bg_color = c.bg;
    v.text_edit_bg_color = Some(if dark { c.bg } else { c.card });
    v.code_bg_color = c.raised;
    v.window_stroke = Stroke::new(1.0, c.line);
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(10);
    let shadow = Color32::from_black_alpha(if dark { 100 } else { 20 });
    v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: shadow };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: shadow };
    v.warn_fg_color = fg_for(dark, AMBER);
    v.error_fg_color = fg_for(dark, RED);
    v.hyperlink_color = c.accent;
    v.selection.bg_fill = c.soft;
    v.selection.stroke = Stroke::new(1.0, c.accent);
    v.striped = false;
    v.collapsing_header_frame = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = c.card;
    w.noninteractive.weak_bg_fill = c.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, c.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, c.ink);
    w.noninteractive.corner_radius = radius;
    for (state, fill, stroke) in [
        (&mut w.inactive, c.raised, c.line),
        (&mut w.hovered, c.hover, c.line.lerp_to_gamma(c.ink, 0.15)),
        (&mut w.active, c.hover, c.accent),
        (&mut w.open, c.hover, c.line),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.0, c.ink);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    v
}

fn fg_for(dark: bool, c: Color32) -> Color32 {
    if dark { c.lerp_to_gamma(Color32::WHITE, 0.42) } else { c }
}

/// A status or label colour made readable as text on the current theme.
pub fn fg(ui: &Ui, c: Color32) -> Color32 {
    fg_for(ui.visuals().dark_mode, c)
}

/// A soft wash of `c`, for chip and badge backgrounds.
pub fn soft(ui: &Ui, c: Color32) -> Color32 {
    if ui.visuals().dark_mode {
        t(ui).card.lerp_to_gamma(c, 0.32)
    } else {
        ui.visuals().window_fill.lerp_to_gamma(c, 0.14)
    }
}

pub fn accent(ui: &Ui) -> Color32 {
    t(ui).accent
}

pub fn muted(ui: &Ui) -> Color32 {
    t(ui).muted
}

pub fn semibold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(FontFamily::Name(SEMIBOLD.into()))
}

pub fn serif(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(FontFamily::Name(SERIF.into())).size(size)
}

pub fn semibold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Small rounded pill with a soft coloured background, sized to its text.
pub fn chip(ui: &mut Ui, text: impl Into<String>, color: Color32) -> Response {
    chip_sensing(ui, text.into(), color, Sense::hover())
}

/// A chip that can be clicked, outlined while hovered.
pub fn chip_button(ui: &mut Ui, text: impl Into<String>, color: Color32) -> Response {
    chip_sensing(ui, text.into(), color, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn chip_sensing(ui: &mut Ui, text: String, color: Color32, sense: Sense) -> Response {
    let galley = ui.painter().layout_no_wrap(text.clone(), FontId::proportional(12.0), fg(ui, color));
    let size = galley.size() + egui::vec2(16.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let role = if sense.senses_click() { egui::Role::Button } else { egui::Role::Label };
    resp.widget_info(|| WidgetInfo::labeled(role, true, &text));
    ui.painter().rect_filled(rect, CornerRadius::same(10), soft(ui, color));
    if sense.senses_click() && resp.hovered() {
        ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, fg(ui, color)), egui::StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg(ui, color));
    resp
}

/// Status chip: icon and label in the status colour.
pub fn status_chip(ui: &mut Ui, s: ReadingStatus) -> Response {
    let (icon, colour) = status_style(s);
    chip(ui, format!("{icon} {}", s.label()), colour)
}

/// The frame for modal dialogs.
pub fn modal_frame(ui: &Ui) -> egui::Frame {
    egui::Frame::popup(ui.style()).corner_radius(CornerRadius::same(16)).inner_margin(Margin::same(24))
}

/// Frame of a card: rounded, light border, on the card fill.
pub fn card_frame(ui: &Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(16))
}

/// A titled card that fills the available width.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    card_frame(ui)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                card_title(ui, title);
            }
            add(ui)
        })
        .inner
}

/// Card heading in small capitals.
pub fn card_title(ui: &mut Ui, title: &str) {
    ui.label(semibold(title.to_uppercase()).size(11.5).extra_letter_spacing(0.8).color(muted(ui)));
    ui.add_space(4.0);
}

/// Serif page title with a muted subtitle, and room for actions on the right.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        // The buttons first, so a long title is cut short rather than run under them.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            right(ui);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(title).heading()).truncate());
                    if !subtitle.is_empty() {
                        ui.add(egui::Label::new(RichText::new(subtitle).color(muted(ui))).truncate());
                    }
                });
            });
        });
    });
    ui.add_space(12.0);
}

/// The filled button used for each screen's main action.
pub fn primary_button(ui: &Ui, text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(semibold(text).color(Color32::WHITE))
        .fill(t(ui).button)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(10))
        .min_size(egui::vec2(0.0, 32.0))
}

/// A button for destructive actions.
pub fn danger_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(semibold(text).color(Color32::WHITE))
        .fill(RED)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(10))
        .min_size(egui::vec2(0.0, 32.0))
}

/// A quiet button: just an icon (and text), no frame until hovered.
pub fn ghost_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(text.into()).frame_when_inactive(false).corner_radius(CornerRadius::same(8))
}

/// An icon centred in a softly tinted rounded square.
pub fn icon_badge(ui: &mut Ui, icon: &str, color: Color32, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same((size / 3.5) as u8), soft(ui, color));
    painter.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(size * 0.52), fg(ui, color));
    resp
}

/// A row that is one big click target: hovering anywhere on it highlights
/// it, and its text can't be selected (so a click never starts a selection).
pub fn clickable_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let resp = ui
        .scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            ui.set_min_width(ui.available_width());
            add(ui);
        })
        .response;
    if resp.hovered() {
        let rect = resp.rect.expand2(egui::vec2(6.0, 3.0));
        ui.painter().set(background, egui::Shape::rect_filled(rect, CornerRadius::same(8), t(ui).hover));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A dashed box to drop files on (and click to choose them). `active`
/// lights it up while files are dragged over the window.
pub fn drop_zone(ui: &mut Ui, size: Vec2, active: bool, title: &str, subtitle: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, title));
    let tones = t(ui);
    let hot = active || resp.hovered();
    let colour = if hot { tones.accent } else { tones.muted };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(12), if active { tones.soft } else if hot { tones.hover } else { Color32::TRANSPARENT });
    if active {
        painter.rect_stroke(rect, CornerRadius::same(12), Stroke::new(2.0, colour), egui::StrokeKind::Inside);
    } else {
        // Dashed outline (square corners keep the dashes simple).
        let r = rect.shrink(1.0);
        let path = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        painter.extend(egui::Shape::dashed_line(&path, Stroke::new(1.3, colour), 6.0, 4.0));
    }
    let icon_size = (size.y * 0.32).clamp(18.0, 30.0);
    painter.text(
        egui::pos2(rect.center().x, rect.top() + size.y * 0.3),
        Align2::CENTER_CENTER,
        icons::FILE_PDF,
        FontId::proportional(icon_size),
        colour,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + size.y * 0.62),
        Align2::CENTER_CENTER,
        title,
        semibold_font(14.0),
        if active { tones.accent } else { tones.ink },
    );
    painter.text(egui::pos2(rect.center().x, rect.top() + size.y * 0.83), Align2::CENTER_CENTER, subtitle, FontId::proportional(11.5), tones.muted);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small coloured dot (collection colours).
pub fn dot(ui: &mut Ui, color: Color32, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0 - 1.0, color);
    resp
}

/// A thin rounded progress bar.
pub fn progress(ui: &mut Ui, fraction: f32, width: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 6.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), t(ui).raised.lerp_to_gamma(t(ui).line, 0.6));
    let f = fraction.clamp(0.0, 1.0);
    if f > 0.0 {
        let mut done = rect;
        done.set_width((rect.width() * f).max(6.0));
        painter.rect_filled(done, CornerRadius::same(3), color);
    }
    resp
}

/// A sidebar entry: icon, label and an optional count, with the selected one
/// raised as a card-coloured pill. `dot` replaces the icon with a colour dot.
pub fn nav_item(
    ui: &mut Ui,
    icon: &str,
    dot: Option<Color32>,
    label: &str,
    count: Option<usize>,
    selected: bool,
    indent: f32,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(egui::Role::Button, true, selected, label));
    let c = t(ui);
    let dark = ui.visuals().dark_mode;
    let fill = if selected {
        Some(c.card)
    } else if resp.hovered() {
        Some(if dark { Color32::from_white_alpha(10) } else { Color32::from_white_alpha(120) })
    } else {
        None
    };
    let painter = ui.painter();
    if let Some(fill) = fill {
        if selected {
            painter.add(
                Shadow { offset: [0, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(if dark { 60 } else { 14 }) }
                    .as_shape(rect, CornerRadius::same(9)),
            );
        }
        painter.rect_filled(rect, CornerRadius::same(9), fill);
    }
    let x = rect.left() + 10.0 + indent;
    let ink = if selected { c.accent } else { c.ink };
    match dot {
        Some(colour) => {
            painter.circle_filled(Pos2::new(x + 7.0, rect.center().y), 4.5, colour);
        }
        None => {
            painter.text(Pos2::new(x, rect.center().y), Align2::LEFT_CENTER, icon, FontId::proportional(16.0), ink);
        }
    }
    let right = rect.right() - 10.0;
    let mut text_right = right;
    if let Some(n) = count {
        let g = painter.layout_no_wrap(n.to_string(), FontId::proportional(12.0), c.muted);
        text_right -= g.size().x + 8.0;
        painter.galley(Pos2::new(right - g.size().x, rect.center().y - g.size().y / 2.0), g, c.muted);
    }
    let font = if selected { semibold_font(14.0) } else { FontId::proportional(14.0) };
    let max = (text_right - (x + 26.0)).max(10.0);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), font, ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max);
    let g = painter.layout_job(job);
    painter.galley(Pos2::new(x + 26.0, rect.center().y - g.size().y / 2.0), g, ink);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Five clickable stars. Clicking the current rating clears it. Returns true if changed.
pub fn star_rating(ui: &mut Ui, rating: &mut u8, size: f32) -> bool {
    let mut changed = false;
    let gold = fg(ui, hex(0xC28A12));
    let empty = t(ui).line.lerp_to_gamma(t(ui).muted, 0.4);
    let width = size * 5.0 + 8.0;
    ui.allocate_ui_with_layout(egui::vec2(width, size), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut hovered = None;
        let mut rects = Vec::new();
        for i in 1..=5u8 {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
            resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, format!("{i} stars")));
            if resp.hovered() {
                hovered = Some(i);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *rating = if *rating == i { 0 } else { i };
                changed = true;
            }
            rects.push(rect);
        }
        let shown = hovered.unwrap_or(*rating);
        for (i, rect) in rects.into_iter().enumerate() {
            let filled = (i as u8) < shown;
            paint_star(ui, rect, if filled { gold } else { empty }, filled);
        }
    });
    changed
}

fn paint_star(ui: &Ui, rect: egui::Rect, color: Color32, filled: bool) {
    let c = rect.center();
    let r_out = rect.width() * 0.46;
    let r_in = r_out * 0.45;
    let points: Vec<Pos2> = (0..10)
        .map(|k| {
            let r = if k % 2 == 0 { r_out } else { r_in };
            let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
            Pos2::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    if filled {
        // A star is not convex: fill it as five triangles around the centre plus the inner pentagon.
        let inner: Vec<Pos2> = points.iter().copied().skip(1).step_by(2).collect();
        ui.painter().add(egui::Shape::convex_polygon(inner, color, Stroke::NONE));
        for k in 0..5 {
            let tip = points[k * 2];
            let left = points[(k * 2 + 9) % 10];
            let right = points[k * 2 + 1];
            ui.painter().add(egui::Shape::convex_polygon(vec![left, tip, right], color, Stroke::NONE));
        }
    } else {
        let mut closed = points.clone();
        closed.push(points[0]);
        ui.painter().add(egui::Shape::line(closed, Stroke::new(1.3, color)));
    }
}

/// A row of options drawn as one segmented control. Returns the clicked one.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, current: T, options: &[(T, String)]) -> Option<T> {
    let mut picked = None;
    egui::Frame::new()
        .fill(t(ui).raised)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (value, label) in options {
                    let selected = *value == current;
                    let text = if selected { semibold(label.clone()).color(t(ui).accent) } else { RichText::new(label.clone()) };
                    let button = egui::Button::new(text)
                        .fill(if selected { t(ui).card } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(8))
                        .min_size(egui::vec2(0.0, 26.0));
                    if ui.add(button).clicked() && !selected {
                        picked = Some(*value);
                    }
                }
            });
        });
    picked
}

/// Muted helper text.
pub fn hint(ui: &mut Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(text.into()).small().color(muted(ui))).wrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color32) -> f64 {
        let ch = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// Every palette meets WCAG AA (4.5:1) for body text, secondary text,
    /// links and button labels, in light and dark mode.
    #[test]
    fn palettes_are_readable() {
        for p in PALETTES {
            for (mode, c) in [("light", &p.light), ("dark", &p.dark)] {
                let checks = [
                    ("ink on card", c.ink, c.card),
                    ("ink on bg", c.ink, c.bg),
                    ("ink on sidebar", c.ink, c.sidebar),
                    ("muted on card", c.muted, c.card),
                    ("muted on sidebar", c.muted, c.sidebar),
                    ("accent on card", c.accent, c.card),
                    ("accent on bg", c.accent, c.bg),
                    ("white on button", Color32::WHITE, c.button),
                ];
                for (what, a, b) in checks {
                    let r = contrast(a, b);
                    assert!(r >= 4.5, "{} {mode}: {what} is {r:.2}:1", p.name);
                }
            }
        }
    }

    #[test]
    fn status_colours_are_readable() {
        for p in PALETTES {
            for colour in [GREEN, AMBER, RED, SLATE] {
                assert!(contrast(colour, p.light.card) >= 4.5, "{} light {colour:?}", p.name);
                let dark = fg_for(true, colour);
                assert!(contrast(dark, p.dark.card) >= 4.5, "{} dark {colour:?}", p.name);
            }
        }
    }

    /// Every icon and every non-ASCII character in the UI sources has a glyph.
    #[test]
    fn fonts_cover_icons_and_text() {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        let sources = [
            include_str!("../app.rs"),
            include_str!("sidebar.rs"),
            include_str!("home.rs"),
            include_str!("list.rs"),
            include_str!("paper.rs"),
            include_str!("matrix.rs"),
            include_str!("dialogs.rs"),
            include_str!("settings.rs"),
            include_str!("synthesis.rs"),
            include_str!("../db/fields.rs"),
            include_str!("../import/pdf.rs"),
            include_str!("../import/lookup.rs"),
            include_str!("../export/outline.rs"),
        ];
        for font in [FontId::proportional(14.0), semibold_font(14.0), FontId::new(14.0, FontFamily::Name(SERIF.into()))] {
            let mut missing: Vec<char> = sources
                .iter()
                .flat_map(|src| src.lines())
                .filter(|line| !line.trim_start().starts_with("//"))
                .flat_map(str::chars)
                .chain(icons::ALL.iter().flat_map(|i| i.chars()))
                .filter(|c| !c.is_ascii() && !ctx.fonts_mut(|f| f.has_glyph(&font, *c)))
                .collect();
            missing.dedup();
            assert!(missing.is_empty(), "missing glyphs in {:?}: {missing:?}", font.family);
        }
    }

    /// The text fonts must not have glyphs at the icon codepoints, or they
    /// would be drawn instead of the Phosphor icons.
    #[test]
    fn text_fonts_leave_icons_to_phosphor() {
        let all = fonts();
        for text_font in ["inter", "inter-semibold", "dm-serif"] {
            let mut fonts = egui::FontDefinitions::empty();
            fonts.font_data.insert(text_font.into(), all.font_data[text_font].clone());
            fonts.families.insert(egui::FontFamily::Proportional, vec![text_font.into()]);
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts);
            let mut out = ctx.run_ui(Default::default(), |_| {});
            out.textures_delta.clear();
            let font = egui::FontId::proportional(14.0);
            let shadowed: Vec<_> =
                icons::ALL.iter().flat_map(|i| i.chars()).filter(|c| ctx.fonts_mut(|f| f.has_glyph(&font, *c))).collect();
            assert!(shadowed.is_empty(), "{text_font} has glyphs for icons {shadowed:?}");
        }
    }
}
