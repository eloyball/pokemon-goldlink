//! Where the two screens go in the window (RFC §6 M2: two-screen layout and scaling; UC9 §3.1:
//! the one-screen layout).

use eframe::egui::{Rect, Vec2, pos2, vec2};
use goldlink_runtime::config::{Layout, Video};

const W: f32 = 256.0;
const H: f32 = 192.0;

/// The one-screen layout's second screen, against the big one.
pub const DOCK: f32 = 0.5;
/// The least space between the big screen and the dock, in DS pixels.
const DOCK_GAP: f32 = 4.0;

fn fit(area: Rect, unit: Vec2, integer: bool) -> f32 {
    let scale = (area.width() / unit.x).min(area.height() / unit.y).max(0.0);
    if integer && scale >= 1.0 {
        scale.floor()
    } else {
        scale
    }
}

/// The space the one-screen layout needs at 1x: the big screen and the dock beside it.
fn focus_unit(video: &Video) -> (Vec2, f32) {
    let gap = (video.gap as f32).max(DOCK_GAP);
    (vec2(W * (1.0 + DOCK) + gap, H), gap)
}

/// The top and bottom screens' rectangles inside `area`, centred, aspect kept. (The one-screen
/// layout is [`focus`]; here it falls back to the screens stacked.)
pub fn screens(area: Rect, video: &Video) -> (Rect, Rect) {
    let gap = video.gap as f32;
    let (unit, step): (Vec2, Vec2) = match video.layout {
        Layout::Vertical | Layout::Focus => (vec2(W, H * 2.0 + gap), vec2(0.0, H + gap)),
        Layout::Horizontal => (vec2(W * 2.0 + gap, H), vec2(W + gap, 0.0)),
    };
    let scale = fit(area, unit, video.integer_scale);
    let size = unit * scale;
    let origin = area.center() - size / 2.0;
    let top = Rect::from_min_size(pos2(origin.x, origin.y), vec2(W, H) * scale);
    let bottom = top.translate(step * scale);
    (top, bottom)
}

/// The one-screen layout (UC9 §3.1): the big screen, and the dock to its right at [`DOCK`] size,
/// their bottoms level. `open` (0 to 1) slides the dock in: closed, the big screen is centred on
/// its own; open, the two are centred together. The big screen's size never changes.
pub fn focus(area: Rect, video: &Video, open: f32) -> (Rect, Rect) {
    let (unit, gap) = focus_unit(video);
    let scale = fit(area, unit, video.integer_scale);
    let mut dock_scale = scale * DOCK;
    if video.integer_scale && dock_scale >= 1.0 {
        dock_scale = dock_scale.floor();
    }
    let big = vec2(W, H) * scale;
    let dock = vec2(W, H) * dock_scale;
    let open = open.clamp(0.0, 1.0);
    let alone = area.center().x - big.x / 2.0;
    let together = area.center().x - (big.x + gap * scale + dock.x) / 2.0;
    let left = alone + (together - alone) * open;
    let big = Rect::from_min_size(pos2(left, area.center().y - big.y / 2.0), big);
    // Slides in from a little further out as it fades in.
    let slide = (1.0 - open) * dock.x * 0.25;
    let dock = Rect::from_min_size(
        pos2(big.right() + gap * scale + slide, big.bottom() - dock.y),
        dock,
    );
    (big, dock)
}

/// The window size that shows the screens at `scale` with this layout.
pub fn window_size(video: &Video, scale: f32) -> Vec2 {
    let gap = video.gap as f32;
    match video.layout {
        Layout::Vertical => vec2(W, H * 2.0 + gap) * scale,
        Layout::Horizontal => vec2(W * 2.0 + gap, H) * scale,
        Layout::Focus => focus_unit(video).0 * scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_fits_and_centres() {
        let v = Video::default();
        let (t, b) = screens(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 768.0)), &v);
        assert_eq!(t.size(), vec2(512.0, 384.0));
        assert_eq!(t.min, pos2(244.0, 0.0));
        assert_eq!(b.min, pos2(244.0, 384.0));
    }

    #[test]
    fn horizontal_with_gap_and_integer_scale() {
        let v = Video {
            layout: Layout::Horizontal,
            gap: 8,
            integer_scale: true,
            ..Video::default()
        };
        let (t, b) = screens(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 500.0)), &v);
        assert_eq!(t.size(), vec2(512.0, 384.0)); // 2.1x floors to 2x
        assert_eq!(b.min.x - t.min.x, (256.0 + 8.0) * 2.0);
        assert_eq!(t.min.y, b.min.y);
    }

    #[test]
    fn focus_keeps_the_big_screen_and_slides_the_dock() {
        let v = Video {
            layout: Layout::Focus,
            ..Video::default()
        };
        let area = Rect::from_min_size(pos2(0.0, 0.0), window_size(&v, 2.0));
        assert_eq!(area.size(), vec2(776.0, 384.0));
        let (big, dock) = focus(area, &v, 1.0);
        assert_eq!(big, Rect::from_min_size(pos2(0.0, 0.0), vec2(512.0, 384.0)));
        assert_eq!(
            dock,
            Rect::from_min_size(pos2(520.0, 192.0), vec2(256.0, 192.0))
        );
        let (alone, _) = focus(area, &v, 0.0);
        assert_eq!(alone.size(), big.size());
        assert_eq!(alone.center().x, area.center().x);
        // A wide window (16:9): both fit, the big screen as large as the height allows.
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let (big, dock) = focus(wide, &v, 1.0);
        assert!(dock.right() <= wide.right() + 0.01 && big.left() >= 0.0);
        assert!((big.width() - 2.0 * dock.width()).abs() < 0.01);
    }
}
