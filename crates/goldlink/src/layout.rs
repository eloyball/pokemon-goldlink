//! Where the two screens go in the window (RFC §6 M2: two-screen layout and scaling).

use eframe::egui::{Rect, Vec2, pos2, vec2};
use goldlink_runtime::config::{Layout, Video};

const W: f32 = 256.0;
const H: f32 = 192.0;

/// The top and bottom screens' rectangles inside `area`, centred, aspect kept.
pub fn screens(area: Rect, video: &Video) -> (Rect, Rect) {
    let gap = video.gap as f32;
    let (unit, step): (Vec2, Vec2) = match video.layout {
        Layout::Vertical => (vec2(W, H * 2.0 + gap), vec2(0.0, H + gap)),
        Layout::Horizontal => (vec2(W * 2.0 + gap, H), vec2(W + gap, 0.0)),
    };
    let mut scale = (area.width() / unit.x).min(area.height() / unit.y).max(0.0);
    if video.integer_scale && scale >= 1.0 {
        scale = scale.floor();
    }
    let size = unit * scale;
    let origin = area.center() - size / 2.0;
    let top = Rect::from_min_size(pos2(origin.x, origin.y), vec2(W, H) * scale);
    let bottom = top.translate(step * scale);
    (top, bottom)
}

/// The window size that shows the screens at `scale` with this layout.
pub fn window_size(video: &Video, scale: f32) -> Vec2 {
    let gap = video.gap as f32;
    match video.layout {
        Layout::Vertical => vec2(W, H * 2.0 + gap) * scale,
        Layout::Horizontal => vec2(W * 2.0 + gap, H) * scale,
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
}
