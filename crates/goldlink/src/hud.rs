//! The one-screen battle HUD (UC9 §3.2): the battle's touch-screen menus drawn by GoldLink in
//! a band along the bottom of the top screen, where the game's own text box was. The message
//! on the left; FIGHT, BAG, RUN and POKéMON on the right, or the four moves with their type and
//! PP. A click sends the touch a player would make on the game's own button, so the game's
//! rules stay in charge; the keys work as ever, and the game's key cursor shows as the
//! highlighted button.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, pos2, vec2,
};
use goldlink_bridge::battle_ui::{BattleHud, menu};

use crate::theme;

/// The DS screen's rows: the battle scene above, the game's text box below.
pub const SCENE_ROWS: f32 = 144.0;
const ROWS: f32 = 192.0;
const COLS: f32 = 256.0;

const NAVY: Color32 = Color32::from_rgb(0x14, 0x1A, 0x2E);
const NAVY_LIGHT: Color32 = Color32::from_rgb(0x22, 0x2B, 0x47);
const NAVY_EDGE: Color32 = Color32::from_rgb(0x4A, 0x56, 0x7E);
const INK: Color32 = Color32::from_rgb(0xF4, 0xF4, 0xF8);
const INK_DIM: Color32 = Color32::from_rgb(0xA8, 0xB0, 0xC8);
const CURSOR: Color32 = Color32::from_rgb(0xFF, 0x4A, 0x3A);

/// The game's own buttons (pret battle_input.c hitboxes): where a click on ours touches.
const TAP_FIGHT: (u16, u16) = (128, 84);
const TAP_BAG: (u16, u16) = (40, 168);
const TAP_RUN: (u16, u16) = (128, 172);
const TAP_POKEMON: (u16, u16) = (216, 168);
const TAP_MOVES: [(u16, u16); 4] = [(64, 52), (192, 52), (64, 116), (192, 116)];
const TAP_CANCEL: (u16, u16) = (128, 172);

/// The band: the frame's rows under the scene.
pub fn band(frame: Rect) -> Rect {
    let s = frame.height() / ROWS;
    Rect::from_min_max(pos2(frame.left(), frame.top() + SCENE_ROWS * s), frame.max)
}

/// Whether the HUD draws this menu (the others show the touch screen as a modal).
pub fn handles(hud: &BattleHud) -> bool {
    hud.waiting && matches!(hud.menu, menu::MAIN_INITIAL | menu::MAIN | menu::FIGHT)
}

/// The type colours, by the game's type number (hg-engine's order).
fn type_colour(t: u8) -> Color32 {
    let rgb = |r, g, b| Color32::from_rgb(r, g, b);
    match t {
        0 => rgb(0xA8, 0xA7, 0x7A),  // Normal
        1 => rgb(0xC2, 0x2E, 0x28),  // Fighting
        2 => rgb(0xA9, 0x8F, 0xF3),  // Flying
        3 => rgb(0xA3, 0x3E, 0xA1),  // Poison
        4 => rgb(0xE2, 0xBF, 0x65),  // Ground
        5 => rgb(0xB6, 0xA1, 0x36),  // Rock
        6 => rgb(0xA6, 0xB9, 0x1A),  // Bug
        7 => rgb(0x73, 0x57, 0x97),  // Ghost
        8 => rgb(0xB7, 0xB7, 0xCE),  // Steel
        9 => rgb(0xD6, 0x85, 0xAD),  // Fairy
        10 => rgb(0xEE, 0x81, 0x30), // Fire
        11 => rgb(0x63, 0x90, 0xF0), // Water
        12 => rgb(0x7A, 0xC7, 0x4C), // Grass
        13 => rgb(0xF7, 0xD0, 0x2C), // Electric
        14 => rgb(0xF9, 0x55, 0x87), // Psychic
        15 => rgb(0x96, 0xD9, 0xD6), // Ice
        16 => rgb(0x6F, 0x35, 0xFC), // Dragon
        17 => rgb(0x70, 0x57, 0x46), // Dark
        _ => rgb(0x68, 0xA0, 0x90),
    }
}

/// A button's outline: a long hexagon, its ends pointed like the battle's own boxes.
fn chevron(r: Rect) -> Vec<Pos2> {
    let tip = (r.height() * 0.35).min(r.width() * 0.2);
    vec![
        pos2(r.left() + tip, r.top()),
        pos2(r.right() - tip, r.top()),
        pos2(r.right(), r.center().y),
        pos2(r.right() - tip, r.bottom()),
        pos2(r.left() + tip, r.bottom()),
        pos2(r.left(), r.center().y),
    ]
}

