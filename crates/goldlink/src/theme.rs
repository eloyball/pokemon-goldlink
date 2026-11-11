//! GoldLink's look (dev/docs/unified-client/UC8.md §2.4): pixel art laid out after Platinum
//! Together's launcher (its look only), HeartGold's golds over a sunset on a Johto town, warm
//! dark panels with gold borders, and two
//! pixel fonts (SIL OFL 1.1): Pixelify Sans for text, Silkscreen for small capital labels. The
//! art is drawn by `scripts/launcher_art.py`.

use std::collections::HashMap;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect,
    Response, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, UiBuilder,
    Vec2, pos2, vec2,
};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const PANEL: Color32 = Color32::from_rgba_premultiplied(38, 22, 30, 240);
pub const PANEL_HEADER: Color32 = rgb(0x5A, 0x2E, 0x2A);
pub const INSET: Color32 = rgb(0x1E, 0x11, 0x18);
/// Panel and field borders: HeartGold's gold.
pub const EDGE: Color32 = rgb(0xE0, 0xA4, 0x40);
pub const EDGE_LIGHT: Color32 = rgb(0xFF, 0xD9, 0x8A);
pub const GREEN: Color32 = rgb(0x6C, 0xC0, 0x5A);
pub const GREEN_DOT: Color32 = rgb(0x4A, 0xDE, 0x80);
pub const AMBER: Color32 = rgb(0xF5, 0x9E, 0x0B);
pub const RED: Color32 = rgb(0xFF, 0x8A, 0x7A);
pub const RED_DARK: Color32 = rgb(0xB0, 0x3A, 0x30);
pub const GOLD: Color32 = rgb(0xFF, 0xD0, 0x50);
pub const GOLD_DARK: Color32 = rgb(0xA8, 0x74, 0x1B);
pub const TEXT: Color32 = rgb(0xFF, 0xF4, 0xE0);
pub const TEXT_DIM: Color32 = rgb(0xE6, 0xC8, 0x9E);
pub const TEXT_FAINT: Color32 = rgb(0x9A, 0x7E, 0x66);
pub const SELECT: Color32 = rgb(0x8A, 0x42, 0x24);
pub const BRACKET: Color32 = rgb(0xFF, 0xE0, 0x70);

/// The capital-letter label font (Silkscreen).
pub fn caps() -> FontFamily {
    FontFamily::Name("caps".into())
}

pub fn caps_bold() -> FontFamily {
    FontFamily::Name("caps-bold".into())
}

pub fn text(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn label(size: f32) -> FontId {
    FontId::new(size, caps())
}

/// The fonts and egui's own widgets (combo boxes, sliders, checkboxes) in the same colours.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(
            name.into(),
            std::sync::Arc::new(FontData::from_static(bytes)),
        );
    };
    add(
        &mut fonts,
        "pixelify",
        include_bytes!("../assets/fonts/PixelifySans.ttf"),
    );
    add(
        &mut fonts,
        "silkscreen",
        include_bytes!("../assets/fonts/Silkscreen-Regular.ttf"),
    );
    add(
        &mut fonts,
        "silkscreen-bold",
        include_bytes!("../assets/fonts/Silkscreen-Bold.ttf"),
    );
    // Pixelify first; egui's own fonts after it cover what it lacks (arrows, accents).
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "pixelify".into());
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    for (family, first) in [("caps", "silkscreen"), ("caps-bold", "silkscreen-bold")] {
        let mut list = vec![first.to_string()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(family.into()), list);
    }
    ctx.set_fonts(fonts);

    ctx.style_mut_of(egui::Theme::Dark, |s| {
        use egui::TextStyle::*;
        s.text_styles = [
            (Heading, text(26.0)),
            (Body, text(17.0)),
            (Button, text(17.0)),
            (Small, text(13.0)),
            (Monospace, FontId::monospace(14.0)),
        ]
        .into();
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        let v = &mut s.visuals;
        v.panel_fill = INSET;
        v.window_fill = PANEL;
        v.window_stroke = Stroke::new(2.0_f32, EDGE);
        v.window_corner_radius = CornerRadius::ZERO;
        v.menu_corner_radius = CornerRadius::ZERO;
        v.extreme_bg_color = INSET;
        v.faint_bg_color = PANEL_HEADER;
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = SELECT;
        v.selection.stroke = Stroke::new(1.0_f32, GOLD);
        v.hyperlink_color = EDGE_LIGHT;
        v.error_fg_color = RED;
        v.warn_fg_color = AMBER;
        v.window_shadow = egui::Shadow::NONE;
        v.popup_shadow = egui::Shadow::NONE;
        for (w, fill, stroke) in [
            (&mut v.widgets.noninteractive, PANEL, PANEL_HEADER),
            (&mut v.widgets.inactive, PANEL_HEADER, EDGE),
            (&mut v.widgets.hovered, SELECT, EDGE_LIGHT),
            (&mut v.widgets.active, SELECT, GOLD),
            (&mut v.widgets.open, SELECT, EDGE_LIGHT),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.bg_stroke = Stroke::new(if fill == PANEL { 1.0_f32 } else { 2.0 }, stroke);
            w.corner_radius = CornerRadius::ZERO;
            w.fg_stroke.color = TEXT;
            w.expansion = 0.0;
        }
    });
    ctx.set_theme(egui::ThemePreference::Dark);
}

