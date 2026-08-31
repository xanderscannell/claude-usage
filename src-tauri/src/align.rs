//! Puts the window on the display it was built for.
//!
//! This dashboard lives on a small auxiliary panel that is exactly the
//! size of the window, and the window is fixed at that size, so there is
//! no maximize to lean on and dragging it into place by hand never lands
//! pixel-perfect. No desktop monitor shares that resolution, so matching
//! a display against the window's own configured size identifies the
//! panel outright, with no coordinates, monitor names, or saved geometry
//! to go stale when the desk gets rearranged.

use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// Slack allowed when measuring a display against the window, to absorb
/// rounding under fractional scaling.
const TOLERANCE_PX: i64 = 2;

/// A monitor reduced to the numbers the choice needs, so the rule can be
/// exercised without opening a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub primary: bool,
}

/// Finds the display that is the window's own size. `w` and `h` are
/// logical pixels, measured against each candidate at that candidate's
/// own scale factor, so the match survives a panel running at anything
/// other than 100%.
pub fn pick(screens: &[Screen], w: f64, h: f64) -> Option<Screen> {
    screens
        .iter()
        .filter(|s| fits(s.width, w * s.scale) && fits(s.height, h * s.scale))
        // A display this size is almost certainly the panel, but prefer a
        // secondary one and settle any remaining tie by position, so an
        // unchanged desk always sends the window to the same place.
        .min_by_key(|s| (s.primary, s.x, s.y))
        .copied()
}

fn fits(actual: u32, wanted: f64) -> bool {
    (actual as i64 - wanted.round() as i64).abs() <= TOLERANCE_PX
}

/// Moves the window onto the panel and sizes it to fill that display
/// exactly. Returns false when the panel is not connected, in which case
/// the window is left precisely where it was.
pub fn align(window: &WebviewWindow) -> bool {
    let Some((w, h)) = target_size(window) else {
        return false;
    };
    let Some(screen) = pick(&screens(window), w, h) else {
        return false;
    };

    // Position first: crossing onto a display at a different scale makes
    // Windows resize the window under us, so correcting the size has to
    // come afterwards or it gets overwritten.
    if window
        .set_position(PhysicalPosition::new(screen.x, screen.y))
        .is_err()
    {
        return false;
    }

    let fill = PhysicalSize::new(screen.width, screen.height);
    if window.inner_size().map(|s| s != fill).unwrap_or(true) {
        let _ = window.set_size(fill);
    }

    // An undecorated window can still carry an invisible frame, which
    // would leave the move a few pixels short of the corner.
    nudge_for_frame(window, screen);
    true
}

/// Corrects for any gap between where the window was asked to sit and
/// where its visible edge actually landed.
fn nudge_for_frame(window: &WebviewWindow, screen: Screen) {
    let (Ok(outer), Ok(inner)) = (window.outer_position(), window.inner_position()) else {
        return;
    };
    let (dx, dy) = (inner.x - outer.x, inner.y - outer.y);
    if dx != 0 || dy != 0 {
        let _ = window.set_position(PhysicalPosition::new(screen.x - dx, screen.y - dy));
    }
}

/// The window's configured size, which is also the resolution of the
/// display it is meant to fill. Read from the config rather than the live
/// window so a stray resize cannot move the goalposts.
fn target_size(window: &WebviewWindow) -> Option<(f64, f64)> {
    let label = window.label();
    window
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == label)
        .map(|w| (w.width, w.height))
}

fn screens(window: &WebviewWindow) -> Vec<Screen> {
    let primary = window.primary_monitor().ok().flatten().map(|m| *m.position());
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Screen {
            x: m.position().x,
            y: m.position().y,
            width: m.size().width,
            height: m.size().height,
            scale: m.scale_factor(),
            primary: Some(*m.position()) == primary,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f64 = 800.0;
    const H: f64 = 480.0;

    fn screen(x: i32, width: u32, height: u32) -> Screen {
        Screen { x, y: 0, width, height, scale: 1.0, primary: false }
    }

    /// Three ordinary monitors, the middle one primary.
    fn desk() -> Vec<Screen> {
        vec![
            screen(-2560, 2560, 1440),
            Screen { primary: true, ..screen(0, 2560, 1440) },
            screen(2560, 2560, 1440),
        ]
    }

    #[test]
    fn finds_the_panel_among_desktop_monitors() {
        let mut all = desk();
        let panel = Screen { y: 960, ..screen(5120, 800, 480) };
        all.push(panel);

        assert_eq!(pick(&all, W, H), Some(panel));
    }

    #[test]
    fn no_panel_means_no_move() {
        assert_eq!(pick(&desk(), W, H), None);
    }

    #[test]
    fn a_scaled_panel_still_matches() {
        // 800x480 logical at 125% is 1000x600 of real pixels.
        let panel = Screen { scale: 1.25, ..screen(5120, 1000, 600) };
        assert_eq!(pick(&[panel], W, H), Some(panel));
    }

    #[test]
    fn fractional_scaling_is_within_tolerance() {
        // 150% of 800x480 is 1200x720; a panel reporting one pixel off
        // is the same panel, not a different display.
        let panel = Screen { scale: 1.5, ..screen(5120, 1201, 719) };
        assert_eq!(pick(&[panel], W, H), Some(panel));
    }

    #[test]
    fn a_secondary_display_wins_over_the_primary() {
        let primary = Screen { primary: true, ..screen(0, 800, 480) };
        let panel = screen(5120, 800, 480);
        assert_eq!(pick(&[primary, panel], W, H), Some(panel));
    }

    #[test]
    fn identical_candidates_resolve_the_same_way_every_time() {
        let near = screen(-2560, 800, 480);
        let far = screen(5120, 800, 480);
        assert_eq!(pick(&[far, near], W, H), Some(near));
        assert_eq!(pick(&[near, far], W, H), Some(near));
    }

    #[test]
    fn a_monitor_that_is_merely_close_is_not_the_panel() {
        assert_eq!(pick(&[screen(5120, 810, 480)], W, H), None);
        assert_eq!(pick(&[screen(5120, 800, 500)], W, H), None);
    }
}