/// The game's key cursor: red brackets on the corners, as on the touch screen.
fn brackets(p: &egui::Painter, r: Rect, s: f32) {
    let len = 4.0 * s;
    let stroke = Stroke::new((1.2 * s).max(2.0), CURSOR);
    let r = r.expand(1.5 * s);
    for (corner, dx, dy) in [
        (r.left_top(), 1.0, 1.0),
        (r.right_top(), -1.0, 1.0),
        (r.left_bottom(), 1.0, -1.0),
        (r.right_bottom(), -1.0, -1.0),
    ] {
        p.line_segment([corner, corner + vec2(len * dx, 0.0)], stroke);
        p.line_segment([corner, corner + vec2(0.0, len * dy)], stroke);
    }
}

/// Pixel fonts are crisp only at whole multiples of their design pixel: Pixelify Sans draws
/// on a grid of 1/11 em, Silkscreen on 1/8 em. Off the grid, rows merge ("5" reads as "8").
fn snap(px: f32, grid: f32) -> f32 {
    ((px / grid).round() * grid).max(grid)
}

/// The text font (Pixelify Sans) at `px` window points, snapped to its grid.
fn text_font(px: f32) -> FontId {
    theme::text(snap(px, 11.0))
}

/// The capitals font (Silkscreen bold) at `px` window points, snapped to its grid.
fn caps_font(px: f32) -> FontId {
    FontId::new(snap(px, 8.0), theme::caps_bold())
}

struct Ctx<'a> {
    ui: &'a mut egui::Ui,
    /// Window points per DS pixel.
    s: f32,
    /// The band's top-left, in window points (DS row 144, column 0).
    origin: Pos2,
    tap: Option<(u16, u16)>,
}

impl Ctx<'_> {
    /// A rectangle in DS pixels (columns, rows from the band's top) → window points.
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(
            self.origin + vec2(x0, y0) * self.s,
            self.origin + vec2(x1, y1) * self.s,
        )
    }

    fn font(&self, ds_px: f32) -> FontId {
        text_font(ds_px * self.s)
    }

    fn caps(&self, ds_px: f32) -> FontId {
        caps_font(ds_px * self.s)
    }

    /// A coloured command button; a click taps the game's.
    #[allow(clippy::too_many_arguments)]
    fn command(
        &mut self,
        id: &str,
        r: Rect,
        label: &str,
        fill: Color32,
        edge: Color32,
        size: f32,
        selected: bool,
        tap: (u16, u16),
    ) {
        let response = self
            .ui
            .interact(r, egui::Id::new(("goldlink-hud", id)), egui::Sense::CLICK);
        let fill = if response.hovered() {
            fill.lerp_to_gamma(Color32::WHITE, 0.15)
        } else {
            fill
        };
        let p = self.ui.painter();
        p.add(egui::Shape::convex_polygon(
            chevron(r),
            fill,
            Stroke::new((0.8 * self.s).max(1.5), edge),
        ));
        // A lighter top half, as the game's buttons have.
        let top = Rect::from_min_max(r.min, pos2(r.max.x, r.center().y));
        p.add(egui::Shape::convex_polygon(
            chevron(top.shrink2(vec2(r.height() * 0.17, 0.0))),
            Color32::from_white_alpha(28),
            Stroke::NONE,
        ));
        p.text(
            r.center() + vec2(0.0, 0.3 * self.s),
            egui::Align2::CENTER_CENTER,
            label,
            self.caps(size),
            INK,
        );
        if selected {
            brackets(p, r, self.s);
        }
        if response.clicked() {
            self.tap = Some(tap);
        }
    }
}

/// The HUD's buttons take clicks but never the keyboard's focus: Enter is the game's Start, and
/// Tab shows the touch screen (`Sense::CLICK`, not `Sense::click()`, which is focusable).
///
/// The band for a menu the HUD draws. Returns the touch to send when a button was clicked.
pub fn draw(ui: &mut egui::Ui, frame: Rect, hud: &BattleHud) -> Option<(u16, u16)> {
    let s = frame.width() / COLS;
    let band = band(frame);
    let mut c = Ctx {
        ui,
        s,
        origin: band.min,
        tap: None,
    };
    c.ui.painter().rect_filled(band, CornerRadius::ZERO, NAVY);
    c.ui.painter().line_segment(
        [band.left_top(), band.right_top()],
        Stroke::new((1.0 * s).max(2.0), CURSOR.gamma_multiply(0.8)),
    );
    match hud.menu {
        menu::FIGHT => fight(&mut c, hud),
        _ => commands(&mut c, hud),
    }
    c.tap
}

