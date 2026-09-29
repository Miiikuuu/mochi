//! Optional data-only gaze rig. No package scripts are evaluated.
#[cfg(feature = "spine")]
use crate::{error, PackageError};
#[cfg(feature = "spine")]
use mochi_spine::Model;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpineFollow {
    pub animation: String,
    pub base_time_offset: f64,
    pub cycle_duration: f64,
    pub focus: [f64; 2],
    pub radius: [f64; 2],
    #[serde(default)]
    pub gaze_radius: Option<[f64; 2]>,
    #[serde(default)]
    pub gaze_dead_zone: Option<f64>,
    #[serde(default)]
    pub native_eyes: Option<NativeEyes>,
    pub dead_zone: f64,
    pub gaze_bones: [String; 2],
    pub gaze_limit: [f64; 2],
    pub eye_omega: f64,
    pub head_bone: String,
    pub head_max_degrees: f64,
    pub head_omega: f64,
    pub head_delay: f64,
    pub torso_bone: String,
    pub torso_counter_ratio: f64,
    pub blink_weight: Vec<[f64; 2]>,
    pub secondary: Vec<FollowSecondary>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEyes {
    pub closure: [FollowClosure; 2],
    pub pupil_center: Option<FollowPupilCenter>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowClosure {
    pub bone: String,
    pub minimum: f64,
    pub open: f64,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowPupilCenter {
    pub max_offset: [f64; 2],
    pub triangle_area_ratio: [f64; 2],
    pub eyes: [FollowPupil; 2],
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowPupil {
    pub slot: String,
    pub attachment: String,
    pub vertex_count: u16,
    pub hull_count: u16,
    pub center_vertices: [u16; 4],
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowSecondary {
    pub bone: String,
    pub delay: f64,
    pub gain: f64,
    pub limit_degrees: f64,
}
impl SpineFollow {
    pub fn gaze_pose(
        &self,
        rotations: Vec<mochi_core::LocalRotation>,
        raw: [f64; 2],
        blink: f64,
        weight: f64,
    ) -> Option<mochi_core::GazePose> {
        use mochi_core::{EyeClosure, GazeEye, GazePose, PupilCenter, Vec2};
        let c = self.native_eyes.as_ref()?;
        let mut direction = std::array::from_fn::<_, 2, _>(|i| {
            if self.gaze_limit[i] > 0.0 {
                (raw[i] / self.gaze_limit[i]).clamp(-1.0, 1.0)
            } else {
                0.0
            }
        });
        let length = direction[0].hypot(direction[1]);
        if length > 1.0 {
            direction = direction.map(|v| v / length);
        }
        Some(GazePose {
            rotations,
            eyes: (0..2)
                .map(|i| GazeEye {
                    bone: self.gaze_bones[i].clone(),
                    closure: EyeClosure {
                        bone: c.closure[i].bone.clone(),
                        minimum: c.closure[i].minimum,
                        open: c.closure[i].open,
                    },
                    translation: Vec2::new(raw[0] * blink * weight, raw[1] * blink * weight),
                    pupil: c.pupil_center.as_ref().map(|p| PupilCenter {
                        slot: p.eyes[i].slot.clone(),
                        attachment: p.eyes[i].attachment.clone(),
                        vertex_count: p.eyes[i].vertex_count,
                        hull_count: p.eyes[i].hull_count,
                        vertices: p.eyes[i].center_vertices,
                        translation: Vec2::new(
                            direction[0] * p.max_offset[0] * weight,
                            direction[1] * p.max_offset[1] * weight,
                        ),
                        area_ratio: p.triangle_area_ratio,
                    }),
                })
                .collect(),
        })
    }
    #[cfg(feature = "spine")]
    pub(crate) fn validate(&self, model: &Model) -> Result<(), PackageError> {
        let bounded = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        if !bounded(self.base_time_offset, 0.0, 3599.0)
            || !bounded(self.cycle_duration, 0.1, 60.0)
            || !model.metadata.animations.iter().any(|a| {
                a.name == self.animation
                    && self.base_time_offset + self.cycle_duration <= a.duration + 1e-6
            })
            || self.focus.iter().any(|&v| !bounded(v, -1e6, 1e6))
            || self.radius.iter().any(|&v| !bounded(v, 1.0, 1e6))
            || self
                .gaze_radius
                .is_some_and(|r| r.iter().any(|&v| !bounded(v, 1.0, 1e6)))
            || self.gaze_dead_zone.is_some_and(|v| !bounded(v, 0.0, 0.5))
            || !bounded(self.dead_zone, 0.0, 0.5)
            || self.gaze_limit.iter().enumerate().any(|(i, &v)| {
                !bounded(
                    v,
                    0.0,
                    if self.native_eyes.is_some() {
                        [6.0, 4.0][i]
                    } else {
                        5.0
                    },
                )
            })
            || !bounded(self.eye_omega, 1.0, 100.0)
            || !bounded(self.head_omega, 1.0, 100.0)
            || !bounded(self.head_max_degrees, 0.0, 20.0)
            || !bounded(self.head_delay, 0.0, 0.5)
            || !bounded(self.torso_counter_ratio, 0.0, 1.0)
            || self.secondary.len() > 8
            || self.secondary.iter().any(|s| {
                !bounded(s.delay, 0.0, 0.5)
                    || !bounded(s.gain, 0.0, 2.0)
                    || !bounded(s.limit_degrees, 0.01, 10.0)
            })
            || !(2..=32).contains(&self.blink_weight.len())
            || self.blink_weight.first().is_none_or(|k| k[0] != 0.0)
            || self
                .blink_weight
                .last()
                .is_none_or(|k| k[0] != self.cycle_duration)
            || self
                .blink_weight
                .iter()
                .any(|k| !bounded(k[0], 0.0, self.cycle_duration) || !bounded(k[1], 0.0, 1.0))
            || self.blink_weight.windows(2).any(|p| p[0][0] >= p[1][0])
        {
            return Err(error("invalid Spine follow parameters"));
        }
        let mut names = std::collections::BTreeSet::new();
        for bone in [&self.head_bone, &self.torso_bone]
            .into_iter()
            .chain(self.gaze_bones.iter())
            .chain(self.secondary.iter().map(|s| &s.bone))
        {
            if bone.is_empty()
                || bone.len() > 256
                || bone.contains('\0')
                || !names.insert(bone)
                || !model.metadata.bone_names.contains(bone)
            {
                return Err(error(format!("unknown or duplicate follow bone: {bone}")));
            }
        }
        if let Some(rig) = &self.native_eyes {
            if rig
                .closure
                .iter()
                .any(|g| !model.metadata.bone_names.contains(&g.bone))
                || rig.pupil_center.as_ref().is_some_and(|p| {
                    p.max_offset.iter().any(|&v| !bounded(v, 0.0, 3.0))
                        || p.eyes
                            .iter()
                            .any(|e| !model.metadata.slot_names.contains(&e.slot))
                })
            {
                return Err(error("invalid native eye rig"));
            }
            let gaze = self
                .gaze_pose(vec![], self.gaze_limit, 1.0, 1.0)
                .expect("present");
            let overlay = mochi_core::PoseOverlay {
                gaze: Some(gaze),
                ..Default::default()
            };
            overlay.validate().map_err(error)?;
            mochi_spine::Evaluator::new(model)
                .map_err(error)?
                .pose_with_overlay(&self.animation, self.base_time_offset, &[], &overlay)
                .map_err(error)?;
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "spine"))]
mod tests {
    use super::*;
    #[test]
    fn follow_rejects_invalid_ranges_channels_and_blink_curves() {
        let config = SpineFollow {
            animation: "idle".into(),
            base_time_offset: 0.5,
            cycle_duration: 2.0,
            focus: [0.0, 0.0],
            radius: [100.0, 100.0],
            gaze_radius: None,
            gaze_dead_zone: None,
            native_eyes: None,
            dead_zone: 0.025,
            gaze_bones: ["left".into(), "right".into()],
            gaze_limit: [2.6, 2.2],
            eye_omega: 32.0,
            head_bone: "head".into(),
            head_max_degrees: 5.8,
            head_omega: 13.0,
            head_delay: 0.11,
            torso_bone: "torso".into(),
            torso_counter_ratio: 0.095,
            blink_weight: vec![[0.0, 1.0], [2.0, 1.0]],
            secondary: vec![],
        };
        let model = Model {
            atlas: String::new(),
            skeleton: vec![],
            pages: vec![],
            metadata: mochi_spine::Metadata {
                version: "3.8".into(),
                bones: 4,
                bone_names: ["left", "right", "head", "torso"]
                    .map(String::from)
                    .to_vec(),
                slots: 0,
                slot_names: vec![],
                animations: vec![mochi_spine::Animation {
                    name: "idle".into(),
                    duration: 4.0,
                }],
            },
        };
        assert!(config.validate(&model).is_ok());
        let mutations: [fn(&mut SpineFollow); 10] = [
            |c| c.radius[0] = 0.0,
            |c| c.focus[0] = f64::NAN,
            |c| c.cycle_duration = 5.0,
            |c| c.dead_zone = 1.0,
            |c| c.gaze_limit[1] = 6.0,
            |c| c.head_delay = -0.1,
            |c| c.gaze_bones[0] = "missing".into(),
            |c| c.gaze_bones[0] = c.gaze_bones[1].clone(),
            |c| c.blink_weight = vec![[0.0, 1.0], [0.0, 0.0], [2.0, 1.0]],
            |c| c.blink_weight[1][1] = 2.0,
        ];
        for mutate in mutations {
            let mut bad = config.clone();
            mutate(&mut bad);
            assert!(bad.validate(&model).is_err());
        }
    }
}
