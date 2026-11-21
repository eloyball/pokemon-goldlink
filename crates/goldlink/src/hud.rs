//! The one-screen battle HUD (UC9 §3.2): the battle's touch-screen menus drawn by GoldLink in
//! a band along the bottom of the top screen, where the game's own text box was. The message
//! on the left; FIGHT, BAG, RUN and POKéMON on the right, or the four moves with their type and
//! PP. A click sends the touch a player would make on the game's own button, so the game's
//! rules stay in charge; the keys work as ever, and the game's key cursor shows as the
//! highlighted button.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, pos2, vec2,
};
use goldlink_bridge::battle_ui::{Ball, BattleHud, menu};
use goldlink_bridge::types::Effect;

use crate::theme;

/// The DS screen's rows: the battle scene above, the game's text box below.
pub const SCENE_ROWS: f32 = 144.0;
const ROWS: f32 = 192.0;

/// Warm greys, so the band sits with HeartGold's yellows, oranges and reds (the user's choice
/// over the first navy), with a gold line along its top.
pub const BAND: Color32 = Color32::from_rgb(0x2A, 0x26, 0x24);
const PANEL: Color32 = Color32::from_rgb(0x3B, 0x36, 0x33);
const EDGE: Color32 = Color32::from_rgb(0x77, 0x6C, 0x63);
const ACCENT: Color32 = Color32::from_rgb(0xD8, 0x9A, 0x3A);
const INK: Color32 = Color32::from_rgb(0xF4, 0xF4, 0xF8);
const INK_DIM: Color32 = Color32::from_rgb(0xCF, 0xC4, 0xB8);
const CURSOR: Color32 = Color32::from_rgb(0xFF, 0x4A, 0x3A);
/// The game's own button colours (measured on its touch screen): a bright top half, a darker
/// bottom half, a dark grey edge.
const FIGHT_TOP: Color32 = Color32::from_rgb(0xEB, 0x38, 0x38);
const FIGHT_BOTTOM: Color32 = Color32::from_rgb(0x8A, 0x41, 0x41);
const BAG_TOP: Color32 = Color32::from_rgb(0xE3, 0x9A, 0x28);
const BAG_BOTTOM: Color32 = Color32::from_rgb(0x8A, 0x69, 0x38);
const RUN_TOP: Color32 = Color32::from_rgb(0x28, 0x92, 0xCB);
const RUN_BOTTOM: Color32 = Color32::from_rgb(0x41, 0x59, 0x71);
const POKEMON_TOP: Color32 = Color32::from_rgb(0x59, 0xB2, 0x28);
const POKEMON_BOTTOM: Color32 = Color32::from_rgb(0x51, 0x71, 0x41);
const BUTTON_EDGE: Color32 = Color32::from_rgb(0x41, 0x41, 0x41);
/// A neutral button's top half (BACK).
const PANEL_TOP: Color32 = Color32::from_rgb(0x55, 0x4E, 0x4A);
/// The labels' outline.
const OUTLINE: Color32 = Color32::from_rgb(0x1A, 0x17, 0x16);

/// The game's own buttons (pret battle_input.c hitboxes): where a click on ours touches.
const TAP_FIGHT: (u16, u16) = (128, 84);
const TAP_BAG: (u16, u16) = (40, 168);
const TAP_RUN: (u16, u16) = (128, 172);
const TAP_POKEMON: (u16, u16) = (216, 168);
const TAP_MOVES: [(u16, u16); 4] = [(64, 52), (192, 52), (64, 116), (192, 116)];
const TAP_CANCEL: (u16, u16) = (128, 172);
/// hg-engine's MEGA button in the fight menu (src/battle/battle_input.c SkillMenuTouchData).
const TAP_MEGA: (u16, u16) = (212, 172);
/// MEGA lit (the touch screen's icon turns this yellow).
const MEGA_LIT_TOP: Color32 = Color32::from_rgb(0xF3, 0xF3, 0x61);
const MEGA_LIT_BOTTOM: Color32 = Color32::from_rgb(0xA8, 0x9A, 0x30);

/// The MEGA button the HUD draws: lit or not, and the game's Mega symbol as a texture.
#[derive(Debug, Clone, Copy)]
pub struct Mega {
    pub lit: bool,
    pub icon: Option<egui::TextureId>,
}

/// The game's Mega symbol, cut from its MEGA button on the touch screen (`bottom`: its pixels,
/// 0x00RRGGBB): the 20 x 20 square around (213, 173), with the button's blues behind it made
/// clear. Grey when off, in colour when lit, as the game draws it.
pub fn mega_icon(bottom: &[u32]) -> egui::ColorImage {
    const X0: usize = 203;
    const Y0: usize = 163;
    const N: usize = 20;
    let mut pixels = vec![Color32::TRANSPARENT; N * N];
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - 9.5, y as f32 - 9.5);
            if dx * dx + dy * dy > 10.2 * 10.2 {
                continue;
            }
            let p = bottom.get((Y0 + y) * 256 + X0 + x).copied().unwrap_or(0) & 0xFF_FFFF;
            // CANCEL's button colours, around the symbol's dark ring.
            if matches!(p, 0x28_92CB | 0x41_5971 | 0x30_79AA | 0x38_698A) {
                continue;
            }
            pixels[y * N + x] = Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8);
        }
    }
    egui::ColorImage {
        size: [N, N],
        pixels,
    }
}

/// hg-engine's MEGA button as the touch screen shows it (`bottom`: the touch screen's pixels,
/// 0x00RRGGBB): None when it isn't offered (CANCEL then runs to the right edge), else whether
/// it's lit. Measured on the running game: its dark edge at x 178 (CANCEL's blue there
/// without it); its icon's middle (212, 172) grey when off, yellow when lit.
pub fn mega_state(bottom: &[u32]) -> Option<bool> {
    let px = |x: usize, y: usize| bottom.get(y * 256 + x).map_or(0, |p| p & 0xFF_FFFF);
    let edge = (162..186).filter(|&y| px(178, y) == 0x41_4141).count();
    if edge < 12 {
        return None;
    }
    let lit = (204..222)
        .flat_map(|x| (164..182).map(move |y| (x, y)))
        .any(|(x, y)| {
            let p = px(x, y);
            let (r, g, b) = (p >> 16, (p >> 8) & 0xFF, p & 0xFF);
            r > 200 && g > 200 && b < 150
        });
    Some(lit)
}

/// The two-option menus' buttons (top, bottom).
const TAP_OPTIONS: [(u16, u16); 2] = [(128, 68), (128, 140)];
/// The target menu's buttons: player left, opponent right, player right, opponent left.
const TAP_TARGETS: [(u16, u16); 4] = [(60, 116), (195, 44), (195, 116), (60, 44)];