fn commands(c: &mut Ctx<'_>, hud: &BattleHud) {
    // The message, left.
    let text = c.rect(4.0, 4.0, 108.0, 44.0);
    message_box(c, text, &hud.message, 9.0);
    let sel = |y: i8, x: i8| hud.cursor == Some((y, x));
    let fight = c.rect(112.0, 4.0, 252.0, 24.0);
    c.command(
        "fight",
        fight,
        "FIGHT",
        Color32::from_rgb(0xE0, 0x3C, 0x31),
        Color32::from_rgb(0x7A, 0x14, 0x10),
        10.0,
        hud.cursor.is_some_and(|(y, _)| y == 0),
        TAP_FIGHT,
    );
    let row = [
        (
            "bag",
            "BAG",
            Color32::from_rgb(0xE3, 0x93, 0x2A),
            Color32::from_rgb(0x7A, 0x4A, 0x0C),
            sel(1, 0),
            TAP_BAG,
        ),
        (
            "run",
            "RUN",
            Color32::from_rgb(0x2F, 0x7F, 0xE0),
            Color32::from_rgb(0x12, 0x3E, 0x7A),
            sel(1, 1),
            TAP_RUN,
        ),
        (
            "pokemon",
            "POKéMON",
            Color32::from_rgb(0x3E, 0xA6, 0x48),
            Color32::from_rgb(0x15, 0x5A, 0x1C),
            sel(1, 2),
            TAP_POKEMON,
        ),
    ];
    let w = (252.0 - 112.0 - 2.0 * 3.0) / 3.0;
    for (i, (id, label, fill, edge, selected, tap)) in row.into_iter().enumerate() {
        let x = 112.0 + i as f32 * (w + 3.0);
        let r = c.rect(x, 27.0, x + w, 44.0);
        c.command(id, r, label, fill, edge, 6.5, selected, tap);
    }
}

fn fight(c: &mut Ctx<'_>, hud: &BattleHud) {
    let cols = [(4.0, 98.0), (101.0, 195.0)];
    let rows = [(4.0, 23.5), (25.5, 44.0)];
    let selected = hud
        .cursor
        .and_then(|(y, x)| (y < 2).then_some((y * 2 + x) as usize));
    for i in 0..4 {
        let (x0, x1) = cols[i % 2];
        let (y0, y1) = rows[i / 2];
        let r = c.rect(x0, y0, x1, y1);
        let id = egui::Id::new(("goldlink-hud-move", i));
        let Some(m) = &hud.moves[i] else {
            c.ui.painter().add(egui::Shape::convex_polygon(
                chevron(r),
                NAVY_LIGHT,
                Stroke::new((0.6 * c.s).max(1.0), NAVY_EDGE),
            ));
            c.ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "–",
                c.font(8.0),
                INK_DIM,
            );
            continue;
        };
        let response = c.ui.interact(r, id, egui::Sense::CLICK);
        let colour = type_colour(m.type_id);
        let fill = if response.hovered() {
            NAVY_LIGHT.lerp_to_gamma(colour, 0.25)
        } else {
            NAVY_LIGHT
        };
        let s = c.s;
        let p = c.ui.painter();
        p.add(egui::Shape::convex_polygon(
            chevron(r),
            fill,
            Stroke::new((0.8 * s).max(1.5), colour),
        ));
        let pad = r.height() * 0.42;
        p.text(
            pos2(r.left() + pad, r.top() + 1.2 * s),
            egui::Align2::LEFT_TOP,
            &m.name,
            text_font(8.0 * s),
            INK,
        );
        // The type badge, bottom left.
        let badge_text = m.type_name.to_uppercase();
        let galley = p.layout_no_wrap(badge_text, caps_font(5.0 * s), INK);
        let badge = Rect::from_min_size(
            pos2(
                r.left() + pad,
                r.bottom() - 1.4 * s - galley.size().y - 1.0 * s,
            ),
            galley.size() + vec2(4.0 * s, 1.0 * s),
        );
        p.rect_filled(
            badge,
            CornerRadius::same((1.5 * s) as u8),
            colour.gamma_multiply(0.85),
        );
        p.galley(badge.min + vec2(2.0 * s, 0.5 * s), galley, INK);
        let pp_colour = if m.pp == 0 {
            CURSOR
        } else if u32::from(m.pp) * 4 <= u32::from(m.max) {
            theme::AMBER
        } else {
            INK_DIM
        };
        p.text(
            pos2(r.right() - pad, r.bottom() - 1.6 * s),
            egui::Align2::RIGHT_BOTTOM,
            format!("PP {}/{}", m.pp, m.max),
            text_font(6.5 * s),
            pp_colour,
        );
        if selected == Some(i) {
            brackets(p, r, s);
        }
        if response.clicked() {
            c.tap = Some(TAP_MOVES[i]);
        }
    }
    // The right column: the highlighted move's type, and back.
    let info = c.rect(199.0, 4.0, 252.0, 23.5);
    let shown = selected.and_then(|i| hud.moves[i].as_ref());
    let p = c.ui.painter();
    p.rect_filled(info, CornerRadius::same((2.0 * c.s) as u8), NAVY_LIGHT);
    p.rect_stroke(
        info,
        CornerRadius::same((2.0 * c.s) as u8),
        Stroke::new((0.6 * c.s).max(1.0), NAVY_EDGE),
        StrokeKind::Inside,
    );
    let line = match shown {
        Some(m) => format!("{}\nPP {}/{}", m.type_name, m.pp, m.max),
        None => "Choose a\nmove".to_owned(),
    };
    p.text(
        info.center(),
        egui::Align2::CENTER_CENTER,
        line,
        c.font(6.5),
        INK_DIM,
    );
    let back = c.rect(199.0, 25.5, 252.0, 44.0);
    c.command(
        "cancel",
        back,
        "BACK",
        NAVY_LIGHT,
        NAVY_EDGE,
        7.0,
        hud.cursor.is_some_and(|(y, _)| y >= 2),
        TAP_CANCEL,
    );
}

