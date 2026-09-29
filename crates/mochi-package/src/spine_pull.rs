//! Data-only, bounded velocity-driven pull and pickup controls.
#[cfg(feature = "spine")]
use crate::{error, PackageError, SpineFollow};
#[cfg(feature = "spine")]
use mochi_spine::Model;
use serde::Deserialize;
use std::collections::BTreeMap;
#[cfg(feature = "spine")]
use std::collections::BTreeSet;
macro_rules! record {
    ($name:ident {$($field:ident:$ty:ty),*$(,)?}) => {
        #[derive(Clone, Debug, Deserialize)]
        #[serde(rename_all="camelCase", deny_unknown_fields)]
        pub struct $name {$(pub $field:$ty),*}
    }
}
record!(SpinePull {
    version:u32, velocity_reference:f64, minimum_input_seconds:f64,
    force_omega:f64, relax_omega:f64, input_hold_seconds:f64,
    pickup_seconds:f64, release_seconds:f64, source_range:[f64;2],
    vertical_weight:f64, follow_omega:f64, lift:PullLift, reaction:PullReaction,
    table:PullTable, grip:PullGrip, direction:Vec<PullDirection>
});
record!(PullTable {times:Vec<f64>, bones:Vec<String>, frames:Vec<Vec<[f64;3]>>});
record!(PullGrip {
    weapon_bone: String,
    hand_bone: String
});
record!(PullDirection {
    bone: String,
    degrees: f64
});
record!(PullLift {velocity_reference:f64, timings:LiftTimings, bones:Vec<LiftBone>, face:LiftFace, gaze:LiftGaze});
record!(LiftTimings {
    pick_in: f64,
    surprise_end: f64,
    release_in: f64,
    recover: f64,
    reset_out: f64
});
record!(LiftBone {bone:String, delay:f64, limit_degrees:f64, sway_degrees:f64, pick:Vec<[f64;2]>, fall_degrees:f64, recovery:Vec<[f64;2]>});
record!(LiftFace {sources:BTreeMap<String,PullExpression>, brow_weight:f64, swap_threshold:f64, pick_surprise:Vec<[f64;2]>, pick_smile:Vec<[f64;2]>, recovery_smile:Vec<[f64;2]>});
record!(PullExpression {
    animation: String,
    time: f64
});
record!(LiftGaze {
    focus: [f64; 2],
    delay: f64,
    held: [f64; 2],
    velocity_gain: f64,
    falling: [f64; 2],
    recovery: [f64; 2],
    limit: [f64; 2]
});
impl SpinePull {
    #[cfg(feature = "spine")]
    pub(crate) fn validate(&self, model: &Model, follow: &SpineFollow) -> Result<(), PackageError> {
        let bad = || error("invalid Spine pull rig");
        let bound = |v: f64, a: f64, b: f64| v.is_finite() && (a..=b).contains(&v);
        let bone = |s: &String| model.metadata.bone_names.contains(s);
        let curve = |v: &[[f64; 2]], limit: f64| {
            !v.is_empty()
                && v.len() <= 32
                && v.iter()
                    .all(|p| bound(p[0], 0.001, 10.0) && bound(p[1], -limit, limit))
                && v.windows(2).all(|w| w[0][0] < w[1][0])
        };
        let r = &self.reaction;
        if !bound(r.velocity_reference, 1.0, 10000.0)
            || r.bones.len() > 8
            || [
                r.timings.pick_in,
                r.timings.release_in,
                r.timings.land_recover,
                r.timings.reset_out,
            ]
            .iter()
            .any(|v| !bound(*v, 0.001, 10.0))
            || r.bones
                .iter()
                .map(|b| &b.bone)
                .collect::<BTreeSet<_>>()
                .len()
                != r.bones.len()
            || r.bones.iter().any(|b| {
                !bone(&b.bone)
                    || !bound(b.delay, 0.0, 0.5)
                    || !bound(b.limit_degrees, 0.01, 4.0)
                    || !bound(
                        b.hold_degrees.abs() + b.sway_degrees.abs(),
                        0.0,
                        b.limit_degrees,
                    )
                    || !bound(b.fall_degrees, -b.limit_degrees, b.limit_degrees)
            })
        {
            return Err(bad());
        }
        let l = &self.lift;
        let t = &l.timings;
        let g = &l.gaze;
        if self.version != 1
            || follow.native_eyes.is_none()
            || !bound(self.velocity_reference, 1.0, 10000.0)
            || !bound(l.velocity_reference, 1.0, 10000.0)
            || [self.force_omega, self.relax_omega, self.follow_omega]
                .iter()
                .any(|v| !bound(*v, 1.0, 100.0))
            || !bound(self.minimum_input_seconds, 0.001, 0.1)
            || !bound(self.input_hold_seconds, 0.0, 1.0)
            || [
                self.pickup_seconds,
                self.release_seconds,
                t.pick_in,
                t.surprise_end,
                t.release_in,
                t.recover,
                t.reset_out,
            ]
            .iter()
            .any(|v| !bound(*v, 0.001, 10.0))
            || !bound(self.vertical_weight, 0.0, 1.0)
            || !bound(self.source_range[0], 0.0, 10.0)
            || !bound(self.source_range[1], self.source_range[0] + 0.001, 10.0)
            || self.table.times.len() < 2
            || self.table.times.len() > 2048
            || self.table.frames.len() != self.table.times.len()
            || self.table.times.first() != Some(&self.source_range[0])
            || self.table.times.last() != Some(&self.source_range[1])
            || self.table.times.iter().any(|v| !bound(*v, 0.0, 10.0))
            || self.table.times.windows(2).any(|w| w[0] >= w[1])
            || self.table.bones.is_empty()
            || self.table.bones.len() > 128
            || self.table.bones.iter().any(|n| !bone(n))
            || self.table.bones.iter().collect::<BTreeSet<_>>().len() != self.table.bones.len()
            || self.table.frames.iter().any(|f| {
                f.len() != self.table.bones.len()
                    || f.iter().any(|v| {
                        !bound(v[0], -256.0, 256.0)
                            || !bound(v[1], -256.0, 256.0)
                            || !bound(v[2], -175.0, 175.0)
                    })
            })
            || !bone(&self.grip.weapon_bone)
            || !bone(&self.grip.hand_bone)
            || self.grip.weapon_bone == self.grip.hand_bone
            || self.direction.len() > 8
            || self
                .direction
                .iter()
                .any(|b| !self.table.bones.contains(&b.bone) || !bound(b.degrees, -5.0, 5.0))
            || self
                .direction
                .iter()
                .map(|b| &b.bone)
                .collect::<BTreeSet<_>>()
                .len()
                != self.direction.len()
            || l.bones.is_empty()
            || l.bones.len() > 16
            || l.bones
                .iter()
                .map(|b| &b.bone)
                .collect::<BTreeSet<_>>()
                .len()
                != l.bones.len()
            || l.bones.iter().any(|b| {
                !bone(&b.bone)
                    || !bound(b.delay, 0.0, 0.5)
                    || !bound(b.limit_degrees, 0.1, 30.0)
                    || !bound(b.sway_degrees, -b.limit_degrees, b.limit_degrees)
                    || !bound(b.fall_degrees, -b.limit_degrees, b.limit_degrees)
                    || !curve(&b.pick, b.limit_degrees - b.sway_degrees.abs())
                    || !curve(&b.recovery, b.limit_degrees - b.sway_degrees.abs())
                    || b.recovery
                        .last()
                        .is_none_or(|k| k[1] != 0.0 || k[0] > t.recover)
            })
            || l.face.brow_weight != 0.0
            || !bound(l.face.swap_threshold, 0.01, 1.0)
            || !curve(&l.face.pick_surprise, 1.0)
            || !curve(&l.face.pick_smile, 1.0)
            || !curve(&l.face.recovery_smile, 1.0)
            || [
                &l.face.pick_surprise,
                &l.face.pick_smile,
                &l.face.recovery_smile,
            ]
            .iter()
            .any(|c| c.iter().any(|k| k[1] < 0.0))
            || l.face.sources.len() != 3
            || ["neutral", "surprised", "happy"]
                .iter()
                .any(|id| !l.face.sources.contains_key(*id))
            || l.face.sources.values().any(|s| {
                !model
                    .metadata
                    .animations
                    .iter()
                    .any(|a| a.name == s.animation && bound(s.time, 0.0, a.duration))
            })
            || !model.metadata.slot_names.iter().any(|s| s == "F_S_Mouth")
            || !bound(g.delay, 0.0, 0.5)
            || !bound(g.velocity_gain, 0.0, 2.0)
            || g.focus.iter().any(|v| !bound(*v, -1e6, 1e6))
            || g.limit.iter().any(|v| !bound(*v, 0.01, 3.0))
            || [g.held, g.falling, g.recovery].iter().any(|v| {
                v.iter()
                    .enumerate()
                    .any(|(i, x)| !bound(*x, -g.limit[i], g.limit[i]))
            })
        {
            return Err(bad());
        }
        Ok(())
    }
}

record!(PullReaction {velocity_reference:f64,timings:ReactionTimings,bones:Vec<ReactionBone>});
record!(ReactionTimings {
    pick_in: f64,
    release_in: f64,
    land_recover: f64,
    reset_out: f64
});
record!(ReactionBone {
    bone: String,
    hold_degrees: f64,
    fall_degrees: f64,
    sway_degrees: f64,
    delay: f64,
    limit_degrees: f64
});
