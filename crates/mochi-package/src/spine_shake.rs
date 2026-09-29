//! Optional finite shake reaction, transported as data and original clip masks.
#[cfg(feature = "spine")]
use crate::{error, PackageError, SpineDrag};
#[cfg(feature = "spine")]
use mochi_core::PoseOverlay;
use mochi_core::{AnimationLayer, BoneChannels, SpiralEye, Vec2};
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
record!(SpineShake {version:u32, detector:ShakeDetector, timings:ShakeTimings,
    frequency:f64, decay:f64, spiral_turns_per_second:f64, mouth_threshold:f64,
    bones:Vec<ShakeBone>, feet:Vec<ShakeFoot>, dizzy:DizzyFace,
    stomp:ShakeClip, face:ShakeClip, post_delay:f64, face_play_seconds:f64,
    bridge_time:f64, stomp_timings:StompTimings});
record!(ShakeDetector {reversals:usize, window_seconds:f64, min_excursion:f64,
    min_speed:f64, reversal_distance:f64, cooldown_seconds:f64, axes:Vec<usize>});
record!(ShakeTimings {
    duration: f64,
    r#in: f64,
    out: f64,
    reset_out: f64
});
record!(StompTimings {
    r#in: f64,
    play_seconds: f64,
    out: f64,
    ground_pause: f64,
    pending_lifetime: f64,
    cancel_out: f64
});
record!(ShakeBone {
    bone: String,
    bias_degrees: f64,
    amplitude_degrees: f64,
    phase: f64,
    delay: f64,
    limit_degrees: f64
});
record!(ShakeFoot {
    bone: String,
    amplitude: [f64; 2],
    phase: f64,
    delay: f64,
    frequency: f64,
    limit: [f64; 2]
});
record!(DizzyFace {texture:String, swap_threshold:f64, color:[f64;4],
    geometry:SpiralGeometry, eyes:Vec<DizzyEye>});
record!(SpiralGeometry {
    turns: f64,
    segments: u16,
    inner_radius: f64,
    cap_segments: u16
});
record!(DizzyEye {
    slot: String,
    highlight_slot: String,
    clip_slot: String,
    center: [f64; 2],
    radius: f64,
    stroke: f64,
    rotation: f64,
    spin_direction: f64
});
record!(ShakeClip {animation:String, start:f64, end:f64, threshold:f64,
    bones:Vec<ShakeChannels>, slots:Vec<String>, draw_order:bool});