/// The band: the frame's rows under the scene.
pub fn band(frame: Rect) -> Rect {
    let s = frame.height() / ROWS;
    Rect::from_min_max(pos2(frame.left(), frame.top() + SCENE_ROWS * s), frame.max)
}

/// Whether the HUD draws this menu (the others show the touch screen as a modal).
pub fn handles(hud: &BattleHud) -> bool {
    hud.waiting
        && (menu::COMMANDS.contains(&hud.menu)
            || matches!(hud.menu, menu::FIGHT | menu::TARGET)
            || hud.options.is_some())
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

/// White text with a dark outline all round (eight directions, about a DS pixel wide).
fn outlined_text(
    p: &egui::Painter,
    at: Pos2,
    align: egui::Align2,
    text: &str,
    font: FontId,
    s: f32,
) {
    let d = (0.7 * s).max(1.0);
    for (dx, dy) in [
        (-d, -d),
        (0.0, -d),
        (d, -d),
        (-d, 0.0),
        (d, 0.0),
        (-d, d),
        (0.0, d),
        (d, d),
    ] {
        p.text(at + vec2(dx, dy), align, text, font.clone(), OUTLINE);
    }
    p.text(at, align, text, font, INK);
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
    /// The band's width in DS pixels (256, or wider with the wider picture).
    w: f32,
    tap: Option<(u16, u16)>,
}

/// The room for a label between a button's pointed ends.
fn label_room(r: Rect) -> f32 {
    r.width() - r.height() * 0.6
}

impl Ctx<'_> {
    /// The largest capitals size (from `ds_px` down, on the font's grid) at which every label
    /// fits its button: a row of buttons shares one size.
    fn fit_caps(&self, labels: &[(&str, Rect)], ds_px: f32) -> FontId {
        let mut font = self.caps(ds_px);
        let p = self.ui.painter();
        while font.size > 8.0
            && labels.iter().any(|(l, r)| {
                p.layout_no_wrap((*l).to_owned(), font.clone(), INK)
                    .size()
                    .x
                    > label_room(*r)
            })
        {
            font.size -= 8.0;
        }
        font
    }

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

    /// A coloured command button; a click taps the game's. Its colours are the game's own (red
    /// FIGHT, orange BAG, blue RUN, green POKéMON), muted to sit on the grey band.
    #[allow(clippy::too_many_arguments)]
    fn command(
        &mut self,
        id: &str,
        r: Rect,
        label: &str,
        top: Color32,
        bottom: Color32,
        font: FontId,
        selected: bool,
        tap: (u16, u16),
    ) {
        self.button(id, r, label, top, bottom, font, selected, tap, None);
    }

    /// A command button with an icon left of its label (MEGA: the game's own Mega symbol).
    #[allow(clippy::too_many_arguments)]
    fn button(
        &mut self,
        id: &str,
        r: Rect,
        label: &str,
        top: Color32,
        bottom: Color32,
        font: FontId,
        selected: bool,
        tap: (u16, u16),
        icon: Option<egui::TextureId>,
    ) {
        let response = self
            .ui
            .interact(r, egui::Id::new(("goldlink-hud", id)), egui::Sense::CLICK);
        let (top, bottom) = if response.hovered() {
            (
                top.lerp_to_gamma(Color32::WHITE, 0.15),
                bottom.lerp_to_gamma(Color32::WHITE, 0.15),
            )
        } else {
            (top, bottom)
        };
        let p = self.ui.painter();
        // The game's two tones: the darker bottom under the bright top half, a dark grey edge.
        p.add(egui::Shape::convex_polygon(
            chevron(r),
            bottom,
            Stroke::NONE,
        ));
        let tip = (r.height() * 0.35).min(r.width() * 0.2);
        p.add(egui::Shape::convex_polygon(
            vec![
                pos2(r.left() + tip, r.top()),
                pos2(r.right() - tip, r.top()),
                pos2(r.right(), r.center().y),
                pos2(r.left(), r.center().y),
            ],
            top,
            Stroke::NONE,
        ));
        p.add(egui::Shape::closed_line(
            chevron(r),
            Stroke::new((0.8 * self.s).max(1.5), BUTTON_EDGE),
        ));
        // The label in white with a dark outline all round, legible on either half; an icon
        // goes before it, the two centred together.
        let mut at = r.center() + vec2(0.0, 0.3 * self.s);
        if let Some(icon) = icon {
            let gap = 3.0 * self.s;
            let label_w = p
                .layout_no_wrap(label.to_owned(), font.clone(), INK)
                .size()
                .x;
            // A round symbol can reach half into the slanted end that text keeps clear of.
            let side = (r.height() * 0.72)
                .min(label_room(r) + r.height() * 0.15 - label_w - gap)
                .max(0.0);
            let left = r.center().x - (side + gap + label_w) / 2.0;
            let icon_rect =
                Rect::from_min_size(pos2(left, r.center().y - side / 2.0), vec2(side, side));
            p.image(
                icon,
                icon_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            at.x = left + side + gap + label_w / 2.0;
        }
        outlined_text(p, at, egui::Align2::CENTER_CENTER, label, font, self.s);
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
pub fn draw(
    ui: &mut egui::Ui,
    frame: Rect,
    hud: &BattleHud,
    mega: Option<Mega>,
) -> Option<(u16, u16)> {
    let s = frame.height() / ROWS;
    let band = band(frame);
    let mut c = Ctx {
        ui,
        s,
        origin: band.min,
        w: frame.width() / s,
        tap: None,
    };
    c.ui.painter().rect_filled(band, CornerRadius::ZERO, BAND);
    c.ui.painter().line_segment(
        [band.left_top(), band.right_top()],
        Stroke::new((1.0 * s).max(2.0), ACCENT),
    );
    if let Some(balls) = &hud.party {
        party_balls(c.ui.painter(), band.left_top(), balls, s);
    }
    match (hud.menu, &hud.options) {
        (menu::FIGHT, _) => fight(&mut c, hud, mega),
        (menu::TARGET, _) => targets(&mut c, hud),
        (_, Some((top, bottom))) => two_options(&mut c, hud, top, bottom),
        _ => commands(&mut c, hud),
    }
    c.tap
}

fn commands(c: &mut Ctx<'_>, hud: &BattleHud) {
    // The message, left.
    // The message on the left two fifths, the buttons on the rest.
    let w = c.w;
    let bx = (w * 0.406).round();
    let text = c.rect(4.0, 4.0, bx - 4.0, 44.0);
    message_box(c, text, &hud.message, 9.0);
    let sel = |y: i8, x: i8| hud.cursor == Some((y, x));
    let fight = c.rect(bx, 4.0, w - 4.0, 24.0);
    let fight_font = c.fit_caps(&[("FIGHT", fight)], 10.0);
    c.command(
        "fight",
        fight,
        "FIGHT",
        FIGHT_TOP,
        FIGHT_BOTTOM,
        fight_font,
        hud.cursor.is_some_and(|(y, _)| y == 0),
        TAP_FIGHT,
    );
    let row = [
        ("bag", "BAG", BAG_TOP, BAG_BOTTOM, sel(1, 0), TAP_BAG),
        (
            "run",
            if hud.run_is_back { "BACK" } else { "RUN" },
            RUN_TOP,
            RUN_BOTTOM,
            sel(1, 1),
            TAP_RUN,
        ),
        (
            "pokemon",
            "POKéMON",
            POKEMON_TOP,
            POKEMON_BOTTOM,
            sel(1, 2),
            TAP_POKEMON,
        ),
    ];
    let bw = (w - 4.0 - bx - 2.0 * 3.0) / 3.0;
    let rects: Vec<Rect> = (0..3)
        .map(|i| {
            let x = bx + i as f32 * (bw + 3.0);
            c.rect(x, 27.0, x + bw, 44.0)
        })
        .collect();
    let labels: Vec<(&str, Rect)> = row.iter().zip(&rects).map(|(b, r)| (b.1, *r)).collect();
    let font = c.fit_caps(&labels, 7.5);
    for ((id, label, fill, edge, selected, tap), r) in row.into_iter().zip(rects) {
        c.command(id, r, label, fill, edge, font.clone(), selected, tap);
    }
}

fn fight(c: &mut Ctx<'_>, hud: &BattleHud, mega: Option<Mega>) {
    // Two columns of moves, then a column for the highlighted move and BACK.
    let w = c.w;
    let info_x = w - 57.0;
    let col = (info_x - 4.0 - 4.0 - 3.0) / 2.0;
    let cols = [(4.0, 4.0 + col), (7.0 + col, info_x - 4.0)];
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
                PANEL,
                Stroke::new((0.6 * c.s).max(1.0), EDGE),
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
            PANEL.lerp_to_gamma(colour, 0.25)
        } else {
            PANEL
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
        let pp_text = format!("PP {}/{}", m.pp, m.max);
        // How it hits the foe by type (the newer games' hint, the user's v0.7.1 play test),
        // between the badge and the PP, shortened when the button is narrow.
        if let Some((long, short, colour)) = m.effect.and_then(effect_label) {
            let pp_left = r.right()
                - pad
                - p.layout_no_wrap(pp_text.clone(), caps_font(5.5 * s), INK)
                    .size()
                    .x;
            let room = pp_left - badge.right() - 3.0 * s;
            let font = caps_font(5.0 * s);
            let fitting = [long, short]
                .into_iter()
                .map(|t| p.layout_no_wrap(t.to_owned(), font.clone(), colour))
                .find(|g| g.size().x <= room);
            if let Some(g) = fitting {
                let at = pos2(badge.right() + 2.0 * s, badge.center().y - g.size().y / 2.0);
                p.galley(at, g, colour);
            }
        }
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
            pp_text,
            caps_font(5.5 * s),
            pp_colour,
        );
        if selected == Some(i) {
            brackets(p, r, s);
        }
        if response.clicked() {
            c.tap = Some(TAP_MOVES[i]);
        }
    }
    // The right column: MEGA when hg-engine offers it (lit when chosen), else the highlighted
    // move's power and accuracy; then BACK. With MEGA, the key cursor's last row is BACK, MEGA.
    let info = c.rect(info_x, 4.0, w - 4.0, 23.5);
    if let Some(Mega { lit, icon }) = mega {
        // MEGA's label as large as BACK's; the symbol beside it takes the room that's left.
        let back = c.rect(info_x, 25.5, w - 4.0, 44.0);
        let font = c.fit_caps(&[("BACK", back)], 7.0);
        let (top, bottom) = if lit {
            (MEGA_LIT_TOP, MEGA_LIT_BOTTOM)
        } else {
            (PANEL_TOP, PANEL)
        };
        c.button(
            "mega",
            info,
            "MEGA",
            top,
            bottom,
            font,
            hud.cursor == Some((2, 1)),
            TAP_MEGA,
            icon,
        );
        let back = c.rect(info_x, 25.5, w - 4.0, 44.0);
        let back_font = c.fit_caps(&[("BACK", back)], 7.0);
        c.command(
            "cancel",
            back,
            "BACK",
            PANEL_TOP,
            PANEL,
            back_font,
            hud.cursor == Some((2, 0)),
            TAP_CANCEL,
        );
        return;
    }
    let shown = selected.and_then(|i| hud.moves[i].as_ref());
    let p = c.ui.painter();
    p.rect_filled(info, CornerRadius::same((2.0 * c.s) as u8), PANEL);
    p.rect_stroke(
        info,
        CornerRadius::same((2.0 * c.s) as u8),
        Stroke::new((0.6 * c.s).max(1.0), EDGE),
        StrokeKind::Inside,
    );
    let line = match shown {
        Some(m) => {
            let num = |v: u8| {
                if v == 0 {
                    "—".to_owned()
                } else {
                    v.to_string()
                }
            };
            format!("POWER {}\nACC. {}", num(m.power), num(m.accuracy))
        }
        None => "CHOOSE\nA MOVE".to_owned(),
    };
    p.text(
        info.center(),
        egui::Align2::CENTER_CENTER,
        line,
        c.caps(5.5),
        INK_DIM,
    );
    let back = c.rect(info_x, 25.5, w - 4.0, 44.0);
    let back_font = c.fit_caps(&[("BACK", back)], 7.0);
    c.command(
        "cancel",
        back,
        "BACK",
        PANEL_TOP,
        PANEL,
        back_font,
        hud.cursor.is_some_and(|(y, _)| y >= 2),
        TAP_CANCEL,
    );
}

/// A move's type matchup as the move button says it: in full, shortened, its colour. Nothing for
/// a plain hit.
fn effect_label(e: Effect) -> Option<(&'static str, &'static str, Color32)> {
    match e {
        Effect::Super => Some(("SUPER EFFECTIVE", "SUPER EFF.", theme::GREEN_DOT)),
        Effect::NotVery => Some(("NOT VERY EFFECTIVE", "NOT VERY EFF.", theme::AMBER)),
        Effect::None => Some(("NO EFFECT", "NO EFFECT", INK_DIM)),
        Effect::Normal => None,
    }
}

/// Yes / no and the other two-choice menus: the message, then the two choices stacked.
fn two_options(c: &mut Ctx<'_>, hud: &BattleHud, top: &str, bottom: &str) {
    let w = c.w;
    // The buttons as wide as their labels need (at a small size), the message the rest: these
    // prompts' messages are long ("… is about to send in …. Will you change Pokémon?" ran past
    // its box, the user's v0.7 play test), their labels mostly short.
    let need = {
        let font = c.caps(6.5);
        let p = c.ui.painter();
        [top, bottom]
            .iter()
            .map(|l| {
                p.layout_no_wrap((*l).to_owned(), font.clone(), INK)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max)
            / c.s
    };
    // label_room keeps 0.6 of the button's height (19 rows) clear for its pointed ends.
    let bw = (need + 0.6 * 19.0 + 6.0).clamp(w * 0.32, w * 0.594);
    let bx = (w - 4.0 - bw).round();
    let text = c.rect(4.0, 4.0, bx - 4.0, 44.0);
    message_box(c, text, &hud.message, 9.0);
    let rects = [
        c.rect(bx, 4.0, w - 4.0, 23.0),
        c.rect(bx, 25.0, w - 4.0, 44.0),
    ];
    let labels = [(top, rects[0]), (bottom, rects[1])];
    let font = c.fit_caps(&labels, 7.5);
    let colours = [(POKEMON_TOP, POKEMON_BOTTOM), (PANEL_TOP, PANEL)];
    for (i, ((label, r), (fill, edge))) in labels.into_iter().zip(colours).enumerate() {
        let selected = hud.cursor.is_some_and(|(y, _)| y as usize == i);
        c.command(
            ["option-top", "option-bottom"][i],
            r,
            label,
            fill,
            edge,
            font.clone(),
            selected,
            TAP_OPTIONS[i],
        );
    }
}

/// The target menu (double battles): the opponents on top, the player's side below, as on the
/// touch screen; battlers the move can't reach aren't offered.
fn targets(c: &mut Ctx<'_>, hud: &BattleHud) {
    let w = c.w;
    let info_x = w - 57.0;
    let col = (info_x - 4.0 - 4.0 - 3.0) / 2.0;
    let cols = [(4.0, 4.0 + col), (7.0 + col, info_x - 4.0)];
    let rows = [(4.0, 23.5), (25.5, 44.0)];
    // Grid position → button: opponent left, opponent right; player left, player right.
    let grid = [3usize, 1, 0, 2];
    for (cell, &battler) in grid.iter().enumerate() {
        let (x0, x1) = cols[cell % 2];
        let (y0, y1) = rows[cell / 2];
        let r = c.rect(x0, y0, x1, y1);
        let Some(t) = &hud.targets[battler] else {
            c.ui.painter().add(egui::Shape::convex_polygon(
                chevron(r),
                PANEL,
                Stroke::new((0.6 * c.s).max(1.0), EDGE),
            ));
            continue;
        };
        let opponent = battler % 2 == 1;
        let (fill, edge) = if opponent {
            (FIGHT_TOP, FIGHT_BOTTOM)
        } else {
            (RUN_TOP, RUN_BOTTOM)
        };
        let label = format!("{}{}", t.name, gender_mark(t.gender));
        let font = c.fit_caps(&[(&label, r)], 7.0);
        let selected = hud
            .cursor
            .is_some_and(|(y, x)| (y as usize) * 2 + x as usize == cell);
        c.command(
            ["target-0", "target-1", "target-2", "target-3"][cell],
            r,
            &label,
            fill,
            edge,
            font,
            selected,
            TAP_TARGETS[battler],
        );
    }
    let info = c.rect(info_x, 4.0, w - 4.0, 23.5);
    let p = c.ui.painter();
    p.rect_filled(info, CornerRadius::same((2.0 * c.s) as u8), PANEL);
    p.text(
        info.center(),
        egui::Align2::CENTER_CENTER,
        "CHOOSE\nA TARGET",
        c.caps(5.5),
        INK_DIM,
    );
    let back = c.rect(info_x, 25.5, w - 4.0, 44.0);
    let back_font = c.fit_caps(&[("BACK", back)], 7.0);
    c.command(
        "target-back",
        back,
        "BACK",
        PANEL_TOP,
        PANEL,
        back_font,
        hud.cursor.is_some_and(|(y, _)| y >= 2),
        TAP_CANCEL,
    );
}

fn gender_mark(g: u8) -> &'static str {
    match g {
        0 => " ♂",
        1 => " ♀",
        _ => "",
    }
}

fn message_box(c: &mut Ctx<'_>, r: Rect, text: &str, size: f32) {
    let s = c.s;
    let p = c.ui.painter();
    p.rect_filled(r, CornerRadius::same((2.0 * s) as u8), PANEL);
    p.rect_stroke(
        r,
        CornerRadius::same((2.0 * s) as u8),
        Stroke::new((0.6 * s).max(1.0), EDGE),
        StrokeKind::Inside,
    );
    let inner = r.shrink(4.0 * s);
    // Smaller, a step of the font's grid at a time, until it fits the box's height.
    let mut font = c.font(size);
    let mut galley = p.layout(text.to_owned(), font.clone(), INK, inner.width());
    while galley.size().y > inner.height() && font.size > 11.0 {
        font.size -= 11.0;
        galley = p.layout(text.to_owned(), font.clone(), INK, inner.width());
    }
    p.galley(
        pos2(inner.left(), inner.center().y - galley.size().y / 2.0),
        galley,
        INK,
    );
}

/// The band when no menu waits: the game's own text box, its words re-inked white on GoldLink's
/// grey (`textbox` is the game's text box rows, already recoloured; see [`recolour_textbox`]).
pub fn draw_text_band(ui: &mut egui::Ui, frame: Rect, textbox: &egui::TextureHandle) {
    let band = band(frame);
    let s = frame.height() / ROWS;
    let p = ui.painter();
    p.rect_filled(band, CornerRadius::ZERO, BAND);
    p.line_segment(
        [band.left_top(), band.right_top()],
        Stroke::new((1.0 * s).max(2.0), ACCENT),
    );
    let inner = band.shrink2(vec2(4.0 * s, 3.0 * s));
    p.rect_filled(inner, CornerRadius::same((2.0 * s) as u8), PANEL);
    p.rect_stroke(
        inner,
        CornerRadius::same((2.0 * s) as u8),
        Stroke::new((0.6 * s).max(1.0), EDGE),
        StrokeKind::Inside,
    );
    // The words from the band's left, as the game prints them from its box's left.
    let words = Rect::from_min_size(band.min, vec2(256.0 * s, band.height()));
    p.image(
        textbox.id(),
        words,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// A choice the game waits on, drawn as a button on the touch screen (the nurse's YES and NO, the
/// PC's list): its face (DS pixels, x1 and y1 past the end) and whether it's the selected one (a
/// red outline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchChoice {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
    pub selected: bool,
}

fn channels(p: u32) -> (u32, u32, u32) {
    ((p >> 16) & 255, (p >> 8) & 255, p & 255)
}

/// The touch screen's choice buttons (DS pixels 256 x 192, 0x00RRGGBB), 2 to 6 of them, or none.
/// A button's face, down a column inside it clear of its words (x 40, or 236 for the PC's list,
/// whose words start at the left): white (251) for at least 10 rows, then light grey (211, 170)
/// and grey (130) for at least 10; its outline above it dark grey, or red when it's the selected
/// one. Its width is the white run across its first white row.
pub fn touch_choices(bottom: &[u32]) -> Vec<TouchChoice> {
    // Three columns, their buttons merged: a button's label can cross a column and hide it (the
    // Poké Mart's SELL at column 40, the user's v0.7 play test), and a menu can stand in two
    // columns (the PC's storage menu: two buttons at the left, three at the right, whose labels
    // cross column 40; the user's v0.7.1 play test: three of five found, so no box). Column 12
    // is left of any label, 236 right of the right column's.
    let mut all: Vec<TouchChoice> = Vec::new();
    for x in [12, 40, 236] {
        for c in touch_choices_at(bottom, x) {
            let same = |o: &TouchChoice| o.y0.abs_diff(c.y0) <= 4 && o.x0 < c.x1 && c.x0 < o.x1;
            match all.iter_mut().find(|o| same(o)) {
                Some(o) => o.selected |= c.selected,
                None => all.push(c),
            }
        }
    }
    // In the game's order: row by row (a row's buttons within a few pixels), left to right.
    all.sort_by_key(|c| c.y0);
    let mut out: Vec<TouchChoice> = Vec::with_capacity(all.len());
    let mut row = 0;
    for (k, c) in all.iter().enumerate() {
        if c.y0 > all[row].y0 + 4 {
            out[row..].sort_by_key(|c| c.x0);
            row = k;
        }
        out.push(*c);
    }
    out[row..].sort_by_key(|c| c.x0);
    if (2..=6).contains(&out.len()) {
        out
    } else {
        Vec::new()
    }
}

/// Whether the touch screen shows a Pokémon's summary (any of its pages): the party panel's
/// frame and ground at its top and bottom edges and the blue Cancel button, away from the party's
/// icons and the page tabs (measured raw on the release v0.7.1, the Info, Battle Moves and
/// Ribbons pages against the party screen and its menu).
pub fn summary_shown(bottom: &[u32]) -> bool {
    const MARKS: [(usize, usize, u32); 9] = [
        (150, 0, 0x41_4141),
        (200, 0, 0xE3_EBEB),
        (250, 0, 0xAA_BAC3),
        (150, 152, 0x41_4141),
        (200, 152, 0xE3_EBEB),
        (250, 152, 0xAA_BAC3),
        (254, 100, 0xAA_BAC3),
        (210, 170, 0x28_92FB),
        (240, 175, 0x28_92FB),
    ];
    MARKS.iter().all(|&(x, y, v)| {
        bottom.get(y * 256 + x).is_some_and(|&p| {
            let (r, g, b) = channels(p);
            let (vr, vg, vb) = channels(v);
            r.abs_diff(vr) <= 8 && g.abs_diff(vg) <= 8 && b.abs_diff(vb) <= 8
        })
    })
}

/// Whether the touch screen shows the Poké Mart's list to buy from (its slate header with the
/// money, a dark line under it, the blue ground; measured raw: 718292, 414141, 389AFB): the
/// one-screen layout pops it up, as the start menu.
pub fn mart_list_shown(bottom: &[u32]) -> bool {
    let near = |x: usize, y: usize, v: u32| {
        bottom.get(y * 256 + x).is_some_and(|&p| {
            let (r, g, b) = channels(p);
            let (vr, vg, vb) = channels(v);
            r.abs_diff(vr) <= 8 && g.abs_diff(vg) <= 8 && b.abs_diff(vb) <= 8
        })
    };
    [2, 128, 250]
        .iter()
        .all(|&x| near(x, 20, 0x71_8292) && near(x, 32, 0x41_4141))
        // The ground at the screen's left edge and between the two columns of items, which the
        // items' buttons never cover (at x 2 a second row's button did: four items, the user's
        // v0.7.1 play test, no pop-up).
        && [(0, 60), (0, 150), (128, 60), (128, 100), (128, 150)]
            .iter()
            .all(|&(x, y)| near(x, y, 0x38_9AFB))
}

fn touch_choices_at(bottom: &[u32], column: usize) -> Vec<TouchChoice> {
    let x_at = column;
    let px = |x: usize, y: usize| bottom.get(y * 256 + x).copied().unwrap_or(0);
    let is = |p: u32, v: u32| {
        let (r, g, b) = channels(p);
        r.abs_diff(v) <= 6 && g.abs_diff(v) <= 6 && b.abs_diff(v) <= 6
    };
    let mut out = Vec::new();
    let mut y = 0;
    while y < 192 {
        if !is(px(x_at, y), 251) {
            y += 1;
            continue;
        }
        let start = y;
        while y < 192 && is(px(x_at, y), 251) {
            y += 1;
        }
        if y - start < 10 || y >= 192 || !is(px(x_at, y), 211) {
            continue;
        }
        let mut end = y;
        while end < 192
            && (is(px(x_at, end), 211) || is(px(x_at, end), 170) || is(px(x_at, end), 130))
        {
            end += 1;
        }
        if end - y >= 10 {
            let row = start + 1;
            let (mut x0, mut x1) = (x_at, x_at);
            while x0 > 0 && is(px(x0 - 1, row), 251) {
                x0 -= 1;
            }
            while x1 < 256 && is(px(x1, row), 251) {
                x1 += 1;
            }
            let y0 = start.saturating_sub(2);
            let (r, g, _) = channels(px(x_at, y0.saturating_sub(2)));
            if x1 - x0 >= 40 {
                out.push(TouchChoice {
                    x0,
                    y0,
                    x1,
                    y1: end,
                    selected: r > 180 && g < 110,
                });
            }
        }
        y = end;
    }
    out
}

/// The game's text boxes' ink and its shadow (a message's letters, pret's TEXT_COLOR 1 and 2 in
/// the field's palette).
pub const TEXT_INK: Color32 = Color32::from_rgb(81, 81, 89);
pub const TEXT_SHADOW: Color32 = Color32::from_rgb(162, 162, 170);

/// Whether the touch screen shows the Pokégear (its map, radio or phone, a call included): the
/// frame over its tab bar, white, black, white lines across the screen (rows 158, 160, 162) and
/// the white one under it (row 190), at four places off the tabs (DS pixels 256 x 192,
/// 0x00RRGGBB).
pub fn pokegear_shown(bottom: &[u32]) -> bool {
    const XS: [usize; 4] = [3, 60, 200, 252];
    let level = |x: usize, y: usize| {
        bottom.get(y * 256 + x).map(|&p| {
            let (r, g, b) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
            if r > 230 && g > 230 && b > 230 {
                1
            } else if r < 20 && g < 20 && b < 20 {
                0
            } else {
                2
            }
        })
    };
    XS.iter().all(|&x| {
        level(x, 158) == Some(1)
            && level(x, 160) == Some(0)
            && level(x, 162) == Some(1)
            && level(x, 190) == Some(1)
    })
}

/// Whether the touch screen shows a phone call (the Pokégear's): the list dimmed behind the
/// call's wide grey button with its arrow (rows 145 and 172 grey across the screen, 227 and 138,
/// the arrow at its middle). The call's words are on the top screen.
pub fn phone_call_shown(bottom: &[u32]) -> bool {
    let grey = |x: usize, y: usize, v: u32| {
        bottom.get(y * 256 + x).is_some_and(|&p| {
            let (r, g, b) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
            r.abs_diff(v) <= 10 && g.abs_diff(v) <= 10 && b.abs_diff(v) <= 10
        })
    };
    let arrow = bottom.get(160 * 256 + 128).is_some_and(|&p| {
        let (r, g, b) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
        r < 130 && g < 130 && b < 150 && b >= r
    });
    [20, 128, 236].iter().all(|&x| grey(x, 145, 227))
        && [20, 236].iter().all(|&x| grey(x, 172, 138))
        && arrow
}

/// Whether the touch screen shows the start menu's green panel (DS pixels 256 x 192,
/// 0x00RRGGBB): five points inside it, (24, 203, 113). Not while its Save screen (or another
/// application) has taken the touch screen: the running shoes' button goes with the panel.
pub fn start_menu_panel(bottom: &[u32]) -> bool {
    const POINTS: [(usize, usize); 5] = [(12, 30), (20, 60), (90, 60), (155, 100), (12, 150)];
    POINTS.iter().all(|&(x, y)| {
        bottom.get(y * 256 + x).is_some_and(|&p| {
            let (r, g, b) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
            r.abs_diff(24) <= 10 && g.abs_diff(203) <= 10 && b.abs_diff(113) <= 10
        })
    })
}

/// Whether the band belongs over the game's text box rows (144 to 191 of the top screen) in a
/// battle: yes, but for a screen the game draws there itself, as a link battle's VS screen with
/// the bottom player's party on a light ground (58% of rows 150 to 184 light, the text box's own
/// inside FBFBFB not counted). The text box, its opening (black, a grey frame) and the dark rows
/// before it keep the band.
pub fn game_textbox_shown(top_screen_rows: &[u32]) -> bool {
    const INSIDE: u32 = 0xFB_FBFB;
    let (mut light, mut all) = (0u32, 0u32);
    for y in 6..41 {
        for x in 8..248 {
            if let Some(&px) = top_screen_rows.get(y * 256 + x) {
                let px = px & 0xFF_FFFF;
                all += 1;
                let lum = ((px >> 16) & 0xFF) + ((px >> 8) & 0xFF) + (px & 0xFF);
                light += u32::from(px != INSIDE && lum > 3 * 180);
            }
        }
    }
    all == 0 || light * 10 < all * 4
}

/// Whether the top screen shows the Pokédex's registration page after a catch (the band hid its
/// bottom, the user's v0.7 play test): its header across the top, one warm colour (measured raw:
/// FBBA38, 921818 as it fades in, darker as it fades out), where a battle has its sky.
pub fn pokedex_page_shown(top: &[u32]) -> bool {
    let at = |x: usize| top.get(256 + x).map(|&p| channels(p & 0xFF_FFFF));
    let Some((r, g, b)) = at(128) else {
        return false;
    };
    let warm = r >= 40 && r >= g + 15 && r >= b + 30;
    // Its frame's ends a darker shade (CB8A20 by FBBA38).
    let ends = [2, 250]
        .iter()
        .all(|&x| at(x).is_some_and(|(r2, g2, _)| r2 + 20 <= r && r2 > g2 + 10));
    warm && ends
        && (16..241).step_by(4).all(|x| {
            at(x).is_some_and(|(r2, g2, b2)| {
                r2.abs_diff(r) <= 6 && g2.abs_diff(g) <= 6 && b2.abs_diff(b) <= 6
            })
        })
}

/// A game window reaching down into the band's rows (DS columns and rows, ends past the last):
/// the level-up's stats box, drawn over the game's text box by the game itself, hid under the band
/// (the user's v0.7 play test). Found by the game's window frame down its left side, in the band's
/// rows: a dark blue line, a mid blue, a light blue, then the white inside; then its top, right
/// and bottom edges along the same frame.
pub fn window_under_band(top: &[u32]) -> Option<[usize; 4]> {
    let px = |x: usize, y: usize| top.get(y * 256 + x).map(|&p| channels(p & 0xFF_FFFF));
    // Measured (the stats box, raw): the dark line 30495F, mid 496179 (two pixels), light AABAC3
    // (two), the inside FBFBFB. The box reaches 13 rows into the band (to row 157).
    let dark = |x: usize, y: usize| {
        px(x, y).is_some_and(|(r, g, b)| {
            r < 80 && (45..115).contains(&g) && (75..145).contains(&b) && b > r + 30
        })
    };
    let white =
        |x: usize, y: usize| px(x, y).is_some_and(|(r, g, b)| r > 225 && g > 225 && b > 220);
    let inside_right = |x: usize, y: usize| (3..=7).any(|d| white(x + d, y));
    let inside_left = |x: usize, y: usize| (3..=7).any(|d| x >= d && white(x - d, y));
    let probe = SCENE_ROWS as usize + 2;
    let x0 = (16..240)
        .find(|&x| (0..3).all(|k| dark(x, probe + 3 * k) && inside_right(x, probe + 3 * k)))?;
    let mut y0 = probe;
    while y0 > 0 && dark(x0, y0 - 1) {
        y0 -= 1;
    }
    // Down the frame's second column (its mid line, then the bottom lines') to the box's end.
    let framed = |x: usize, y: usize| {
        dark(x, y) || px(x, y).is_some_and(|(r, g, b)| (150..195).contains(&r) && g > r && b > g)
    };
    let mut y1 = probe;
    while y1 + 1 < 192 && framed(x0 + 1, y1 + 1) {
        y1 += 1;
    }
    // The right edge: the dark line again, the inside left of it; on to the outer line.
    let mut x1 = (x0 + 8..256).find(|&x| dark(x, probe) && inside_left(x, probe))?;
    while x1 + 1 < 256 && dark(x1 + 1, probe) {
        x1 += 1;
    }
    (y0 < SCENE_ROWS as usize && x1 - x0 >= 24).then_some([x0, y0, x1 + 1, y1 + 1])
}

/// The game's text box rows (144 to 191 of the top screen) with only its words kept: the text
/// colour becomes white, its shadow a near black, everything else transparent. The game prints
/// battle messages in one palette (measured: text 515159, shadow A2A2AA on FBFBFB; the box's
/// bottom border is the text colour too, so only the box's inside is kept). Only from an open
/// box, its inside at least half FBFBFB: the box fades in and out (and so does the whole screen),
/// and on the way its frame and inside pass through the text colour, which drew white blocks.
pub fn recolour_textbox(top_screen_rows: &[u32]) -> egui::ColorImage {
    const TEXT: u32 = 0x51_5159;
    const SHADOW: u32 = 0xA2_A2AA;
    const INSIDE: u32 = 0xFB_FBFB;
    let rows = (ROWS - SCENE_ROWS) as usize;
    let mut pixels = vec![Color32::TRANSPARENT; 256 * rows];
    let inside = (6..41.min(rows))
        .flat_map(|y| (8..248).map(move |x| y * 256 + x))
        .filter(|&i| {
            top_screen_rows
                .get(i)
                .is_some_and(|&px| px & 0xFF_FFFF == INSIDE)
        })
        .count();
    if inside * 2 < (41.min(rows) - 6) * 240 {
        return egui::ColorImage {
            size: [256, rows],
            pixels,
        };
    }
    // The box's inside (rows 150 to 184, columns 8 to 247): its borders share the text colour.
    for y in 6..41.min(rows) {
        for x in 8..248 {
            let Some(&px) = top_screen_rows.get(y * 256 + x) else {
                continue;
            };
            pixels[y * 256 + x] = match px & 0xFF_FFFF {
                TEXT => INK,
                SHADOW => Color32::from_rgb(0x14, 0x11, 0x10),
                _ => Color32::TRANSPARENT,
            };
        }
    }
    egui::ColorImage {
        size: [256, rows],
        pixels,
    }
}

/// The player's party, as the game's touch screen shows it: a small ball per Pokémon, greyed
/// when it has fainted, a dot for an empty slot; just above the band, on the left.
fn party_balls(p: &egui::Painter, band_top_left: Pos2, balls: &[Ball; 6], s: f32) {
    let r = 3.5 * s;
    let y = band_top_left.y - r - 2.5 * s;
    for (i, ball) in balls.iter().enumerate() {
        let c = pos2(
            band_top_left.x + 6.0 * s + r + i as f32 * (2.0 * r + 2.5 * s),
            y,
        );
        let line = Stroke::new((0.6 * s).max(1.0), Color32::from_rgb(0x1A, 0x17, 0x16));
        if *ball == Ball::Empty {
            p.circle_filled(c, 1.3 * s, Color32::from_rgb(0x8A, 0x84, 0x7E));
            p.circle_stroke(c, 1.3 * s, line);
            continue;
        }
        let (upper, lower) = match ball {
            Ball::Fainted => (
                Color32::from_rgb(0x7C, 0x76, 0x72),
                Color32::from_rgb(0xA8, 0xA2, 0x9C),
            ),
            _ => (
                Color32::from_rgb(0xE8, 0x3A, 0x34),
                Color32::from_rgb(0xF4, 0xF2, 0xEE),
            ),
        };
        p.circle_filled(c, r, lower);
        // The upper half.
        let top: Vec<Pos2> = (0..=16)
            .map(|k| {
                let a = std::f32::consts::PI * (1.0 + k as f32 / 16.0);
                c + vec2(r * a.cos(), r * a.sin())
            })
            .collect();
        p.add(egui::Shape::convex_polygon(top, upper, Stroke::NONE));
        p.line_segment([c - vec2(r, 0.0), c + vec2(r, 0.0)], line);
        p.circle_filled(c, 1.1 * s, lower);
        p.circle_stroke(c, 1.1 * s, line);
        p.circle_stroke(c, r, line);
        if *ball == Ball::Status {
            // A status condition: a small amber mark, as the game's ball shows one.
            p.circle_filled(c + vec2(r * 0.7, -r * 0.7), 1.3 * s, theme::AMBER);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_text_box_is_told_from_a_vs_screen() {
        let rows = (ROWS - SCENE_ROWS) as usize;
        // The text box: its inside, a line of words.
        let mut text = vec![0x00FB_FBFBu32; 256 * rows];
        for x in 20..200 {
            text[12 * 256 + x] = 0x0051_5159;
        }
        assert!(game_textbox_shown(&text));
        // The link battle's VS screen: a light background with party balls.
        let vs = vec![0x00E3_EBEBu32; 256 * rows];
        assert!(!game_textbox_shown(&vs));
        // Dark rows (the intro, before the box opens) and the box opening: the band.
        assert!(game_textbox_shown(&vec![0x0018_1818; 256 * rows]));
        let mut opening = vec![0u32; 256 * rows];
        for y in 10..30 {
            for x in 30..220 {
                opening[y * 256 + x] = 0x0099_9999;
            }
        }
        assert!(game_textbox_shown(&opening));
    }

    #[test]
    fn words_only_come_from_an_open_box() {
        let rows = (ROWS - SCENE_ROWS) as usize;
        let ink = |img: &egui::ColorImage| img.pixels.iter().filter(|&&c| c == INK).count();
        // The open box: its words.
        let mut open = vec![0x00FB_FBFBu32; 256 * rows];
        for x in 20..200 {
            open[12 * 256 + x] = 0x0051_5159;
        }
        assert_eq!(ink(&recolour_textbox(&open)), 180);
        // The box fading in: its darker right part passes through the text colour. No words.
        let mut fading = vec![0x00D6_D6D6u32; 256 * rows];
        for y in 6..41 {
            for x in 235..248 {
                fading[y * 256 + x] = 0x0051_5159;
            }
        }
        assert_eq!(ink(&recolour_textbox(&fading)), 0);
    }
}

#[cfg(test)]
mod window_tests {
    use super::window_under_band;

    /// The level-up's stats box as the game draws it (measured raw, the v0.7.1 levelup route): x
    /// 130 to 223, y 60 to 157, over the text box's white rows.
    fn screen(with_box: bool) -> Vec<u32> {
        let mut px = vec![0xCB_A259u32; 256 * 192];
        for y in 144..192 {
            for x in 0..256 {
                px[y * 256 + x] = 0xFB_FBFB;
            }
        }
        if with_box {
            let (x0, y0, x1, y1) = (130usize, 60usize, 223usize, 157usize);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let d = (x - x0).min(x1 - x).min(y - y0).min(y1 - y);
                    px[y * 256 + x] = match d {
                        0 => 0x30_495F,
                        1 | 2 => 0x49_6179,
                        3 | 4 => 0xAA_BAC3,
                        _ => 0xFB_FBFB,
                    };
                }
            }
        }
        px
    }

    #[test]
    fn finds_the_stats_box() {
        assert_eq!(window_under_band(&screen(true)), Some([130, 60, 224, 158]));
    }

    #[test]
    fn the_pokedex_page_hides_the_band() {
        let mut px = screen(false);
        // A battle's sky (measured: CBCBCB to EBEBDB across the row).
        for x in 0..256 {
            let v = 0xCB + (x as u32 * 0x20) / 256;
            px[256 + x] = v << 16 | v << 8 | v;
        }
        assert!(!super::pokedex_page_shown(&px), "a battle's sky");
        for (v, end) in [
            (0xFB_BA38u32, 0xCB_8A20u32),
            (0x92_1818, 0x70_1010),
            (0x58_3808, 0x40_2004),
        ] {
            px[256..512].fill(v);
            px[256..256 + 16].fill(end);
            px[256 + 245..512].fill(end);
            assert!(super::pokedex_page_shown(&px), "{v:06X}");
        }
        px[256..512].fill(0x30_3030);
        assert!(!super::pokedex_page_shown(&px), "a fade's grey");
    }

    #[test]
    fn nothing_without_it() {
        assert_eq!(window_under_band(&screen(false)), None);
    }
}

#[cfg(test)]
mod choice_tests {
    use super::{mart_list_shown, summary_shown, touch_choices};

    /// The Poké Mart's question as the touch screen draws it (measured raw, the v0.7.1 mart route):
    /// three wide buttons (white 15 rows, D3D3D3 2, AAAAAA 15, 828282 2) on green, the first
    /// selected (a red line above it), each label a dark run across column 40 in the second.
    fn mart_question() -> Vec<u32> {
        let mut px = vec![0x28_DB92u32; 256 * 192];
        for (k, top) in [32usize, 80, 128].iter().enumerate() {
            let rows = [
                (15, 0xFB_FBFB),
                (2, 0xD3_D3D3),
                (15, 0xAA_AAAA),
                (2, 0x82_8282),
            ];
            let mut y = *top;
            for (n, v) in rows {
                for _ in 0..n {
                    px[y * 256 + 8..y * 256 + 248].fill(v);
                    y += 1;
                }
            }
            if k == 0 {
                px[(top - 4) * 256 + 8..(top - 4) * 256 + 248].fill(0xDB_3841);
            }
            // The label's letters in the lower half (SELL reaches column 40).
            if k == 1 {
                for y in top + 19..top + 22 {
                    px[y * 256 + 18..y * 256 + 46].fill(0x41_4141);
                }
            }
        }
        px
    }

    /// The PC's storage menu (measured on the user's save, release v0.7.1): DEPOSIT and MOVE
    /// POKéMON at the left, WITHDRAW, MOVE ITEMS and SEE YA! at the right, every left label
    /// across column 40.
    #[test]
    fn a_menu_in_two_columns_is_read_row_by_row() {
        let mut px = vec![0x28_DB92u32; 256 * 192];
        let mut button = |x0: usize, x1: usize, top: usize, selected: bool| {
            let mut y = top;
            for (n, v) in [
                (15, 0xFB_FBFB),
                (2, 0xD3_D3D3),
                (15, 0xAA_AAAA),
                (2, 0x82_8282),
            ] {
                for _ in 0..n {
                    px[y * 256 + x0..y * 256 + x1].fill(v);
                    y += 1;
                }
            }
            if selected {
                px[(top - 4) * 256 + x0..(top - 4) * 256 + x1].fill(0xDB_3841);
            }
            if x0 < 40 {
                for y in top + 4..top + 28 {
                    px[y * 256 + 16..y * 256 + 64].fill(0x41_4141);
                }
            }
        };
        button(8, 120, 32, true);
        button(136, 248, 32, false);
        button(8, 120, 80, false);
        button(136, 248, 80, false);
        button(136, 248, 128, false);
        let c = touch_choices(&px);
        let at: Vec<(usize, usize)> = c.iter().map(|c| (c.x0, c.y0)).collect();
        assert_eq!(at, [(8, 30), (136, 30), (8, 78), (136, 78), (136, 126)]);
        assert!(c[0].selected && c[1..].iter().all(|c| !c.selected));
    }

    #[test]
    fn the_summary_is_told() {
        let mut px = vec![0xE3_EBEBu32; 256 * 192];
        for (x, y, v) in [
            (150, 0, 0x41_4141),
            (250, 0, 0xAA_BAC3),
            (150, 152, 0x41_4141),
            (250, 152, 0xAA_BAC3),
            (254, 100, 0xAA_BAC3),
            (210, 170, 0x28_92FB),
            (240, 175, 0x28_92FB),
        ] {
            px[y * 256 + x] = v;
        }
        assert!(summary_shown(&px));
        assert!(!summary_shown(&mart_question()));
    }

    #[test]
    fn a_label_across_a_column_hides_no_button() {
        let c = touch_choices(&mart_question());
        assert_eq!(c.len(), 3);
        assert!(c[0].selected && !c[1].selected);
    }

    #[test]
    fn the_buy_list_is_told() {
        let mut px = vec![0x38_9AFBu32; 256 * 192];
        for y in 0..32 {
            px[y * 256..y * 256 + 256].fill(0x71_8292);
        }
        px[32 * 256..33 * 256].fill(0x41_4141);
        // Four items: two rows of buttons, their dark edges at x 2.
        for y in 40..124 {
            px[y * 256 + 2..y * 256 + 126].fill(0x41_4141);
            px[y * 256 + 130..y * 256 + 254].fill(0x41_4141);
        }
        assert!(mart_list_shown(&px));
        assert!(!mart_list_shown(&mart_question()));
    }
}
