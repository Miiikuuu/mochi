//! Optional, bounded authored drag acting. Imported data only; no package code.
#[cfg(feature = "spine")]
use crate::{error, PackageError};
use mochi_core::{BonePose, PoseOverlay, SlotPose, Vec2};
#[cfg(feature = "spine")]
use mochi_spine::{Evaluator, Model};
use serde::Deserialize;
use std::collections::BTreeMap;
#[cfg(feature = "spine")]
use std::collections::BTreeSet;

macro_rules! record {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Deserialize)]
        #[serde(rename_all="camelCase", deny_unknown_fields)]
        pub struct $name { $(pub $field: $ty),* }
    };
}
record!(SpineDrag {
    version: u32, velocity_reference: f64, follow_omega: f64,
    timings: DragTimings, bones: Vec<DragBone>, feet: Vec<DragFoot>,
    story: DragStory, gaze: DragGaze, landing_glance: LandingGlance, face: DragFace
});
record!(DragTimings {
    pick_in: f64,
    release_in: f64,
    land_recover: f64,
    land_smile_in: f64,
    land_smile_hold: f64,
    expression_out: f64,
    reset_out: f64
});
record!(DragBone {
    bone: String,
    fall_degrees: f64,
    sway_degrees: f64,
    delay: f64,
    limit_degrees: f64
});
record!(DragFoot {
    bone: String,
    fall_offset: [f64; 2],
    sway_x: f64,
    delay: f64,
    limit: [f64; 2]
});
record!(DragStory {poses: BTreeMap<String, DragPose>, keyframes: Vec<DragKey>});
record!(DragPose {rotations: BTreeMap<String,f64>, feet: BTreeMap<String,[f64;2]>, face: [f64;2], gaze_weight:f64});
record!(DragKey {
    time: f64,
    pose: String,
    phase: String
});
record!(DragGaze {
    bones: [String; 2],
    limit: [f64; 2],
    focus: [f64; 2],
    radius: [f64; 2],
    lag_limit: [f64; 2],
    landing: [f64; 2]
});
record!(LandingGlance {start:f64, rise:f64, hold_until:f64, out:f64, rotations:BTreeMap<String,f64>});
record!(DragFace {mouth_slots:Vec<String>, brow_names:Vec<String>, presets:Vec<FacePreset>});
record!(FacePreset {id:String, brow_weight:f64, brows:Vec<FaceBrow>, mouth:Option<FaceMouth>});
record!(FaceBrow {name:String, x:f64, y:f64, rotation:f64, deform:Vec<f64>});
record!(FaceMouth {slot:String, attachment:String, deform:Vec<f64>});
impl DragFace {
    pub fn overlay(&self, face: [f64; 2]) -> PoseOverlay {
        let weights = [1.0 - face[0] - face[1], face[0], face[1]];
        let ids = ["neutral", "surprised", "happy"];
        let mut out = PoseOverlay::default();
        for name in &self.brow_names {
            let mut weight = 0.0;
            let mut xy = [0.0; 2];
            let mut angle = 0.0;
            let mut vertices = Vec::new();
            for (id, w) in ids.iter().zip(weights) {
                let preset = self
                    .presets
                    .iter()
                    .find(|p| p.id == *id)
                    .expect("validated preset");
                let w = w * preset.brow_weight;
                let brow = preset
                    .brows
                    .iter()
                    .find(|b| &b.name == name)
                    .expect("validated brow");
                vertices.resize(brow.deform.len(), 0.0);
                weight += w;
                xy[0] += brow.x * w;
                xy[1] += brow.y * w;
                angle += brow.rotation * w;
                for (v, b) in vertices.iter_mut().zip(&brow.deform) {
                    *v += b * w;
                }
            }
            if weight > 0.0 {
                out.bones.push(BonePose {
                    bone: name.clone(),
                    translation: Vec2::new(xy[0] / weight, xy[1] / weight),
                    rotation_degrees: angle / weight,
                    weight,
                });
                out.slots.push(SlotPose {
                    slot: name.clone(),
                    attachment: Some(name.clone()),
                    vertices: vertices.into_iter().map(|v| v / weight).collect(),
                    weight,
                });
            }
        }
        let id = if face[0] + face[1] < 0.5 {
            "neutral"
        } else if face[0] > face[1] {
            "surprised"
        } else {
            "happy"
        };
        let Some(mouth) = &self
            .presets
            .iter()
            .find(|p| p.id == id)
            .expect("validated mouth")
            .mouth
        else {
            return out;
        };
        for name in &self.mouth_slots {
            let active = name == &mouth.slot;
            out.slots.push(SlotPose {
                slot: name.clone(),
                attachment: active.then(|| mouth.attachment.clone()),
                vertices: if active { mouth.deform.clone() } else { vec![] },
                weight: 1.0,
            });
        }
        out
    }
}
impl SpineDrag {
    #[cfg(feature = "spine")]
    pub(crate) fn validate(&self, model: &Model) -> Result<(), PackageError> {
        let bad = || error("invalid Spine drag rig");
        let bounded = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        let time = |v: f64| bounded(v, 0.001, 10.0);
        let t = &self.timings;
        let l = &self.landing_glance;
        let g = &self.gaze;
        if self.version != 2
            || !bounded(self.velocity_reference, 1.0, 10000.0)
            || !bounded(self.follow_omega, 1.0, 100.0)
            || [
                t.pick_in,
                t.release_in,
                t.land_recover,
                t.land_smile_in,
                t.land_smile_hold,
                t.expression_out,
                t.reset_out,
                l.rise,
                l.out,
            ]
            .into_iter()
            .any(|v| !time(v))
            || !bounded(l.start, 0.0, 10.0)
            || !bounded(l.hold_until, l.start + l.rise, 20.0)
            || !(1..=16).contains(&self.bones.len())
            || self.feet.len() > 4
            || !(1..=16).contains(&self.story.poses.len())
            || !(1..=64).contains(&self.story.keyframes.len())
            || self.story.keyframes.iter().any(|k| {
                !bounded(k.time, 0.001, 60.0)
                    || !self.story.poses.contains_key(&k.pose)
                    || k.phase.len() > 64
            })
            || self
                .story
                .keyframes
                .windows(2)
                .any(|w| w[0].time >= w[1].time)
            || g.limit.iter().any(|&v| !bounded(v, 0.01, 5.0))
            || g.radius.iter().any(|&v| !bounded(v, 1.0, 1e6))
            || g.focus.iter().any(|&v| !bounded(v, -1e6, 1e6))
            || g.lag_limit.iter().any(|&v| !bounded(v, 0.0, 1000.0))
            || g.landing
                .iter()
                .enumerate()
                .any(|(i, &v)| !bounded(v, -g.limit[i], g.limit[i]))
        {
            return Err(bad());
        }
        let mut names = BTreeSet::new();
        for name in self
            .bones
            .iter()
            .map(|b| &b.bone)
            .chain(self.feet.iter().map(|b| &b.bone))
            .chain(g.bones.iter())
        {
            if !names.insert(name) || !model.metadata.bone_names.contains(name) {
                return Err(bad());
            }
        }
        for b in &self.bones {
            if !bounded(b.limit_degrees, 0.01, 30.0)
                || !bounded(b.delay, 0.0, 0.5)
                || !bounded(b.sway_degrees, -b.limit_degrees, b.limit_degrees)
                || !bounded(b.fall_degrees, -b.limit_degrees, b.limit_degrees)
            {
                return Err(bad());
            }
        }
        for b in &self.feet {
            if !bounded(b.delay, 0.0, 0.5)
                || !bounded(b.sway_x, -32.0, 32.0)
                || b.limit.iter().any(|&v| !bounded(v, 0.01, 32.0))
                || b.fall_offset
                    .iter()
                    .enumerate()
                    .any(|(i, &v)| !bounded(v, -b.limit[i], b.limit[i]))
            {
                return Err(bad());
            }
        }
        for (name, angle) in &l.rotations {
            if !self
                .bones
                .iter()
                .any(|b| &b.bone == name && bounded(*angle, -b.limit_degrees, b.limit_degrees))
            {
                return Err(bad());
            }
        }
        for p in self.story.poses.values() {
            if p.rotations.len() != self.bones.len()
                || p.feet.len() != self.feet.len()
                || p.face.iter().any(|&v| !bounded(v, 0.0, 1.0))
                || p.face.iter().sum::<f64>() > 1.0
                || !bounded(p.gaze_weight, 0.0, 1.0)
                || self.bones.iter().any(|b| {
                    p.rotations.get(&b.bone).is_none_or(|&v| {
                        !bounded(
                            v,
                            -b.limit_degrees + b.sway_degrees.abs(),
                            b.limit_degrees - b.sway_degrees.abs(),
                        )
                    })
                })
                || self.feet.iter().any(|b| {
                    p.feet.get(&b.bone).is_none_or(|v| {
                        v.iter()
                            .enumerate()
                            .any(|(i, &v)| !bounded(v, -b.limit[i], b.limit[i]))
                    })
                })
            {
                return Err(bad());
            }
        }
        let face = &self.face;
        if face.presets.len() != 3
            || face.brow_names.len() != 2
            || !(1..=6).contains(&face.mouth_slots.len())
        {
            return Err(bad());
        }
        let mut slots = BTreeSet::new();
        for n in face.brow_names.iter().chain(&face.mouth_slots) {
            if !slots.insert(n) || !model.metadata.slot_names.contains(n) {
                return Err(bad());
            }
        }
        let mut ids = BTreeSet::new();
        for p in &face.presets {
            if !["neutral", "surprised", "happy"].contains(&p.id.as_str())
                || !ids.insert(&p.id)
                || !bounded(p.brow_weight, 0.0, 1.0)
                || p.brows.len() != 2
                || p.mouth
                    .as_ref()
                    .is_some_and(|m| !face.mouth_slots.contains(&m.slot))
            {
                return Err(bad());
            }
            let mut brows = BTreeSet::new();
            for b in &p.brows {
                if !brows.insert(&b.name)
                    || !face.brow_names.contains(&b.name)
                    || !model.metadata.bone_names.contains(&b.name)
                    || [b.x, b.y].iter().any(|&v| !bounded(v, -10000.0, 10000.0))
                    || !bounded(b.rotation, -360.0, 360.0)
                    || b.deform.is_empty()
                    || b.deform.len() > 128
                    || b.deform.iter().any(|&v| !bounded(v, -10000.0, 10000.0))
                {
                    return Err(bad());
                }
            }
        }
        for n in &face.brow_names {
            let lengths: BTreeSet<_> = face
                .presets
                .iter()
                .flat_map(|p| {
                    p.brows
                        .iter()
                        .filter(|b| &b.name == n)
                        .map(|b| b.deform.len())
                })
                .collect();
            if lengths.len() != 1 {
                return Err(bad());
            }
        }
        // Resolve actual attachment topology in the bounded evaluator before any
        // package can reach the desktop. Names alone do not prove mesh safety.
        let mut evaluator = Evaluator::new(model).map_err(error)?;
        let animation = &model.metadata.animations.first().ok_or_else(bad)?.name;
        for weights in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]] {
            evaluator
                .pose_with_overlay(animation, 0.0, &[], &face.overlay(weights))
                .map_err(error)?;
        }
        Ok(())
    }
}