record!(ShakeChannels {name:String, channels:Vec<String>});
impl ShakeClip {
    pub fn layer(&self, time: f64, weight: f64) -> AnimationLayer {
        AnimationLayer {
            animation: self.animation.clone(),
            time,
            weight,
            attachment_threshold: self.threshold,
            draw_order: self.draw_order,
            bones: self
                .bones
                .iter()
                .map(|b| BoneChannels {
                    bone: b.name.clone(),
                    channels: ["x", "y", "rotation"].map(|c| b.channels.iter().any(|v| v == c)),
                })
                .collect(),
            slots: self.slots.clone(),
            ..Default::default()
        }
    }
}
impl DizzyFace {
    pub fn eyes(&self, phase: f64, closed: bool) -> Vec<SpiralEye> {
        self.eyes
            .iter()
            .map(|e| SpiralEye {
                texture: self.texture.clone(),
                slot: e.slot.clone(),
                highlight: e.highlight_slot.clone(),
                clip: e.clip_slot.clone(),
                center: Vec2::new(e.center[0], e.center[1]),
                radius: e.radius,
                stroke: e.stroke,
                rotation: e.rotation + e.spin_direction * phase,
                turns: self.geometry.turns,
                segments: self.geometry.segments,
                cap_segments: self.geometry.cap_segments,
                inner_radius: self.geometry.inner_radius,
                color: self.color,
                closed,
            })
            .collect()
    }
}
impl SpineShake {
    #[cfg(feature = "spine")]
    pub(crate) fn validate(&self, model: &Model, drag: &SpineDrag) -> Result<(), PackageError> {
        let bad = || error("invalid Spine shake rig");
        let bound = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        let d = &self.detector;
        let t = &self.timings;
        let s = &self.stomp_timings;
        if self.version != 1
            || !(2..=8).contains(&d.reversals)
            || !bound(d.window_seconds, 0.1, 5.0)
            || !bound(d.min_excursion, 1.0, 1000.0)
            || !bound(d.min_speed, 1.0, 10000.0)
            || !bound(d.reversal_distance, 0.1, d.min_excursion)
            || !bound(d.cooldown_seconds, t.duration, 30.0)
            || d.axes.is_empty()
            || d.axes.len() > 2
            || d.axes.iter().any(|v| *v > 1)
            || d.axes.iter().collect::<BTreeSet<_>>().len() != d.axes.len()
            || [
                t.duration,
                t.r#in,
                t.out,
                t.reset_out,
                s.r#in,
                s.play_seconds,
                s.out,
                s.pending_lifetime,
                s.cancel_out,
                self.face_play_seconds,
            ]
            .iter()
            .any(|&v| !bound(v, 0.001, 30.0))
            || t.r#in + t.out > t.duration
            || s.pending_lifetime < t.duration
            || !bound(self.post_delay, 0.0, 5.0)
            || !bound(s.ground_pause, 0.0, 5.0)
            || !bound(self.frequency, 0.1, 20.0)
            || !bound(self.decay, 0.0, 20.0)
            || !bound(self.spiral_turns_per_second, -5.0, 5.0)
            || !bound(self.dizzy.swap_threshold, 0.01, 1.0)
            || !bound(self.mouth_threshold, self.dizzy.swap_threshold, 1.0)
            || !bound(self.bridge_time, self.face.start, self.face.end)
            || self.bones.len() > 16
            || self.feet.len() > 4
            || self.dizzy.eyes.len() != 2
            || self
                .dizzy
                .eyes
                .iter()
                .any(|e| ![-1.0, 1.0].contains(&e.spin_direction))
        {
            return Err(bad());
        }
        let mut names = BTreeSet::new();
        for b in &self.bones {
            if !names.insert(&b.bone)
                || !drag.bones.iter().any(|v| v.bone == b.bone)
                || !bound(b.limit_degrees, 0.01, 8.0)
                || !bound(b.bias_degrees, -8.0, 8.0)
                || !bound(b.amplitude_degrees, -8.0, 8.0)
                || !bound(b.phase, -10.0, 10.0)
                || !bound(b.delay, 0.0, 0.5)
            {
                return Err(bad());
            }
        }
        for b in &self.feet {
            if !names.insert(&b.bone)
                || !drag.feet.iter().any(|v| v.bone == b.bone)
                || b.limit.iter().any(|&v| !bound(v, 0.01, 8.0))
                || b.amplitude
                    .iter()
                    .enumerate()
                    .any(|(i, &v)| !bound(v, -b.limit[i], b.limit[i]))
                || !bound(b.phase, -10.0, 10.0)
                || !bound(b.delay, 0.0, 0.5)
                || !bound(b.frequency, 0.1, 20.0)
            {
                return Err(bad());
            }
        }
        for c in [&self.stomp, &self.face] {
            let Some(a) = model
                .metadata
                .animations
                .iter()
                .find(|a| a.name == c.animation)
            else {
                return Err(bad());
            };
            if !bound(c.start, 0.0, a.duration)
                || !bound(c.end, c.start + 0.001, a.duration)
                || c.bones.iter().any(|b| {
                    !model.metadata.bone_names.contains(&b.name)
                        || b.channels.len() > 3
                        || b.channels
                            .iter()
                            .any(|s| !["x", "y", "rotation"].contains(&s.as_str()))
                        || b.channels.iter().collect::<BTreeSet<_>>().len() != b.channels.len()
                })
                || c.slots
                    .iter()
                    .any(|n| !model.metadata.slot_names.contains(n))
            {
                return Err(bad());
            }
        }
        if !model
            .pages
            .iter()
            .any(|p| p.asset == self.dizzy.texture && p.width == 1 && p.height == 1)
            || self.face.draw_order
            || drag
                .face
                .mouth_slots
                .iter()
                .any(|s| !self.face.slots.contains(s))
        {
            return Err(bad());
        }
        let mut evaluator = Evaluator::new(model).map_err(error)?;
        let animation = &model.metadata.animations.first().ok_or_else(bad)?.name;
        let mut overlay = PoseOverlay {
            spirals: self.dizzy.eyes(0.0, false),
            ..Default::default()
        };
        evaluator
            .pose_with_overlay(animation, 0.0, &[], &overlay)
            .map_err(error)?;
        overlay.spirals.clear();
        for k in 0..=4 {
            overlay.layers = [&self.stomp, &self.face]
                .map(|c| c.layer(c.start + (c.end - c.start) * k as f64 / 4.0, 1.0))
                .into();
            evaluator
                .pose_with_overlay(animation, 0.0, &[], &overlay)
                .map_err(error)?;
        }
        Ok(())
    }
}