/// The art, as textures (nearest-neighbour: pixels stay square).
pub struct Art {
    pub background: TextureHandle,
    icons: HashMap<&'static str, TextureHandle>,
}

fn decode(bytes: &[u8]) -> egui::ColorImage {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .expect("a bundled PNG");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("a bundled PNG");
    let px = &buf[..info.buffer_size()];
    let size = [info.width as usize, info.height as usize];
    match info.color_type {
        png::ColorType::Rgba => egui::ColorImage::from_rgba_unmultiplied(size, px),
        png::ColorType::Rgb => egui::ColorImage::from_rgb(size, px),
        other => panic!("bundled PNG in {other:?}"),
    }
}

impl Art {
    pub fn load(ctx: &egui::Context) -> Self {
        let tex = |name: &str, bytes: &[u8]| {
            ctx.load_texture(name, decode(bytes), TextureOptions::NEAREST)
        };
        let mut icons = HashMap::new();
        macro_rules! icon {
            ($($name:literal),*) => {$(
                icons.insert($name, tex($name, include_bytes!(concat!("../assets/icon-", $name, ".png"))));
            )*};
        }
        icon!(
            "continue", "play", "join", "create", "restore", "import", "settings", "leave"
        );
        Self {
            background: tex("background", include_bytes!("../assets/background.png")),
            icons,
        }
    }

    pub fn icon(&self, name: &str) -> Option<&TextureHandle> {
        self.icons.get(name)
    }

