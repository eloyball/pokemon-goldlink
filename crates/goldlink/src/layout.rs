//! Where the two screens go in the window (RFC §6 M2: two-screen layout and scaling; UC9 §3.1:
//! the one-screen layout).

use eframe::egui::{Rect, Vec2, pos2, vec2};
use goldlink_runtime::config::{Layout, Video};

const W: f32 = 256.0;
const H: f32 = 192.0;

fn fit(area: Rect, unit: Vec2, integer: bool) -> f32 {
    let scale = (area.width() / unit.x).min(area.height() / unit.y).max(0.0);
    if integer && scale >= 1.0 {
        scale.floor()
    } else {
        scale
    }
}

/// The top and bottom screens' rectangles inside `area`, centred, aspect kept. (The one-screen
/// layout is [`one_screen`]; here it falls back to the screens stacked.)
pub fn screens(area: Rect, video: &Video) -> (Rect, Rect) {
    let gap = video.gap as f32;
    let (unit, step): (Vec2, Vec2) = match video.layout {
        Layout::Vertical | Layout::OneScreen => (vec2(W, H * 2.0 + gap), vec2(0.0, H + gap)),
        Layout::Horizontal => (vec2(W * 2.0 + gap, H), vec2(W + gap, 0.0)),
    };
    let scale = fit(area, unit, video.integer_scale);
    let size = unit * scale;
    let origin = area.center() - size / 2.0;
    let top = Rect::from_min_size(pos2(origin.x, origin.y), vec2(W, H) * scale);
    let bottom = top.translate(step * scale);
    (top, bottom)
}

/// The one-screen layout's frame (UC9 §3): one DS screen's shape, centred, as large as fits.
pub fn one_screen(area: Rect, video: &Video) -> Rect {
    let unit = vec2(W, H);
    let size = unit * fit(area, unit, video.integer_scale);
    Rect::from_center_size(area.center(), size)
}

/// The window size that shows the screens at `scale` with this layout.
pub fn window_size(video: &Video, scale: f32) -> Vec2 {
    let gap = video.gap as f32;
    match video.layout {
        Layout::Vertical => vec2(W, H * 2.0 + gap) * scale,
        Layout::Horizontal => vec2(W * 2.0 + gap, H) * scale,
        Layout::OneScreen => vec2(W, H) * scale,
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
    fn one_screen_is_one_ds_screen() {
        let v = Video {
            layout: Layout::OneScreen,
            integer_scale: true,
            ..Video::default()
        };
        assert_eq!(window_size(&v, 2.0), vec2(512.0, 384.0));
        let r = one_screen(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0)),
            &v,
        );
        assert_eq!(r.size(), vec2(1280.0, 960.0)); // 5.6x floors to 5x
        assert_eq!(r.center(), pos2(960.0, 540.0));
    }
}
