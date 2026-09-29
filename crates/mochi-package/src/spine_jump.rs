//! Optional original-clip jump with bounded flight, contact and eyelid timing.
#[cfg(feature = "spine")]
use crate::{error, PackageError};
use crate::{ShakeClip, SpineFollow};
use mochi_core::FootPlant;
#[cfg(feature = "spine")]
use mochi_core::{AnimationLayer, LocalRotation, PoseOverlay};
#[cfg(feature = "spine")]
use mochi_spine::{Evaluator, Model};
use serde::Deserialize;
#[cfg(feature = "spine")]
use std::collections::BTreeSet;
macro_rules! record {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Deserialize)]
        #[serde(rename_all="camelCase", deny_unknown_fields)]
        pub struct $name { $(pub $field: $ty),* }
    };
}
record!(SpineJump {version:u32,duration:f64,entry_seconds:f64,exit_start:f64,cancel_seconds:f64,
    flight:JumpFlight,source_curve:Vec<[f64;2]>,crouch_curve:Vec<[f64;2]>,secondary_curve:Vec<[f64;2]>,
    secondary:Vec<JumpSecondary>,hip_bone:String,foot_bones:[String;2],body:ShakeClip,face:ShakeClip,
    face_timing:JumpFaceTiming,face_curve:Vec<[f64;2]>,mouth_source:JumpMouthSource,mouth_slots:Vec<String>,eye_highlights:JumpHighlights});
