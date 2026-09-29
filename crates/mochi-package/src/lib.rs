//! Bounded data-only directory package loader. No dynamic code loading.
mod animation;
mod containers;
mod files;
mod resources;
mod rules;
mod spine;
use animation::{AnimatedAppearance, AnimatedBehavior, AnimationSet};
use mochi_core::*;
use mochi_runtime::Runtime;
use mochi_schema::{Component, MateManifest, SCHEMA_VERSION};
pub use resources::{ContentHash, DecodeLimits, DecodedImage};
use rules::RulesBehavior;
use serde::de::DeserializeOwned;
use serde::Deserialize;
pub use spine::{SpineCamera, SpineResource};
mod spine_jump;
pub use spine_jump::*;
mod spine_shake;
pub use spine_shake::*;
mod spine_pull;
pub use spine_pull::*;
mod spine_drag;
pub use spine_drag::*;
mod spine_follow;
pub use spine_follow::{FollowSecondary, SpineFollow};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub struct PackageError(pub String);
impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for PackageError {}
fn error(e: impl fmt::Display) -> PackageError {
    PackageError(e.to_string())
}

/// Host-selected import policy; a package cannot raise its own budgets.
#[derive(Debug, Clone, Copy)]
pub struct LoadOptions {
    /// Number of ordinary files, including TOML (1..=4096).
    /// Directory enumeration is also bounded to twice this count.
    pub max_files: usize,
    pub decode: DecodeLimits,
}
impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            max_files: 128,
            decode: DecodeLimits::default(),
        }
    }
}