    /// The night, covering `rect` at a whole-pixel scale, centred (cropped at the edges).
    pub fn paint_background(&self, painter: &egui::Painter, rect: Rect) {
        let size = self.background.size_vec2();
        // Covers the window; whole pixels when the window is a multiple of the art's size.
        let scale = (rect.width() / size.x).max(rect.height() / size.y).max(1.0);
        let shown = size * scale;
        let at = Rect::from_center_size(rect.center(), shown);
        painter.rect_filled(rect, CornerRadius::ZERO, INSET);
        painter.with_clip_rect(rect).image(
            self.background.id(),
            at,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

/// A pixel border: `width` px of `colour` inside `rect`.
pub fn border(painter: &egui::Painter, rect: Rect, width: f32, colour: Color32) {
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(width, colour),
        StrokeKind::Inside,
    );
}

/// A panel at `rect`: navy, a bright border, a header strip with `title` (and `icon`); the
/// contents get the rest. Returns what `add` returned.
pub fn panel<R>(
    ui: &mut Ui,
    rect: Rect,
    title: &str,
    accent: Color32,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, PANEL);
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), 34.0));
    p.rect_filled(header, CornerRadius::ZERO, PANEL_HEADER);
    // A dithered corner on the header, as pixel-art panels have.
    for i in 0..6 {
        for j in 0..(6 - i) {
            if (i + j) % 2 == 0 {
                let at = pos2(
                    header.right() - 6.0 - 3.0 * j as f32,
                    header.top() + 4.0 + 3.0 * i as f32,
                );
                p.rect_filled(
                    Rect::from_min_size(at, vec2(2.0, 2.0)),
                    CornerRadius::ZERO,
                    accent.gamma_multiply(0.35),
                );
            }
        }
    }
    p.text(
        pos2(header.left() + 14.0, header.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        text(20.0),
        TEXT,
    );
    border(p, rect, 3.0, accent);
    p.line_segment(
        [
            pos2(rect.left() + 3.0, header.bottom()),
            pos2(rect.right() - 3.0, header.bottom()),
        ],
        Stroke::new(2.0_f32, accent.gamma_multiply(0.6)),
    );
    let inner = Rect::from_min_max(
        pos2(rect.left() + 16.0, header.bottom() + 12.0),
        pos2(rect.right() - 16.0, rect.bottom() - 14.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(inner), add)
        .inner
}

/// A box inside a panel (the reference's stat boxes): an inset with a thin border.
pub fn inset<R>(ui: &mut Ui, colour: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(INSET)
        .stroke(Stroke::new(2.0_f32, colour))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Gold: the one thing to do.
    Primary,
    /// Green: PLAY, the button the whole screen leads to (gold blends into the sunset behind it).
    Go,
    Neutral,
    Danger,
}

/// A pixel button: flat fill, a 2 px bright border, a darker bottom edge, the label centred.
pub fn button(ui: &mut Ui, label: &str, kind: Kind, size: Vec2, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hot = enabled && resp.hovered();
    let (fill, edge, fg) = match (kind, enabled) {
        (_, false) => (rgb(0x33, 0x22, 0x26), rgb(0x5A, 0x44, 0x40), TEXT_FAINT),
        (Kind::Primary, _) => (
            if hot {
                rgb(0xE6, 0xB0, 0x40)
            } else {
                rgb(0xD0, 0x9A, 0x30)
            },
            rgb(0xFF, 0xE0, 0x8A),
            rgb(0x2A, 0x1A, 0x00),
        ),
        (Kind::Go, _) => (
            if hot {
                rgb(0x34, 0xA8, 0x52)
            } else {
                rgb(0x26, 0x8C, 0x42)
            },
            rgb(0x9C, 0xF0, 0xA8),
            TEXT,
        ),
        (Kind::Neutral, _) => (
            if hot {
                rgb(0x7A, 0x40, 0x28)
            } else {
                rgb(0x5E, 0x30, 0x20)
            },
            EDGE_LIGHT,
            TEXT,
        ),
        (Kind::Danger, _) => (
            if hot { rgb(0xD0, 0x55, 0x4A) } else { RED_DARK },
            rgb(0xFF, 0x9A, 0x90),
            TEXT,
        ),
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, fill);
    // The pressed look: a darker bottom strip.
    let shade = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 4.0), rect.max);
    p.rect_filled(shade, CornerRadius::ZERO, Color32::from_black_alpha(70));
    border(p, rect, 2.0, edge);
    let size = if rect.height() >= 50.0 {
        26.0
    } else if rect.height() >= 38.0 {
        20.0
    } else {
        17.0
    };
    p.text(
        rect.center() - vec2(0.0, 1.0),
        egui::Align2::CENTER_CENTER,
        label,
        text(size),
        fg,
    );
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// The selection brackets around the chosen menu row (red corners).
fn brackets(p: &egui::Painter, r: Rect) {
    let s = Stroke::new(3.0_f32, BRACKET);
    let l = 9.0;
    for (corner, dx, dy) in [
        (r.left_top(), 1.0, 1.0),
        (r.right_top(), -1.0, 1.0),
        (r.left_bottom(), 1.0, -1.0),
        (r.right_bottom(), -1.0, -1.0),
    ] {
        p.line_segment([corner, corner + vec2(l * dx, 0.0)], s);
        p.line_segment([corner, corner + vec2(0.0, l * dy)], s);
    }
}