record!(JumpFlight {
    takeoff: f64,
    landing: f64,
    height: f64,
    tuck_height: f64,
    tuck_x: [f64; 2]
});
record!(JumpSecondary {
    bone: String,
    degrees: f64
});
record!(JumpFaceTiming {
    start: f64,
    r#in: f64,
    hold_until: f64,
    out: f64,
    swap_threshold: f64
});
record!(JumpMouthSource {
    animation: String,
    time: f64
});
record!(JumpHighlights {
    slots: [String; 2],
    fade_source_start: f64,
    hidden_source_at: f64
});
impl SpineJump {
    pub fn plant(&self, follow: &SpineFollow, anchor_time: f64) -> FootPlant {
        FootPlant {
            hip: self.hip_bone.clone(),
            feet: self.foot_bones.clone(),
            anchor_animation: follow.animation.clone(),
            anchor_time,
            reference_time: self.body.start,
            crouch: 0.0,
            tuck: 0.0,
            tuck_height: self.flight.tuck_height,
            tuck_x: self.flight.tuck_x,
        }
    }
    #[cfg(feature = "spine")]
    pub(crate) fn validate(&self, model: &Model, follow: &SpineFollow) -> Result<(), PackageError> {
        let bad = || error("invalid Spine jump rig");
        let bounded = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        let f = &self.flight;
        let t = &self.face_timing;
        let h = &self.eye_highlights;
        if self.version != 1
            || !bounded(self.duration, 0.1, 5.0)
            || !bounded(self.entry_seconds, 0.001, f.takeoff)
            || !bounded(f.takeoff, self.entry_seconds, f.landing - 0.001)
            || !bounded(f.landing, f.takeoff + 0.001, self.exit_start)
            || !bounded(self.exit_start, f.landing, self.duration - 0.001)
            || !bounded(self.cancel_seconds, 0.001, 1.0)
            || !bounded(f.height, 0.1, 128.0)
            || !bounded(f.tuck_height, 0.0, 24.0)
            || f.tuck_x.iter().any(|&v| !bounded(v, -24.0, 24.0))
            || !bounded(t.start, 0.0, f.takeoff)
            || !bounded(t.r#in, 0.001, t.hold_until - t.start)
            || !bounded(t.hold_until, t.start + t.r#in, f.landing)
            || !bounded(t.out, 0.001, f.landing - t.hold_until)
            || t.hold_until + t.out >= f.landing
            || !bounded(t.swap_threshold, 0.001, 1.0)
            || self.body.start != 0.0
            || self.body.end > 0.5
            || self.secondary.len() > 8
            || self.mouth_slots.len() != 4
            || self.mouth_slots.iter().collect::<BTreeSet<_>>().len() != 4
            || self
                .mouth_slots
                .iter()
                .any(|s| !self.face.slots.contains(s))
            || self.face.draw_order
            || h.slots[0] == h.slots[1]
            || h.slots.iter().any(|s| !self.face.slots.contains(s))
            || !bounded(h.fade_source_start, self.face.start, self.face.end)
            || !bounded(
                h.hidden_source_at,
                h.fade_source_start + 0.001,
                self.face.end,
            )
        {
            return Err(bad());
        }
        for (keys, lo, hi) in [
            (&self.source_curve, self.body.start, self.body.end),
            (&self.crouch_curve, -32.0, 32.0),
            (&self.secondary_curve, -1.0, 1.0),
            (&self.face_curve, self.face.start, self.face.end),
        ] {
            if !(2..=32).contains(&keys.len())
                || keys
                    .iter()
                    .any(|k| !bounded(k[0], 0.0, self.duration) || !bounded(k[1], lo, hi))
                || keys.windows(2).any(|w| w[0][0] >= w[1][0])
            {
                return Err(bad());
            }
        }
        if self.source_curve.windows(2).any(|w| w[0][1] > w[1][1]) {
            return Err(bad());
        }
        for c in [&self.body, &self.face] {
            let Some(a) = model
                .metadata
                .animations
                .iter()
                .find(|a| a.name == c.animation)
            else {
                return Err(bad());
            };
            if !bounded(c.start, 0.0, a.duration)
                || !bounded(c.end, c.start + 0.001, a.duration)
                || c.bones.iter().any(|b| {
                    !b.name.starts_with("F_")
                        || !model.metadata.bone_names.contains(&b.name)
                        || b.channels.is_empty()
                        || b.channels.len() > 3
                        || b.channels
                            .iter()
                            .any(|c| !["x", "y", "rotation"].contains(&c.as_str()))
                        || b.channels.iter().collect::<BTreeSet<_>>().len() != b.channels.len()
                })
                || c.slots
                    .iter()
                    .any(|s| !model.metadata.slot_names.contains(s))
            {
                return Err(bad());
            }
        }
        if !model.metadata.animations.iter().any(|a| {
            a.name == self.mouth_source.animation
                && bounded(self.mouth_source.time, 0.0, a.duration)
        }) || self
            .secondary
            .iter()
            .any(|b| !model.metadata.bone_names.contains(&b.bone) || !bounded(b.degrees, -5.0, 5.0))
        {
            return Err(bad());
        }
        let mut evaluator = Evaluator::new(model).map_err(error)?;
        for time in [
            self.body.start,
            (self.body.start + self.body.end) / 2.0,
            self.body.end,
        ] {
            let mut body = self.body.layer(time, 1.0);
            body.foot_plant = Some(self.plant(follow, follow.base_time_offset));
            body.rotations = self
                .secondary
                .iter()
                .map(|r| LocalRotation {
                    bone: r.bone.clone(),
                    degrees: r.degrees,
                })
                .collect();
            let mut face = self.face.layer(self.face.end, 1.0);
            face.slots.retain(|s| !self.mouth_slots.contains(s));
            let mouth = AnimationLayer {
                animation: self.mouth_source.animation.clone(),
                time: self.mouth_source.time,
                weight: 1.0,
                attachment_threshold: t.swap_threshold,
                slots: self.mouth_slots.clone(),
                ..Default::default()
            };
            let overlay = PoseOverlay {
                layers: vec![body, face, mouth],
                ..Default::default()
            };
            evaluator
                .pose_with_overlay(&follow.animation, follow.base_time_offset, &[], &overlay)
                .map_err(error)?;
        }
        Ok(())
    }
}