pub struct LoadedPackage {
    manifest: MateManifest,
    assets: BTreeMap<String, Vec<u8>>,
    images: BTreeMap<String, DecodedImage>,
    content_hash: ContentHash,
    spine: Option<SpineResource>,
    // Private implementation configuration, not a classification of Mate kinds.
    behavior: BehaviorConfig,
    appearance: AppearanceConfig,
}
// Built-in behavior recipes, not Mate types.
enum BehaviorConfig {
    Idle,
    Wander(WanderConfig),
    Rules(RulesBehavior),
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct WanderConfig {
    schema_version: u32,
    min_x: f64,
    max_x: f64,
    speed: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BehaviorSource {
    file: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpriteConfig {
    asset: String,
    width: f64,
    height: f64,
    #[serde(default)]
    animations: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnimatedSpriteConfig {
    file: String,
    width: f64,
    height: f64,
}
// Loader recipes, not Mate categories. The runtime only sees existing traits.
enum AppearanceConfig {
    Spine {
        animations: Arc<AnimationSet>,
        size: Vec2,
    },
    Static(SpriteConfig),
    Animated {
        animations: Arc<AnimationSet>,
        size: Vec2,
    },
}
fn validate_sprite(files: &BTreeMap<String, Vec<u8>>, asset: &str) -> Result<(), PackageError> {
    files::validate_asset_key(asset)?;
    if !asset.starts_with("sprites/") || !files.contains_key(asset) {
        return Err(error(format!("missing sprite asset: {asset}")));
    }
    Ok(())
}
fn config<T: DeserializeOwned>(component: &Component) -> Result<T, PackageError> {
    toml::Value::try_from(&component.config)
        .map_err(error)?
        .try_into()
        .map_err(error)
}

impl LoadedPackage {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PackageError> {
        Self::load_with_limits(path, DecodeLimits::default())
    }
    pub fn load_with_limits(
        path: impl AsRef<Path>,
        limits: DecodeLimits,
    ) -> Result<Self, PackageError> {
        Self::load_with_options(
            path,
            LoadOptions {
                decode: limits,
                ..LoadOptions::default()
            },
        )
    }
    pub fn load_with_options(
        path: impl AsRef<Path>,
        options: LoadOptions,
    ) -> Result<Self, PackageError> {
        if !(1..=4096).contains(&options.max_files) {
            return Err(error("max_files must be in 1..=4096"));
        }
        let mut files = files::snapshot(path.as_ref(), options.max_files)?;
        let content_hash = resources::package_hash(&files);
        let manifest_bytes = files
            .remove("manifest.toml")
            .ok_or_else(|| error("missing manifest.toml"))?;
        let manifest =
            MateManifest::from_toml(std::str::from_utf8(&manifest_bytes).map_err(error)?)
                .map_err(error)?;
        let images = resources::decode_images(&files, options.decode)?;
        let mut spine_resource = None;
        let mut behavior = None;
        let mut appearance = None;
        for component in &manifest.components {
            match component.id.as_str() {
                "mochi.behavior.idle" | "mochi.behavior.wander" | "mochi.behavior.rules" => {
                    if behavior.is_some() {
                        return Err(error("packages require exactly one behavior component"));
                    }
                    if component.id.as_str() == "mochi.behavior.idle" {
                        if !component.config.is_empty() {
                            return Err(error("idle component accepts no config"));
                        }
                        behavior = Some(BehaviorConfig::Idle);
                    } else {
                        let source: BehaviorSource = config(component)?;
                        let is_rules = component.id.as_str() == "mochi.behavior.rules";
                        let expected = if is_rules {
                            "rules.toml"
                        } else {
                            "behavior.toml"
                        };
                        if source.file != expected {
                            return Err(error(format!("behavior file must be {expected}")));
                        }
                        let bytes = files
                            .get(expected)
                            .ok_or_else(|| error(format!("missing {expected}")))?;
                        let text = std::str::from_utf8(bytes).map_err(error)?;
                        behavior = Some(if is_rules {
                            BehaviorConfig::Rules(RulesBehavior::parse(text)?)
                        } else {
                            let cfg: WanderConfig = toml::from_str(text).map_err(error)?;
                            if cfg.schema_version != SCHEMA_VERSION {
                                return Err(error("unsupported behavior schema_version"));
                            }
                            WanderBehavior::new(cfg.min_x, cfg.max_x, cfg.speed).map_err(error)?;
                            BehaviorConfig::Wander(cfg)
                        });
                    }
                }
                "mochi.appearance.sprite" => {
                    if appearance.is_some() {
                        return Err(error("packages require exactly one appearance component"));
                    }
                    let cfg: SpriteConfig = config(component)?;
                    for asset in std::iter::once(&cfg.asset).chain(cfg.animations.values()) {
                        validate_sprite(&files, asset)?;
                    }
                    for name in cfg.animations.keys() {
                        if name.trim().is_empty() || name.len() > 256 {
                            return Err(error("invalid animation name"));
                        }
                    }
                    let projection = SpriteAppearance {
                        asset: cfg.asset.clone(),
                        size: Vec2::new(cfg.width, cfg.height),
                        animations: cfg.animations.clone(),
                    };
                    projection
                        .scene(&MateState::default())
                        .validate()
                        .map_err(error)?;
                    appearance = Some(AppearanceConfig::Static(cfg));
                }
                "mochi.appearance.spine" => {
                    if appearance.is_some() {
                        return Err(error("packages require exactly one appearance component"));
                    }
                    let cfg: AnimatedSpriteConfig = config(component)?;
                    if cfg.file != "spine.toml" {
                        return Err(error("Spine file must be spine.toml"));
                    }
                    let (resource, animations) = spine::load(&files, &images)?;
                    let size = Vec2::new(cfg.width, cfg.height);
                    spine::SpineAppearance {
                        animations: animations.clone(),
                        size,
                    }
                    .scene(&animations.initial_state())
                    .validate()
                    .map_err(error)?;
                    spine_resource = Some(resource);
                    appearance = Some(AppearanceConfig::Spine { animations, size });
                }
                "mochi.appearance.animated-sprite" => {
                    if appearance.is_some() {
                        return Err(error("packages require exactly one appearance component"));
                    }
                    let cfg: AnimatedSpriteConfig = config(component)?;
                    if cfg.file != "animations.toml" {
                        return Err(error("animation file must be animations.toml"));
                    }
                    let bytes = files
                        .get(&cfg.file)
                        .ok_or_else(|| error("missing animations.toml"))?;
                    let animations = Arc::new(AnimationSet::parse(
                        std::str::from_utf8(bytes).map_err(error)?,
                    )?);
                    for asset in animations.assets() {
                        validate_sprite(&files, asset)?;
                    }
                    let size = Vec2::new(cfg.width, cfg.height);
                    AnimatedAppearance {
                        animations: animations.clone(),
                        size,
                    }
                    .scene(&animations.initial_state())
                    .validate()
                    .map_err(error)?;
                    appearance = Some(AppearanceConfig::Animated { animations, size });
                }
                id => return Err(error(format!("unsupported component: {id}"))),
            }
        }
        let behavior = behavior.ok_or_else(|| error("missing behavior component"))?;
        let appearance = appearance.ok_or_else(|| error("missing appearance component"))?;
        if !matches!(appearance, AppearanceConfig::Animated { .. })
            && files.contains_key("animations.toml")
        {
            return Err(error("unused animations.toml in static sprite package"));
        }
        if spine_resource.is_none()
            && (files.contains_key("spine.toml")
                || files.keys().any(|key| key.starts_with("spine/")))
        {
            return Err(error("unused Spine data in sprite package"));
        }
        files.remove("animations.toml");
        for (name, used) in [
            (
                "behavior.toml",
                matches!(behavior, BehaviorConfig::Wander(_)),
            ),
            ("rules.toml", matches!(behavior, BehaviorConfig::Rules(_))),
        ] {
            if !used && files.contains_key(name) {
                return Err(error(format!("unused {name} in package")));
            }
            files.remove(name);
        }
        if let (
            BehaviorConfig::Rules(rules),
            AppearanceConfig::Animated { animations, .. }
            | AppearanceConfig::Spine { animations, .. },
        ) = (&behavior, &appearance)
        {
            rules.reserve_animation_actions(animations.reserved_actions())?;
        }
        Ok(Self {
            manifest,
            assets: files,
            images,
            content_hash,
            spine: spine_resource,
            behavior,
            appearance,
        })
    }
    pub fn spine(&self, key: &str) -> Result<&SpineResource, PackageError> {
        if key != "spine.toml" {
            return Err(error("unknown skeletal resource"));
        }
        self.spine
            .as_ref()
            .ok_or_else(|| error("package contains no Spine resource"))
    }
    pub fn manifest(&self) -> &MateManifest {
        &self.manifest
    }
    /// Immutable bytes captured at load time; rendering need not reopen paths.
    pub fn assets(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.assets
    }
    /// Identity of the captured file contents, including configuration.
    /// Does not include the directory name, metadata, decode policy or empty directories.
    pub fn content_hash(&self) -> ContentHash {
        self.content_hash
    }
    /// Exact, package-local Scene resource key lookup. Never opens a path.
    /// Sounds are opaque bytes and cannot be resolved as decoded images.
    pub fn image(&self, key: &str) -> Result<&DecodedImage, PackageError> {
        self.images
            .get(key)
            .ok_or_else(|| error(format!("no decoded image for key: {key}")))
    }
    pub fn instantiate(&self, instance_id: impl Into<String>) -> Result<Runtime, PackageError> {
        self.instantiate_with_behavior(instance_id, |behavior| behavior)
    }
    pub fn has_animation(&self, alias: &str) -> bool {
        match &self.appearance {
            AppearanceConfig::Animated { animations, .. }
            | AppearanceConfig::Spine { animations, .. } => animations.contains(alias),
            AppearanceConfig::Static(sprite) => {
                alias == "idle" || sprite.animations.contains_key(alias)
            }
        }
    }
    /// A trusted host can decorate package behavior before the animation clock.
    /// The clock and appearance remain package-owned; assets are never rewritten.
    pub fn instantiate_with_behavior(
        &self,
        instance_id: impl Into<String>,
        decorate: impl FnOnce(Box<dyn Behavior>) -> Box<dyn Behavior>,
    ) -> Result<Runtime, PackageError> {
        let behavior: Box<dyn Behavior> = match &self.behavior {
            BehaviorConfig::Wander(config) => Box::new(
                WanderBehavior::new(config.min_x, config.max_x, config.speed).map_err(error)?,
            ),
            BehaviorConfig::Idle => Box::new(IdleBehavior),
            BehaviorConfig::Rules(rules) => Box::new(rules.clone()),
        };
        let behavior = decorate(behavior);
        let (state, behavior, appearance): (MateState, Box<dyn Behavior>, Box<dyn Appearance>) =
            match &self.appearance {
                AppearanceConfig::Static(sprite) => (
                    MateState::default(),
                    behavior,
                    Box::new(SpriteAppearance {
                        asset: sprite.asset.clone(),
                        size: Vec2::new(sprite.width, sprite.height),
                        animations: sprite.animations.clone(),
                    }),
                ),
                AppearanceConfig::Spine { animations, size } => (
                    animations.initial_state(),
                    Box::new(AnimatedBehavior {
                        inner: behavior,
                        animations: animations.clone(),
                    }),
                    Box::new(spine::SpineAppearance {
                        animations: animations.clone(),
                        size: *size,
                    }),
                ),
                AppearanceConfig::Animated { animations, size } => (
                    animations.initial_state(),
                    Box::new(AnimatedBehavior {
                        inner: behavior,
                        animations: animations.clone(),
                    }),
                    Box::new(AnimatedAppearance {
                        animations: animations.clone(),
                        size: *size,
                    }),
                ),
            };
        Runtime::new(
            instance_id,
            state,
            self.manifest.capabilities.clone(),
            vec![behavior],
            appearance,
        )
        .map_err(error)
    }
}
