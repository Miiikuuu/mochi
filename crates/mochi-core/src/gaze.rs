//! Native-eye constraints expressed as bounded data, without runtime objects.
use crate::{LocalRotation, SchemaError, Vec2};
#[derive(Debug, Clone, PartialEq)]
pub struct GazePose {
    /// Added rotations follow the original parent's reflection sign.
    pub rotations: Vec<LocalRotation>,
    pub eyes: Vec<GazeEye>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct GazeEye {
    pub bone: String,
    pub closure: EyeClosure,
    pub translation: Vec2,
    pub pupil: Option<PupilCenter>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct EyeClosure {
    pub bone: String,
    pub minimum: f64,
    pub open: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PupilCenter {
    pub slot: String,
    pub attachment: String,
    pub vertex_count: u16,
    pub hull_count: u16,
    pub vertices: [u16; 4],
    pub translation: Vec2,
    pub area_ratio: [f64; 2],
}
impl GazePose {
    pub fn validate(&self) -> Result<(), SchemaError> {
        let name = |s: &str| !s.is_empty() && s.len() <= 256 && !s.contains('\0');
        let bound = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        let bad = || SchemaError("invalid native gaze overlay".into());
        let mut bones = std::collections::BTreeSet::new();
        let mut slots = std::collections::BTreeSet::new();
        if self.rotations.len() > 10 || self.eyes.len() > 2 {
            return Err(bad());
        }
        for r in &self.rotations {
            if !name(&r.bone) || !bones.insert(&r.bone) || !bound(r.degrees, -20.0, 20.0) {
                return Err(bad());
            }
        }
        for e in &self.eyes {
            let c = &e.closure;
            if !name(&e.bone)
                || !bones.insert(&e.bone)
                || !name(&c.bone)
                || !bound(c.minimum, 0.0, 1.0)
                || !bound(c.open, c.minimum + 0.001, 2.0)
                || !bound(e.translation.x, -6.0, 6.0)
                || !bound(e.translation.y, -4.0, 4.0)
            {
                return Err(bad());
            }
            if let Some(p) = &e.pupil {
                if !name(&p.slot)
                    || !slots.insert(&p.slot)
                    || !name(&p.attachment)
                    || !(7..=32).contains(&p.vertex_count)
                    || !(3..=p.vertex_count - 4).contains(&p.hull_count)
                    || p.vertices
                        .iter()
                        .any(|v| *v < p.hull_count || *v >= p.vertex_count)
                    || p.vertices
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != 4
                    || !bound(p.translation.x, -3.0, 3.0)
                    || !bound(p.translation.y, -3.0, 3.0)
                    || !bound(p.area_ratio[0], 0.01, 0.99)
                    || !bound(p.area_ratio[1], 1.01, 10.0)
                {
                    return Err(bad());
                }
            }
        }
        Ok(())
    }
}
