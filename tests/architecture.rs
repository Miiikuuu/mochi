mod cases {
    pub mod animation;
    pub mod boundaries;
    pub mod broker;
    pub mod choices;
    pub mod composition;
    pub mod core;
    pub mod host;
    pub mod package;
    pub mod preview;
    pub mod renderer;
    pub mod resources;
    pub mod rules;
    pub mod runtime;
    pub mod schema;
}
use mochi_core::*;
use mochi_runtime::Runtime;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn ns(s: &str) -> Namespace {
    Namespace::new(s).unwrap()
}
fn appearance() -> Box<dyn Appearance> {
    Box::new(SpriteAppearance {
        asset: "sprites/neko.png".into(),
        size: Vec2::new(32.0, 32.0),
        animations: BTreeMap::new(),
    })
}
fn runtime(behaviors: Vec<Box<dyn Behavior>>) -> Runtime {
    Runtime::new(
        "test-1",
        MateState::default(),
        Default::default(),
        behaviors,
        appearance(),
    )
    .unwrap()
}
fn neko_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/neko/neko.mochi")
}
fn manifest_text() -> String {
    std::fs::read_to_string(neko_path().join("manifest.toml")).unwrap()
}
