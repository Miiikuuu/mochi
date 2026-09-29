//! Bounded, data-only local pose overlays. No renderer or animation runtime.
use crate::{SchemaError, Vec2};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PoseOverlay {
    /// Deltas on the fully constrained local pose; scale/shear stay unchanged.
    pub applied: Option<Box<AppliedPose>>,
    pub gaze: Option<crate::GazePose>,
    pub bones: Vec<BonePose>,
    pub slots: Vec<SlotPose>,
    /// Original animation channels, applied in order after additive controls.
    pub layers: Vec<AnimationLayer>,
    /// Small procedural eye symbols, still subject to the original clipping.
    pub spirals: Vec<SpiralEye>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedPose {
    pub bones: Vec<AppliedDelta>,
    /// Preserve this hand's point in weapon-local coordinates by translation.
    pub grip: [String; 2],
}
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedDelta {
    pub bone: String,
    pub translation: Vec2,
    pub rotation_degrees: f64,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimationLayer {
    pub animation: String,
    pub time: f64,
    pub weight: f64,
    pub attachment_threshold: f64,
    pub bones: Vec<BoneChannels>,
    pub slots: Vec<String>,
    pub draw_order: bool,
    pub foot_plant: Option<FootPlant>,
    pub rotations: Vec<LocalRotation>,
    pub slot_alpha: Vec<SlotAlpha>,
}
/// Retain contact from an original pose while replaying a body's local channels.
#[derive(Debug, Clone, PartialEq)]
pub struct FootPlant {
    pub hip: String,
    pub feet: [String; 2],
    pub anchor_animation: String,
    pub anchor_time: f64,
    pub reference_time: f64,
    pub crouch: f64,
    pub tuck: f64,
    pub tuck_height: f64,
    pub tuck_x: [f64; 2],
}
#[derive(Debug, Clone, PartialEq)]
pub struct LocalRotation {
    pub bone: String,
    pub degrees: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SlotAlpha {
    pub slot: String,
    pub alpha: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BoneChannels {
    pub bone: String,
    /// x, y, rotation. Scale and shear cannot be copied.
    pub channels: [bool; 3],
}
#[derive(Debug, Clone, PartialEq)]
pub struct SpiralEye {
    pub texture: String,
    pub slot: String,
    pub highlight: String,
    pub clip: String,
    pub center: Vec2,
    pub radius: f64,
    pub stroke: f64,
    pub rotation: f64,
    pub turns: f64,
    pub segments: u16,
    pub cap_segments: u16,
    pub inner_radius: f64,
    pub color: [f64; 4],
    pub closed: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct BonePose {
    pub bone: String,
    pub translation: Vec2,
    pub rotation_degrees: f64,
    pub weight: f64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SlotPose {
    pub slot: String,
    /// None hides the slot; Some selects an existing attachment.
    pub attachment: Option<String>,
    /// Absolute local vertices of an unweighted mesh, blended with the base.
    pub vertices: Vec<f64>,
    pub weight: f64,
}
impl PoseOverlay {
    pub fn validate(&self) -> Result<(), SchemaError> {
        if let Some(gaze) = &self.gaze {
            gaze.validate()?;
        }
        let name = |s: &str| !s.is_empty() && s.len() <= 256 && !s.contains('\0');
        let bounded = |v: f64, limit: f64| v.is_finite() && v.abs() <= limit;
        let weight = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        let mut bones = std::collections::BTreeSet::new();
        let mut slots = std::collections::BTreeSet::new();
        if let Some(p) = &self.applied {
            let mut names = std::collections::BTreeSet::new();
            if p.bones.is_empty()
                || p.bones.len() > 128
                || p.grip.iter().any(|s| !name(s))
                || p.grip[0] == p.grip[1]
                || p.bones.iter().any(|b| {
                    !name(&b.bone)
                        || !names.insert(&b.bone)
                        || !bounded(b.translation.x, 256.0)
                        || !bounded(b.translation.y, 256.0)
                        || !bounded(b.rotation_degrees, 180.0)
                })
            {
                return Err(SchemaError("invalid applied pose delta".into()));
            }
        }
        if self.layers.len() > 3 || self.spirals.len() > 2 {
            return Err(SchemaError("too many skeletal effects".into()));
        }
        for layer in &self.layers {
            let mut bones = std::collections::BTreeSet::new();
            let mut slots = std::collections::BTreeSet::new();
            if !name(&layer.animation)
                || !bounded(layer.time, 3600.0)
                || layer.time < 0.0
                || !weight(layer.weight)
                || !weight(layer.attachment_threshold)
                || layer.attachment_threshold == 0.0
                || layer.bones.len() > 192
                || layer.slots.len() > 40
                || layer.bones.iter().any(|b| {
                    !name(&b.bone) || !bones.insert(&b.bone) || !b.channels.iter().any(|c| *c)
                })
                || layer.slots.iter().any(|s| !name(s) || !slots.insert(s))
            {
                return Err(SchemaError("invalid original animation layer".into()));
            }
            if let Some(p) = &layer.foot_plant {
                if [&p.hip, &p.feet[0], &p.feet[1], &p.anchor_animation]
                    .into_iter()
                    .any(|n| !name(n))
                    || p.feet[0] == p.feet[1]
                    || p.feet.contains(&p.hip)
                    || [p.anchor_time, p.reference_time]
                        .iter()
                        .any(|&t| !bounded(t, 3600.0) || t < 0.0)
                    || !bounded(p.crouch, 32.0)
                    || !weight(p.tuck)
                    || !bounded(p.tuck_height, 24.0)
                    || p.tuck_height < 0.0
                    || p.tuck_x.iter().any(|&v| !bounded(v, 24.0))
                {
                    return Err(SchemaError("invalid foot contact overlay".into()));
                }
            }
            let mut rotations = std::collections::BTreeSet::new();
            let mut alphas = std::collections::BTreeSet::new();
            if layer.rotations.len() > 8
                || layer.slot_alpha.len() > 8
                || layer.rotations.iter().any(|r| {
                    !name(&r.bone) || !bounded(r.degrees, 5.0) || !rotations.insert(&r.bone)
                })
                || layer
                    .slot_alpha
                    .iter()
                    .any(|s| !name(&s.slot) || !weight(s.alpha) || !alphas.insert(&s.slot))
            {
                return Err(SchemaError("invalid animation layer accents".into()));
            }
        }
        let mut eyes = std::collections::BTreeSet::new();
        for eye in &self.spirals {
            if [&eye.texture, &eye.slot, &eye.highlight, &eye.clip]
                .into_iter()
                .any(|n| !name(n))
                || !eyes.insert(&eye.slot)
                || eye.slot == eye.highlight
                || eye.slot == eye.clip
                || eye.highlight == eye.clip
                || !bounded(eye.center.x, 12.0)
                || !bounded(eye.center.y, 12.0)
                || !bounded(eye.rotation, 1e6)
                || !bounded(eye.radius, 12.0)
                || eye.radius <= 0.0
                || !bounded(eye.stroke, 2.2)
                || eye.stroke <= 0.0
                || !(1.0..=3.0).contains(&eye.turns)
                || !(32..=160).contains(&eye.segments)
                || !(3..=24).contains(&eye.cap_segments)
                || !eye.inner_radius.is_finite()
                || eye.inner_radius <= 0.0
                || eye.inner_radius >= eye.radius
                || eye.color.iter().any(|c| !weight(*c))
            {
                return Err(SchemaError("invalid spiral eye".into()));
            }
        }
        if self.bones.len() > 8
            || self.slots.len() > 8
            || self.bones.iter().any(|b| {
                !name(&b.bone)
                    || !bones.insert(&b.bone)
                    || !bounded(b.translation.x, 10000.0)
                    || !bounded(b.translation.y, 10000.0)
                    || !bounded(b.rotation_degrees, 360.0)
                    || !weight(b.weight)
            })
            || self.slots.iter().any(|s| {
                !name(&s.slot)
                    || !slots.insert(&s.slot)
                    || s.attachment.as_ref().is_some_and(|a| !name(a))
                    || !weight(s.weight)
                    || s.vertices.len() > 128
                    || s.vertices.len() % 2 != 0
                    || s.vertices.iter().any(|&v| !bounded(v, 10000.0))
                    || (s.attachment.is_none() && !s.vertices.is_empty())
            })
        {
            return Err(SchemaError("invalid skeletal pose overlay".into()));
        }
        Ok(())
    }
}