/// One row of the PLAY menu: its icon, its label, an optional badge on the right.
pub fn menu_row(
    ui: &mut Ui,
    art: &Art,
    icon: &str,
    label: &str,
    badge: Option<&str>,
    selected: bool,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect.shrink2(vec2(4.0, 3.0)), CornerRadius::ZERO, SELECT);
        brackets(p, rect);
    } else if resp.hovered() {
        p.rect_filled(
            rect.shrink2(vec2(4.0, 3.0)),
            CornerRadius::ZERO,
            PANEL_HEADER,
        );
    }
    if let Some(t) = art.icon(icon) {
        let r = Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), vec2(24.0, 24.0));
        p.image(
            t.id(),
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    p.text(
        pos2(rect.left() + 52.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        text(21.0),
        TEXT,
    );
    if let Some(b) = badge {
        let g = p.layout_no_wrap(b.to_owned(), label_font(12.0), TEXT);
        let br = Rect::from_center_size(
            pos2(
                rect.right() - 16.0 - g.size().x / 2.0 - 8.0,
                rect.center().y,
            ),
            g.size() + vec2(16.0, 8.0),
        );
        p.rect_filled(br, CornerRadius::ZERO, INSET);
        border(p, br, 2.0, EDGE);
        p.galley(br.center() - g.size() / 2.0, g, TEXT);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn label_font(size: f32) -> FontId {
    label(size)
}

/// A small capital caption above a field ("WORLD CODE").
pub fn caption(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(label(12.0))
            .color(TEXT_DIM),
    );
}

/// A line of secondary text that wraps.
pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(self::text(15.0)).color(TEXT_DIM));
}

/// A round status dot (the fonts have no "●").
pub fn dot(p: &egui::Painter, at: Pos2, colour: Color32) {
    // Pixel-ish: a small square with its corners cut.
    let r = Rect::from_center_size(at, vec2(10.0, 10.0));
    p.rect_filled(r.shrink2(vec2(0.0, 2.0)), CornerRadius::ZERO, colour);
    p.rect_filled(r.shrink2(vec2(2.0, 0.0)), CornerRadius::ZERO, colour);
    p.rect_filled(
        Rect::from_min_size(r.min + vec2(2.0, 2.0), vec2(2.0, 2.0)),
        CornerRadius::ZERO,
        Color32::from_white_alpha(120),
    );
}

/// A status pill: a dot and a capital label, framed.
pub fn pill(ui: &mut Ui, label: &str, colour: Color32) -> Response {
    let g = ui
        .painter()
        .layout_no_wrap(label.to_uppercase(), self::label(15.0), TEXT);
    let size = vec2(g.size().x + 40.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, INSET);
    border(p, rect, 2.0, colour.gamma_multiply(0.8));
    dot(p, pos2(rect.left() + 16.0, rect.center().y), colour);
    p.galley(
        pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0),
        g,
        TEXT,
    );
    resp
}

/// A text field in the pixel style.
pub fn field(ui: &mut Ui, value: &mut String, hint_text: &str, width: f32, size: f32) -> Response {
    let out = egui::Frame::new()
        .fill(INSET)
        .stroke(Stroke::new(2.0_f32, EDGE))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text(RichText::new(hint_text).color(TEXT_FAINT))
                    .font(text(size))
                    .frame(false)
                    .desired_width(width),
            )
        });
    if out.inner.has_focus() {
        border(ui.painter(), out.response.rect, 2.0, GOLD);
    }
    out.inner
}

/// A row of choices (Players 2..6).
pub fn segments<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    choices: &[(T, &str)],
    width: f32,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let w = (width - 4.0 * (choices.len() as f32 - 1.0)) / choices.len() as f32;
        for (v, label) in choices {
            let on = *value == *v;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(
                rect,
                CornerRadius::ZERO,
                if on {
                    SELECT
                } else if resp.hovered() {
                    PANEL_HEADER
                } else {
                    INSET
                },
            );
            border(p, rect, 2.0, if on { GOLD } else { EDGE });
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                *label,
                text(17.0),
                if on { TEXT } else { TEXT_DIM },
            );
            if resp.clicked() && !on {
                *value = *v;
                changed = true;
            }
        }
    });
    changed
}

/// A little animated "…" for waits.
pub fn dots(ui: &Ui) -> &'static str {
    match (ui.input(|i| i.time) * 3.0) as u64 % 4 {
        0 => "",
        1 => ".",
        2 => "..",
        _ => "...",
    }
}

/// Where things go on a launcher screen (UC8 §2.4): the logo plate and the status at the top,
/// two panels, the tip line and the big button at the bottom.
pub struct Chrome {
    pub full: Rect,
    pub status: Rect,
    pub left: Rect,
    pub right: Rect,
    /// Both panels' space (a screen with one panel).
    pub main: Rect,
    pub tip: Rect,
    pub button: Rect,
}

