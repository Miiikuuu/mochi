//! Desktop presentation only: held keys, fractional position and facing.
use mochi_core::{Scene, Vec2};

pub const SPEED: f64 = 96.0; // Desktop logical pixels per second.

pub struct Keys {
    down: [bool; 256],
    arrows: [u8; 256],
}
impl Default for Keys {
    fn default() -> Self {
        Self {
            down: [false; 256],
            arrows: [0; 256],
        }
    }
}
impl Keys {
    /// Returns false for auto-repeat. Cancelled keys require a physical release.
    pub fn press(&mut self, code: u8, symbol: u32, enabled: bool) -> bool {
        let code = usize::from(code);
        if std::mem::replace(&mut self.down[code], true) {
            return false;
        }
        if enabled {
            self.arrows[code] = match symbol {
                0xff51 => 1,
                0xff53 => 2,
                0xff52 => 4,
                0xff54 => 8,
                _ => 0,
            };
        }
        true
    }
    pub fn release(&mut self, code: u8) {
        self.down[usize::from(code)] = false;
        self.arrows[usize::from(code)] = 0;
    }
    pub fn cancel_motion(&mut self) {
        self.arrows.fill(0);
    }
    /// Do not inherit a key held while another application/character had focus.
    pub fn focus(&mut self, bitmap: [u8; 32]) {
        self.cancel_motion();
        for (code, down) in self.down.iter_mut().enumerate() {
            *down = bitmap[code / 8] & (1 << (code % 8)) != 0;
        }
    }
    pub fn direction(&self) -> [i8; 2] {
        let bits = self.arrows.iter().fold(0, |a, b| a | b);
        [
            i8::from(bits & 2 != 0) - i8::from(bits & 1 != 0),
            i8::from(bits & 8 != 0) - i8::from(bits & 4 != 0),
        ]
    }
    pub fn controlling(&self) -> bool {
        self.arrows.iter().any(|&arrow| arrow != 0)
    }
}

#[derive(Clone, Copy)]
pub struct Bounds {
    pub origin: [i32; 2],
    pub size: [u32; 2],
}
impl Bounds {
    pub fn distance_squared(self, point: [f64; 2]) -> f64 {
        (0..2)
            .map(|i| {
                let min = f64::from(self.origin[i]);
                let max = min + f64::from(self.size[i]);
                (point[i] - point[i].clamp(min, max)).powi(2)
            })
            .sum()
    }
}