fn message_box(c: &mut Ctx<'_>, r: Rect, text: &str, size: f32) {
    let s = c.s;
    let p = c.ui.painter();
    p.rect_filled(r, CornerRadius::same((2.0 * s) as u8), NAVY_LIGHT);
    p.rect_stroke(
        r,
        CornerRadius::same((2.0 * s) as u8),
        Stroke::new((0.6 * s).max(1.0), NAVY_EDGE),
        StrokeKind::Inside,
    );
    let inner = r.shrink(4.0 * s);
    let galley = p.layout(text.to_owned(), c.font(size), INK, inner.width());
    p.galley(
        pos2(inner.left(), inner.center().y - galley.size().y / 2.0),
        galley,
        INK,
    );
}

/// The band when no menu waits: the game's own text box, its words re-inked white on GoldLink's
/// navy (`textbox` is the game's text box rows, already recoloured; see [`recolour_textbox`]).
pub fn draw_text_band(ui: &mut egui::Ui, frame: Rect, textbox: &egui::TextureHandle) {
    let band = band(frame);
    let s = frame.width() / COLS;
    let p = ui.painter();
    p.rect_filled(band, CornerRadius::ZERO, NAVY);
    p.line_segment(
        [band.left_top(), band.right_top()],
        Stroke::new((1.0 * s).max(2.0), CURSOR.gamma_multiply(0.8)),
    );
    let inner = band.shrink2(vec2(4.0 * s, 3.0 * s));
    p.rect_filled(inner, CornerRadius::same((2.0 * s) as u8), NAVY_LIGHT);
    p.rect_stroke(
        inner,
        CornerRadius::same((2.0 * s) as u8),
        Stroke::new((0.6 * s).max(1.0), NAVY_EDGE),
        StrokeKind::Inside,
    );
    p.image(
        textbox.id(),
        band,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// The game's text box rows (144 to 191 of the top screen) with only its words kept: the text
/// colour becomes white, its shadow a dark navy, everything else transparent. The game prints
/// battle messages in one palette (measured: text 515159, shadow A2A2AA on FBFBFB; the box's
/// bottom border is the text colour too, so only the box's inside is kept).
pub fn recolour_textbox(top_screen_rows: &[u32]) -> egui::ColorImage {
    const TEXT: u32 = 0x51_5159;
    const SHADOW: u32 = 0xA2_A2AA;
    let rows = (ROWS - SCENE_ROWS) as usize;
    let mut pixels = vec![Color32::TRANSPARENT; 256 * rows];
    // The box's inside (rows 150 to 184, columns 8 to 247): its borders share the text colour.
    for y in 6..41.min(rows) {
        for x in 8..248 {
            let Some(&px) = top_screen_rows.get(y * 256 + x) else {
                continue;
            };
            pixels[y * 256 + x] = match px & 0xFF_FFFF {
                TEXT => INK,
                SHADOW => Color32::from_rgb(0x0A, 0x0E, 0x1A),
                _ => Color32::TRANSPARENT,
            };
        }
    }
    egui::ColorImage {
        size: [256, rows],
        pixels,
    }
}
