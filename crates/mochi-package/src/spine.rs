use crate::{animation::AnimationSet, error, DecodedImage, PackageError};
#[cfg(feature = "spine")]
use crate::{
    animation::{TailLoop, TimedClip},
    files,
};
use mochi_core::*;
#[cfg(feature = "spine")]
use mochi_spine::{Model, Page};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpineCamera {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg(feature = "spine")]
struct Document {
    schema_version: u32,
    runtime: String,
    skeleton: String,
    atlas: String,
    alpha: String,
    pages: BTreeMap<String, String>,
    camera: SpineCamera,
    default: String,
    clips: BTreeMap<String, Clip>,
    follow: Option<crate::SpineFollow>,
    drag: Option<crate::SpineDrag>,
    pull: Option<String>,
    shake: Option<crate::SpineShake>,
    jump: Option<crate::SpineJump>,
    idle_action: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg(feature = "spine")]
struct Clip {
    animation: String,
    mode: Mode,
    next: Option<String>,
    end_seconds: Option<f64>,
    loop_tail_from_seconds: Option<f64>,
    loop_tail_half_cycle_seconds: Option<f64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg(feature = "spine")]
enum Mode {
    Loop,
    Once,
}

pub struct SpineResource {
    #[cfg(feature = "spine")]
    pub model: Model,
    pub camera: SpineCamera,
    pub follow: Option<crate::SpineFollow>,
    pub drag: Option<crate::SpineDrag>,
    pub pull: Option<std::sync::Arc<crate::SpinePull>>,
    pub shake: Option<crate::SpineShake>,
    pub jump: Option<crate::SpineJump>,
    /// Optional one-shot action alternated with desktop idle strolls.
    pub idle_action: Option<String>,
}
#[cfg(feature = "spine")]
pub(crate) fn load(
    files: &BTreeMap<String, Vec<u8>>,
    images: &BTreeMap<String, DecodedImage>,
) -> Result<(SpineResource, Arc<AnimationSet>), PackageError> {
    let source = files
        .get("spine.toml")
        .ok_or_else(|| error("missing spine.toml"))?;
    let doc: Document =
        toml::from_str(std::str::from_utf8(source).map_err(error)?).map_err(error)?;
    if doc.schema_version != 1 || doc.runtime != "3.8" || doc.alpha != "premultiplied" {
        return Err(error(
            "Spine requires schema_version=1, runtime=3.8, alpha=premultiplied",
        ));
    }
    let camera = doc.camera;
    if [camera.x, camera.y, camera.width, camera.height]
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
        || camera.width <= 0.0
        || camera.height <= 0.0
    {
        return Err(error("invalid Spine camera"));
    }
    for (key, ext) in [(&doc.skeleton, ".skel"), (&doc.atlas, ".atlas")] {
        files::validate_asset_key(key)?;
        if !key.starts_with("spine/") || !key.ends_with(ext) || !files.contains_key(key) {
            return Err(error("missing or invalid Spine asset"));
        }
    }
    let mut pages = Vec::new();
    for (name, asset) in doc.pages {
        files::validate_asset_key(&name)?;
        files::validate_asset_key(&asset)?;
        let image = images
            .get(&asset)
            .ok_or_else(|| error(format!("missing Spine page: {asset}")))?;
        // This data contract is explicit: do not premultiply a PMA source again.
        if image
            .rgba8()
            .chunks_exact(4)
            .any(|p| p[..3].iter().any(|c| *c > p[3]))
        {
            return Err(error(
                "Spine page declared premultiplied contains RGB greater than alpha",
            ));
        }
        pages.push(Page {
            name,
            asset,
            width: image.width(),
            height: image.height(),
        });
    }
    let model = Model::load(
        std::str::from_utf8(&files[&doc.atlas])
            .map_err(error)?
            .into(),
        files[&doc.skeleton].clone(),
        pages,
    )
    .map_err(error)?;
    if let Some(follow) = &doc.follow {
        follow.validate(&model)?;
    }
    if let Some(drag) = &doc.drag {
        if doc.follow.is_none() {
            return Err(error("drag rig requires a follow base"));
        }
        drag.validate(&model)?;
        let follow = doc.follow.as_ref().expect("checked above");
        let names: std::collections::BTreeSet<_> = drag
            .bones
            .iter()
            .map(|b| &b.bone)
            .chain(drag.feet.iter().map(|b| &b.bone))
            .chain(drag.gaze.bones.iter())
            .chain([&follow.head_bone, &follow.torso_bone])
            .chain(follow.gaze_bones.iter())
            .chain(follow.secondary.iter().map(|b| &b.bone))
            .collect();
        if names.len() > 32 {
            return Err(error("combined drag/follow bone budget exceeded"));
        }
    }
    let pull = if let Some(path) = &doc.pull {
        files::validate_asset_key(path)?;
        if !path.starts_with("spine/") || !path.ends_with(".json") || doc.drag.is_some() {
            return Err(error(
                "pull requires a spine JSON file and cannot combine with story drag",
            ));
        }
        let config: crate::SpinePull =
            serde_json::from_slice(files.get(path).ok_or_else(|| error("missing pull data"))?)
                .map_err(error)?;
        config.validate(
            &model,
            doc.follow
                .as_ref()
                .ok_or_else(|| error("pull requires follow"))?,
        )?;
        Some(Arc::new(config))
    } else {
        None
    };
    if let Some(shake) = &doc.shake {
        shake.validate(
            &model,
            doc.drag
                .as_ref()
                .ok_or_else(|| error("shake requires drag rig"))?,
        )?;
    }
    if let Some(jump) = &doc.jump {
        jump.validate(
            &model,
            doc.follow
                .as_ref()
                .ok_or_else(|| error("jump requires follow base"))?,
        )?;
    }
    if let Some(alias) = &doc.idle_action {
        let clip = doc
            .clips
            .get(alias)
            .ok_or_else(|| error("unknown idle action"))?;
        if alias == &doc.default
            || !matches!(clip.mode, Mode::Once)
            || clip.next.as_deref() != Some(doc.default.as_str())
        {
            return Err(error(
                "idle action must play once and return to the default clip",
            ));
        }
    }
    let mut timed = BTreeMap::new();
    for (alias, clip) in doc.clips {
        let animation = model
            .metadata
            .animations
            .iter()
            .find(|a| a.name == clip.animation)
            .ok_or_else(|| error(format!("unknown Spine animation: {}", clip.animation)))?;
        let duration = match clip.end_seconds {
            Some(end)
                if matches!(clip.mode, Mode::Once)
                    && end.is_finite()
                    && end > 0.0
                    && end <= animation.duration =>
            {
                end
            }
            Some(_) => {
                return Err(error(
                    "Spine clip end_seconds must be within a once animation",
                ))
            }
            None => animation.duration,
        };
        let duration_ns = ((duration * 1e9).round() as u64).max(1);
        let tail_loop = match (
            clip.loop_tail_from_seconds,
            clip.loop_tail_half_cycle_seconds,
        ) {
            (None, None) => None,
            (Some(start), Some(half_cycle))
                if matches!(clip.mode, Mode::Once)
                    && clip.next.is_none()
                    && start.is_finite()
                    && start >= 0.0
                    && start < duration
                    && half_cycle.is_finite()
                    && half_cycle > 0.0
                    && half_cycle <= 3600.0 =>
            {
                Some(TailLoop {
                    start_ns: (start * 1e9).round() as u64,
                    half_cycle_ns: (half_cycle * 1e9).round() as u64,
                })
            }
            _ => {
                return Err(error(
                    "Spine tail loop requires a once clip and valid range",
                ))
            }
        };
        timed.insert(
            alias,
            TimedClip {
                source: clip.animation,
                duration_ns,
                looping: matches!(clip.mode, Mode::Loop),
                next: clip.next,
                tail_loop,
            },
        );
    }
    let animations = Arc::new(AnimationSet::timed(doc.default, timed)?);
    Ok((
        SpineResource {
            model,
            camera,
            follow: doc.follow,
            drag: doc.drag,
            pull,
            shake: doc.shake,
            jump: doc.jump,
            idle_action: doc.idle_action,
        },
        animations,
    ))
}
pub(crate) struct SpineAppearance {
    pub animations: Arc<AnimationSet>,
    pub size: Vec2,
}
impl Appearance for SpineAppearance {
    fn scene(&self, state: &MateState) -> Scene {
        let (animation, time_ns) = self.animations.pose(state);
        Scene {
            nodes: vec![SceneNode {
                id: "body".into(),
                transform: Transform {
                    translation: state.position,
                    ..Transform::default()
                },
                opacity: 1.0,
                content: SceneContent::Skeletal {
                    asset: "spine.toml".into(),
                    size: self.size,
                    animation: animation.into(),
                    time_ns,
                    offsets: Vec::new(),
                    overlay: Default::default(),
                },
            }],
        }
    }
}

#[cfg(not(feature = "spine"))]
pub(crate) fn load(
    _files: &BTreeMap<String, Vec<u8>>,
    _images: &BTreeMap<String, DecodedImage>,
) -> Result<(SpineResource, Arc<AnimationSet>), PackageError> {
    Err(error(
        "Spine support is not included in this build; install a Sprite character.",
    ))
}