/// Paints the background and the logo plate, and lays the screen out.
pub fn chrome(ui: &mut Ui, art: &Art) -> Chrome {
    let full = ui.max_rect();
    art.paint_background(ui.painter(), full);
    let m = 20.0;
    let gap = 14.0;
    let top_h = 54.0;
    let bottom_h = 62.0;

    // The logo plate: a slanted right edge, gold letters with a shadow.
    let logo = Rect::from_min_size(full.min + vec2(m, m), vec2(268.0, top_h));
    let p = ui.painter();
    let pts = vec![
        logo.left_top(),
        logo.right_top(),
        logo.right_bottom() - vec2(22.0, 0.0),
        logo.left_bottom(),
    ];
    p.add(egui::Shape::convex_polygon(
        pts.clone(),
        PANEL_HEADER,
        Stroke::NONE,
    ));
    p.add(egui::Shape::closed_line(pts, Stroke::new(3.0_f32, EDGE)));
    let title_at = pos2(logo.left() + 16.0, logo.top() + 6.0);
    let big = FontId::new(24.0, caps_bold());
    p.text(
        title_at + vec2(2.0, 2.0),
        egui::Align2::LEFT_TOP,
        "GOLDLINK",
        big.clone(),
        rgb(0x5A, 0x2A, 0x10),
    );
    p.text(title_at, egui::Align2::LEFT_TOP, "GOLDLINK", big, GOLD);
    p.text(
        pos2(logo.left() + 18.0, logo.bottom() - 6.0),
        egui::Align2::LEFT_BOTTOM,
        "CO-OP HEARTGOLD",
        label(10.0),
        TEXT_DIM,
    );

    let status = Rect::from_min_max(
        pos2(logo.right() + gap, full.top() + m),
        pos2(full.right() - m, full.top() + m + top_h),
    );
    let main = Rect::from_min_max(
        pos2(full.left() + m, logo.bottom() + gap),
        pos2(full.right() - m, full.bottom() - m - bottom_h - gap),
    );
    let left_w = (main.width() * 0.42).clamp(300.0, 430.0);
    let left = Rect::from_min_max(main.min, pos2(main.left() + left_w, main.bottom()));
    let right = Rect::from_min_max(pos2(left.right() + gap, main.top()), main.max);
    let button = Rect::from_min_max(
        pos2(full.right() - m - 200.0, full.bottom() - m - bottom_h),
        pos2(full.right() - m, full.bottom() - m),
    );
    let tip = Rect::from_min_max(
        pos2(full.left() + m, button.top()),
        pos2(button.left() - gap, button.bottom()),
    );
    Chrome {
        full,
        status,
        left,
        right,
        main,
        tip,
        button,
    }
}

/// The tip line at the bottom: a hint, or the latest message (red when it's an error).
pub fn tip_line(ui: &mut Ui, rect: Rect, text: &str, error: bool) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, PANEL);
    border(p, rect, 3.0, if error { RED } else { EDGE });
    let inner = rect.shrink2(vec2(16.0, 8.0));
    let galley = p.layout(
        text.to_owned(),
        self::text(18.0),
        if error { RED } else { TEXT },
        inner.width(),
    );
    let at = pos2(inner.left(), inner.center().y - galley.size().y / 2.0);
    p.with_clip_rect(rect).galley(at, galley, TEXT);
}

/// The big button at the bottom right (PLAY, QUIT…), with an optional icon.
pub fn big_button(
    ui: &mut Ui,
    art: &Art,
    rect: Rect,
    icon: Option<&str>,
    label: &str,
    kind: Kind,
    enabled: bool,
) -> Response {
    let resp = ui
        .scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            button(ui, "", kind, rect.size(), enabled)
        })
        .inner;
    let p = ui.painter();
    let g = p.layout_no_wrap(label.to_owned(), FontId::new(28.0, caps_bold()), TEXT);
    let icon_w = if icon.is_some() { 30.0 } else { 0.0 };
    let start = rect.center().x - (g.size().x + icon_w) / 2.0;
    if let Some(t) = icon.and_then(|i| art.icon(i)) {
        let r = Rect::from_min_size(pos2(start, rect.center().y - 12.0), vec2(24.0, 24.0));
        p.image(
            t.id(),
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    let fg = match (kind, enabled) {
        (_, false) => TEXT_FAINT,
        (Kind::Primary, _) => rgb(0x2A, 0x1A, 0x00),
        _ => TEXT,
    };
    p.galley(
        pos2(start + icon_w, rect.center().y - g.size().y / 2.0 - 1.0),
        g,
        fg,
    );
    resp
}
