//! Keep idle animation from rebuilding the native input region every frame.
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::Rectangle;

/// Button handling must use the same captured region installed on the window.
/// Rechecking the current animation's alpha would silently reject valid presses
/// whenever the visible silhouette moves inside that stable region.
pub fn contains(shape: &[Rectangle], x: i16, y: i16) -> bool {
    let [x, y] = [i32::from(x), i32::from(y)];
    shape.iter().any(|r| {
        x >= i32::from(r.x)
            && x < i32::from(r.x) + i32::from(r.width)
            && y >= i32::from(r.y)
            && y < i32::from(r.y) + i32::from(r.height)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Stable,
    Pixel,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Pose {
    pub mirrored: bool,
    pub dragging: bool,
    pub reacting: bool,
    pub jumping: bool,
}

pub struct InputRegion {
    mode: Mode,
    initialized: bool,
    animation: String,
    pose: Pose,
    settle_at: Option<Instant>,
}

impl InputRegion {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            initialized: false,
            animation: String::new(),
            pose: Pose::default(),
            settle_at: None,
        }
    }

    pub fn refresh(&mut self, animation: &str, pose: Pose, now: Instant) -> bool {
        if self.mode == Mode::Pixel {
            return true;
        }
        if !self.initialized || self.animation != animation || self.pose != pose {
            self.initialized = true;
            self.animation.clear();
            self.animation.push_str(animation);
            self.pose = pose;
            // Capture immediately for a new pose/facing, then once more after
            // the transition. Breathing, gaze and animation frames do not
            // repeatedly change which native surface owns the pointer.
            self.settle_at = Some(now + Duration::from_millis(250));
            return true;
        }
        if pose.jumping {
            // Jump translates the whole character inside its window. Keep its
            // airborne outline accurate; idle stabilizes again after landing.
            return true;
        }
        if self.settle_at.is_some_and(|deadline| now >= deadline) {
            self.settle_at = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_press_uses_captured_outline_after_pixels_move() {
        let mut frame = vec![0; 8 * 4];
        frame[3] = 255;
        frame[7] = 255;
        let captured = crate::pixels::input_shape(&frame, 4);
        // The animated image moves away, while the installed input region stays.
        frame.fill(0);
        frame[11] = 255;
        let next_frame = crate::pixels::input_shape(&frame, 4);
        assert!(contains(&captured, 0, 0));
        assert!(contains(&captured, 1, 0));
        assert!(!contains(&next_frame, 0, 0));
        assert!(!contains(&captured, 2, 0));
        assert!(contains(&next_frame, 2, 0));
    }

    #[test]
    fn press_bounds_exclude_holes_and_rectangle_endpoints() {
        let shape = [
            Rectangle {
                x: 2,
                y: 3,
                width: 2,
                height: 4,
            },
            Rectangle {
                x: 6,
                y: 3,
                width: 2,
                height: 4,
            },
        ];
        assert!(contains(&shape, 2, 3));
        assert!(contains(&shape, 3, 6));
        assert!(contains(&shape, 6, 3));
        for (x, y) in [(-1, 3), (2, -1), (1, 3), (4, 3), (5, 3), (8, 3), (2, 7)] {
            assert!(!contains(&shape, x, y));
        }
    }

    #[test]
    fn idle_frames_do_not_rebuild_the_input_region() {
        let start = Instant::now();
        let mut input = InputRegion::new(Mode::Stable);
        let updates = (0..600)
            .filter(|frame| {
                input.refresh(
                    "idle",
                    Pose::default(),
                    start + Duration::from_millis(frame * 17),
                )
            })
            .count();
        assert_eq!(updates, 2);
    }

    #[test]
    fn facing_action_drag_recovery_and_jump_refresh_the_outline() {
        let start = Instant::now();
        let mut input = InputRegion::new(Mode::Stable);
        let mut pose = Pose::default();
        assert!(input.refresh("idle", pose, start));
        assert!(!input.refresh("idle", pose, start));
        pose.mirrored = true;
        assert!(input.refresh("walk", pose, start));
        assert!(!input.refresh("walk", pose, start));
        pose.dragging = true;
        assert!(input.refresh("idle", pose, start));
        pose.dragging = false;
        pose.reacting = true;
        assert!(input.refresh("idle", pose, start));
        pose.reacting = false;
        pose.jumping = true;
        assert!(input.refresh("idle", pose, start));
        assert!(input.refresh("idle", pose, start));
        pose.jumping = false;
        assert!(input.refresh("idle", pose, start));
        assert!(input.refresh("idle", pose, start + Duration::from_millis(250)));
        assert!(!input.refresh("idle", pose, start + Duration::from_secs(1)));
    }

    #[test]
    fn pixel_mode_always_tracks_the_current_frame() {
        let mut input = InputRegion::new(Mode::Pixel);
        let start = Instant::now();
        for _ in 0..20 {
            assert!(input.refresh("idle", Pose::default(), start));
        }
    }
}