pub struct Motion {
    position: [f64; 2],
    pub mirrored: bool,
    pub bounds: Bounds,
    size: u16,
}
impl Motion {
    pub fn new(position: [i32; 2], bounds: Bounds, size: u16) -> Self {
        Self {
            position: position.map(f64::from),
            mirrored: false,
            bounds,
            size,
        }
    }
    pub fn place(&mut self, position: [i32; 2]) {
        self.position = position.map(f64::from);
    }
    pub fn origin(&self) -> [i32; 2] {
        self.position.map(|p| p.round() as i32)
    }
    /// Do not snap a user's partially offscreen placement into the monitor.
    pub fn horizontal_room(&self) -> [f64; 2] {
        let min = self.bounds.origin.map(f64::from);
        let max = [0, 1].map(|i| min[i] + f64::from(self.bounds.size[i]) - f64::from(self.size));
        if (0..2).any(|i| self.position[i] < min[i] || self.position[i] > max[i]) {
            return [0.0, 0.0];
        }
        [self.position[0] - min[0], max[0] - self.position[0]]
    }
    pub fn advance(&mut self, direction: [i8; 2], distance: f64) -> bool {
        let [x, y] = direction.map(f64::from);
        let length = x.hypot(y);
        if length == 0.0 {
            return false;
        }
        if x != 0.0 {
            self.mirrored = x < 0.0;
        }
        let before = self.position;
        for (i, axis) in [x, y].into_iter().enumerate() {
            let min = f64::from(self.bounds.origin[i]);
            let max = min + f64::from(self.bounds.size[i].saturating_sub(u32::from(self.size)));
            self.position[i] = (self.position[i] + axis / length * distance).clamp(min, max);
        }
        self.position != before
    }
    pub fn project(&self, scene: &mut Scene) {
        if self.mirrored {
            for node in &mut scene.nodes {
                // H * R(theta) * S = R(-theta) * S(-sx, sy).
                node.transform.translation.x = f64::from(self.size) - node.transform.translation.x;
                node.transform.scale.x = -node.transform.scale.x;
                node.transform.rotation_radians = -node.transform.rotation_radians;
            }
        }
    }
    pub fn unproject(&self, mut point: Vec2) -> Vec2 {
        if self.mirrored {
            point.x = f64::from(self.size) - point.x;
        }
        point
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_core::{SceneContent, SceneNode, Transform};
    fn motion() -> Motion {
        Motion::new(
            [0, 0],
            Bounds {
                origin: [-1000; 2],
                size: [2000; 2],
            },
            256,
        )
    }
    #[test]
    fn diagonal_speed_and_fractional_position_do_not_depend_on_step_size() {
        let mut diagonal = motion();
        let mut axis = motion();
        for _ in 0..60 {
            diagonal.advance([1, -1], SPEED / 60.0);
            axis.advance([1, 0], SPEED / 60.0);
        }
        assert!((diagonal.position[0].hypot(diagonal.position[1]) - SPEED).abs() < 1e-9);
        let mut single = motion();
        single.advance([1, -1], SPEED);
        assert_eq!(diagonal.origin(), single.origin());
        assert_eq!(axis.origin(), [96, 0]);
    }
    #[test]
    fn opposite_keys_cancel_and_releasing_one_resumes_the_other() {
        let mut keys = Keys::default();
        assert!(keys.press(113, 0xff51, true));
        assert!(!keys.press(113, 0xff51, true));
        keys.press(114, 0xff53, true);
        assert_eq!(keys.direction(), [0, 0]);
        keys.release(114);
        assert_eq!(keys.direction(), [-1, 0]);
        keys.release(113);
        assert_eq!(keys.direction(), [0, 0]);
    }
    #[test]
    fn cancellation_and_focus_never_rearm_a_held_key_from_repeat() {
        let mut keys = Keys::default();
        keys.press(113, 0xff51, true);
        keys.cancel_motion();
        assert!(!keys.press(113, 0xff51, true));
        assert_eq!(keys.direction(), [0, 0]);
        keys.release(113);
        assert!(keys.press(113, 0xff51, true));
        let mut bitmap = [0; 32];
        bitmap[113 / 8] = 1 << (113 % 8);
        keys.focus(bitmap);
        assert!(!keys.press(113, 0xff51, true));
        assert_eq!(keys.direction(), [0, 0]);
    }
    #[test]
    fn boundaries_stop_without_bounce_or_accumulated_overshoot() {
        let mut m = motion();
        assert!(m.advance([-1, -1], 5000.0));
        assert_eq!(m.origin(), [-1000, -1000]);
        assert!(!m.advance([-1, -1], 5000.0));
        assert!(m.advance([1, 0], 1.0));
        assert_eq!(m.origin(), [-999, -1000]);
        assert!(!m.mirrored);
        m.advance([0, 1], 1.0);
        assert!(!m.mirrored);
        m.advance([1, 1], 5000.0);
        assert_eq!(m.origin(), [744, 744]);
    }
    #[test]
    fn oversized_window_and_off_monitor_drag_have_valid_bounds() {
        let mut m = Motion::new(
            [800, 900],
            Bounds {
                origin: [-400, 0],
                size: [128, 128],
            },
            256,
        );
        m.advance([1, 0], 2.0);
        assert_eq!(m.origin(), [-400, 0]);
        assert_eq!(m.bounds.distance_squared([-450.0, 0.0]), 2500.0);
    }
    #[test]
    fn mirrored_rotated_scene_and_pointer_share_the_same_coordinates() {
        let mut m = motion();
        m.advance([-1, 0], 1.0);
        let mut scene = Scene {
            nodes: vec![SceneNode {
                id: "body".into(),
                opacity: 1.0,
                transform: Transform {
                    translation: Vec2::new(30.0, 40.0),
                    scale: Vec2::new(2.0, 3.0),
                    rotation_radians: 0.7,
                },
                content: SceneContent::Text {
                    text: "x".into(),
                    font_size: 10.0,
                },
            }],
        };
        let original = scene.clone();
        m.project(&mut scene);
        let p = Vec2::new(4.0, 8.0);
        let apply = |t: &Transform| {
            Vec2::new(
                t.translation.x + t.rotation_radians.cos() * t.scale.x * p.x
                    - t.rotation_radians.sin() * t.scale.y * p.y,
                t.translation.y
                    + t.rotation_radians.sin() * t.scale.x * p.x
                    + t.rotation_radians.cos() * t.scale.y * p.y,
            )
        };
        let restored = m.unproject(apply(&scene.nodes[0].transform));
        let expected = apply(&original.nodes[0].transform);
        assert!((restored.x - expected.x).abs() < 1e-10);
        assert!((restored.y - expected.y).abs() < 1e-10);
        m.project(&mut scene);
        assert_eq!(scene, original);
    }
}
