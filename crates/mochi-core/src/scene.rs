use crate::{MateState, Namespace, SchemaError, Value, Vec2};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    pub translation: Vec2,
    pub scale: Vec2,
    pub rotation_radians: f64,
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec2::default(),
            scale: Vec2::new(1.0, 1.0),
            rotation_radians: 0.0,
        }
    }
}

/// Flat display list in painter order; no backend drawing objects or file handles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub nodes: Vec<SceneNode>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SceneNode {
    pub id: String,
    pub transform: Transform,
    pub opacity: f64,
    pub content: SceneContent,
}
/// Bounded additive pose channels in model space, applied after animation.
#[derive(Debug, Clone, PartialEq)]
pub struct BoneOffset {
    pub bone: String,
    pub rotation_degrees: f64,
    pub local_translation: Vec2,
    pub world_translation: Vec2,
}
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SceneContent {
    Sprite {
        asset: String,
        size: Vec2,
        animation: String,
    },
    /// Absolute skeletal pose. The package identifies its format and fixed camera;
    /// no evaluator, texture handle or mutable playback state enters Core.
    Skeletal {
        asset: String,
        size: Vec2,
        animation: String,
        time_ns: u64,
        offsets: Vec<BoneOffset>,
        overlay: crate::PoseOverlay,
    },
    Text {
        text: String,
        font_size: f64,
    },
    /// Future data-only primitives. Backends must explicitly reject or support.
    Extension {
        kind: Namespace,
        data: Value,
    },
}
impl Scene {
    pub fn validate(&self) -> Result<(), SchemaError> {
        let mut ids = std::collections::BTreeSet::new();
        for node in &self.nodes {
            if node.id.is_empty() || !ids.insert(&node.id) {
                return Err(SchemaError("empty or duplicate scene node id".into()));
            }
            node.transform.translation.validate()?;
            node.transform.scale.validate()?;
            if !node.transform.rotation_radians.is_finite()
                || !node.opacity.is_finite()
                || !(0.0..=1.0).contains(&node.opacity)
            {
                return Err(SchemaError("invalid scene transform/opacity".into()));
            }
            match &node.content {
                SceneContent::Sprite {
                    asset,
                    size,
                    animation,
                }
                | SceneContent::Skeletal {
                    asset,
                    size,
                    animation,
                    ..
                } => {
                    size.validate()?;
                    if asset.is_empty() || size.x <= 0.0 || size.y <= 0.0 {
                        return Err(SchemaError("invalid sprite asset/size".into()));
                    }
                    crate::state::validate_label(animation)?;
                    if matches!(node.content, SceneContent::Skeletal { time_ns, .. } if time_ns > 3_600_000_000_000)
                    {
                        return Err(SchemaError("skeletal pose time exceeds one hour".into()));
                    }
                    if let SceneContent::Skeletal {
                        offsets, overlay, ..
                    } = &node.content
                    {
                        overlay.validate()?;
                        if offsets.len() > 32 {
                            return Err(SchemaError("too many skeletal bone offsets".into()));
                        }
                        let mut names = std::collections::BTreeSet::new();
                        for offset in offsets {
                            if offset.bone.is_empty()
                                || offset.bone.len() > 256
                                || offset.bone.contains('\0')
                                || !names.insert(&offset.bone)
                                || !offset.rotation_degrees.is_finite()
                                || offset.rotation_degrees.abs() > 30.0
                                || !offset.local_translation.x.is_finite()
                                || !offset.local_translation.y.is_finite()
                                || offset.local_translation.x.abs() > 32.0
                                || offset.local_translation.y.abs() > 32.0
                                || !offset.world_translation.x.is_finite()
                                || !offset.world_translation.y.is_finite()
                                || offset.world_translation.x.abs() > 5.0
                                || offset.world_translation.y.abs() > 5.0
                            {
                                return Err(SchemaError("invalid skeletal bone offset".into()));
                            }
                        }
                    }
                }
                SceneContent::Text { font_size, .. }
                    if !font_size.is_finite() || *font_size <= 0.0 =>
                {
                    return Err(SchemaError("invalid font size".into()))
                }
                SceneContent::Extension { data, .. } => data.validate()?,
                _ => {}
            }
        }
        Ok(())
    }
}

/// Pure State → Scene projection; cannot draw or resolve files through this API.
pub trait Appearance {
    fn scene(&self, state: &MateState) -> Scene;
}

/// Maps animation names to package-relative asset keys, falling back to one asset.
pub struct SpriteAppearance {
    pub asset: String,
    pub size: Vec2,
    pub animations: BTreeMap<String, String>,
}
impl Appearance for SpriteAppearance {
    fn scene(&self, state: &MateState) -> Scene {
        Scene {
            nodes: vec![SceneNode {
                id: "body".into(),
                transform: Transform {
                    translation: state.position,
                    ..Transform::default()
                },
                opacity: 1.0,
                content: SceneContent::Sprite {
                    asset: self
                        .animations
                        .get(&state.current_animation)
                        .unwrap_or(&self.asset)
                        .clone(),
                    size: self.size,
                    animation: state.current_animation.clone(),
                },
            }],
        }
    }
}
