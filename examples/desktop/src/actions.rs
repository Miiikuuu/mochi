//! Theme-local action routing, outside the package and animation clock.
use mochi_core::{Behavior, MateAction, MateContext, MateEvent, Namespace, Value};
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub struct Policy(pub BTreeMap<String, Setting>);
#[derive(Clone)]
pub struct Setting {
    pub enabled: bool,
    pub trigger: String,
    pub clip: bool,
}
impl Policy {
    pub fn load() -> Result<Vec<Self>, Box<dyn std::error::Error>> {
        let Ok(raw) = std::env::var("MOCHI_ACTION_POLICY") else {
            return Ok(Vec::new());
        };
        if raw.len() > 262144 {
            return Err("action policy too large".into());
        }
        let values: serde_json::Value = serde_json::from_str(&raw)?;
        let values = values.as_array().ok_or("invalid action policy")?;
        if values.len() > 4 {
            return Err("too many action policies".into());
        }
        values
            .iter()
            .map(|value| {
                let mut result = BTreeMap::new();
                for (key, item) in value.as_object().ok_or("invalid character policy")? {
                    result.insert(
                        key.clone(),
                        Setting {
                            enabled: item["enabled"].as_bool().ok_or("invalid action enabled")?,
                            trigger: item["trigger"]
                                .as_str()
                                .ok_or("invalid action trigger")?
                                .into(),
                            clip: item["kind"].as_str().ok_or("invalid action kind")? == "clip",
                        },
                    );
                }
                Ok(Self(result))
            })
            .collect()
    }
    pub fn enabled(&self, key: &str) -> bool {
        self.0
            .get(key)
            .is_none_or(|s| s.enabled && s.trigger != "manual")
    }
    pub fn allows(&self, key: &str, trigger: &str) -> bool {
        self.enabled(key)
            && self
                .0
                .get(key)
                .is_none_or(|s| s.trigger == "package" || s.trigger == trigger)
    }
    pub fn bound(&self, trigger: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(_, s)| s.enabled && s.trigger == trigger)
            .map(|(key, _)| key.clone())
    }
    pub fn key_action(&self, key: u32) -> Option<String> {
        if key == 0x20 && self.allows("jump", "space") {
            return Some("jump".into());
        }
        if (0x36..=0x39).contains(&key) {
            return self.bound(&format!("key_{}", key - 0x30));
        }
        if (0x31..=0x35).contains(&key) {
            let alias = ["idle", "wave", "walk", "sit", "sleep"][(key - 0x31) as usize];
            if alias == "idle" || self.allows(alias, "package") {
                return Some(alias.into());
            }
        }
        None
    }
    pub fn wrap(self, inner: Box<dyn Behavior>) -> Box<dyn Behavior> {
        Box::new(Managed {
            inner,
            policy: self,
        })
    }
}

pub fn play(alias: &str) -> Vec<MateAction> {
    vec![
        MateAction::PlayAnimation(alias.into()),
        MateAction::EmitEvent(MateEvent::Custom {
            name: Namespace::new("mochi.animation.restart").expect("constant namespace"),
            payload: Value::Object(BTreeMap::new()),
        }),
    ]
}

struct Managed {
    inner: Box<dyn Behavior>,
    policy: Policy,
}
impl Behavior for Managed {
    fn on_event(&self, context: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        if let MateEvent::Custom { name, .. } = event {
            if let Some(alias) = name.as_str().strip_prefix("mochi.desktop.") {
                if let Some(setting) = self.policy.0.get(alias) {
                    return if setting.clip && self.policy.enabled(alias) {
                        play(alias)
                    } else {
                        Vec::new()
                    };
                }
            }
        }
        let trigger = match event {
            MateEvent::PointerClick { button: 1, .. } => Some("click"),
            MateEvent::PointerEnter => Some("hover"),
            _ => None,
        };
        if let Some(alias) = trigger.and_then(|t| self.policy.bound(t)) {
            return play(&alias);
        }
        let actions = self.inner.on_event(context, event);
        let blocked = actions.iter().any(|action| matches!(action,
            MateAction::PlayAnimation(alias) if !self.policy.enabled(alias)
                || self.policy.0.get(alias).is_some_and(|s| !["package", "always"].contains(&s.trigger.as_str()))
                || (alias == "walk" && !["idle", "walk"].contains(&context.state.current_animation.as_str())
                    && matches!(event, MateEvent::Tick { .. }))));
        if blocked {
            // Keep unrelated state/capabilities, but prevent disabled wander from
            // moving a stationary pose and remove its animation-restart event.
            actions
                .into_iter()
                .filter(|a| match a {
                    MateAction::PlayAnimation(_)
                    | MateAction::MoveTo(_)
                    | MateAction::MoveBy(_)
                    | MateAction::SetVelocity(_) => false,
                    MateAction::EmitEvent(MateEvent::Custom { name, .. }) => {
                        name.as_str() != "mochi.animation.restart"
                    }
                    _ => true,
                })
                .collect()
        } else {
            actions
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_package::LoadOptions;
    use mochi_package::LoadedPackage;
    use std::{path::PathBuf, time::Duration};
    #[cfg(feature = "spine")]
    fn package() -> LoadedPackage {
        LoadedPackage::load_with_options(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../characters/shanbi-sit/shanbi.mochi"),
            LoadOptions {
                max_files: 256,
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn policy(entries: &[(&str, bool, &str)]) -> Policy {
        Policy(
            entries
                .iter()
                .map(|(id, enabled, trigger)| {
                    (
                        id.to_string(),
                        Setting {
                            enabled: *enabled,
                            trigger: trigger.to_string(),
                            clip: *id != "jump",
                        },
                    )
                })
                .collect(),
        )
    }
    #[cfg(feature = "spine")]
    fn click() -> MateEvent {
        MateEvent::PointerClick {
            position: Default::default(),
            button: 1,
        }
    }
    #[test]
    #[cfg(feature = "spine")]
    fn disabled_click_and_custom_dispatch_cannot_start_action() {
        let p = policy(&[("wave", false, "package")]);
        let mut rt = package()
            .instantiate_with_behavior("test", |b| p.wrap(b))
            .unwrap();
        rt.process(click()).unwrap();
        assert_eq!(rt.state().current_animation, "idle");
        rt.process(MateEvent::Custom {
            name: Namespace::new("mochi.desktop.wave").unwrap(),
            payload: Value::Object(BTreeMap::new()),
        })
        .unwrap();
        assert_eq!(rt.state().current_animation, "idle");
        rt.process(MateEvent::Tick {
            delta: Duration::from_secs_f64(0.5),
        })
        .unwrap();
        assert_ne!(
            rt.scene().unwrap(),
            package().instantiate("fresh").unwrap().scene().unwrap()
        );
    }
    #[test]
    #[cfg(feature = "spine")]
    fn reassigned_trigger_preserves_clock_restart_and_recovery() {
        let p = policy(&[("wave", true, "hover")]);
        let mut rt = package()
            .instantiate_with_behavior("test", |b| p.wrap(b))
            .unwrap();
        rt.process(click()).unwrap();
        assert_eq!(rt.state().current_animation, "idle");
        let enter = || MateEvent::PointerEnter;
        rt.process(enter()).unwrap();
        let first = rt.scene().unwrap();
        rt.process(MateEvent::Tick {
            delta: Duration::from_millis(250),
        })
        .unwrap();
        assert_ne!(first, rt.scene().unwrap());
        rt.process(enter()).unwrap();
        assert_eq!(first, rt.scene().unwrap());
        rt.process(MateEvent::Tick {
            delta: Duration::from_secs(20),
        })
        .unwrap();
        assert_eq!(rt.state().current_animation, "idle");
    }
    #[test]
    fn triggers_gate_keyboard_idle_and_jump_independently() {
        let p = policy(&[
            ("walk", true, "idle"),
            ("sit", false, "package"),
            ("jump", true, "key_6"),
        ]);
        assert!(p.allows("walk", "idle"));
        assert!(!p.allows("walk", "keyboard"));
        assert!(p.key_action(0x33).is_none());
        assert!(p.key_action(0x34).is_none());
        assert!(p.key_action(0x20).is_none());
        assert_eq!(p.key_action(0x36).as_deref(), Some("jump"));
        assert_eq!(p.key_action(0x31).as_deref(), Some("idle"));
    }
    #[test]
    fn png_wander_does_not_override_a_selected_action() {
        let package = LoadedPackage::load_with_options(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../animated-neko/neko.mochi"),
            LoadOptions {
                max_files: 256,
                ..Default::default()
            },
        )
        .unwrap();
        let p = policy(&[
            ("idle", true, "always"),
            ("wave", true, "key_6"),
            ("walk", true, "package"),
        ]);
        let mut rt = package
            .instantiate_with_behavior("png", |b| p.wrap(b))
            .unwrap();
        rt.process(MateEvent::Custom {
            name: Namespace::new("mochi.desktop.wave").unwrap(),
            payload: Value::Object(BTreeMap::new()),
        })
        .unwrap();
        let first = rt.scene().unwrap();
        rt.process(MateEvent::Tick {
            delta: Duration::from_millis(250),
        })
        .unwrap();
        assert_eq!(rt.state().current_animation, "wave");
        assert_eq!(rt.state().position, Default::default());
        assert_ne!(first, rt.scene().unwrap());
        rt.process(MateEvent::Tick {
            delta: Duration::from_secs(2),
        })
        .unwrap();
        // This PNG clip intentionally holds its last frame (no authored next).
        assert_eq!(rt.state().current_animation, "wave");
        rt.process(MateEvent::Custom {
            name: Namespace::new("mochi.desktop.idle").unwrap(),
            payload: Value::Object(BTreeMap::new()),
        })
        .unwrap();
        assert_eq!(rt.state().current_animation, "idle");
        rt.process(MateEvent::Tick {
            delta: Duration::from_millis(250),
        })
        .unwrap();
        assert_eq!(rt.state().current_animation, "walk");
    }
    #[test]
    #[cfg(feature = "spine")]
    fn shanbi_work_keeps_moving_until_the_task_ends() {
        let package = package();
        let p = policy(&[("work", true, "task")]);
        let mut rt = package
            .instantiate_with_behavior("work", |b| p.wrap(b))
            .unwrap();
        rt.process(MateEvent::Custom {
            name: Namespace::new("mochi.desktop.work").unwrap(),
            payload: Value::Object(BTreeMap::new()),
        })
        .unwrap();
        assert_eq!(rt.state().current_animation, "work");
        rt.process(MateEvent::Tick {
            delta: Duration::from_millis(1900),
        })
        .unwrap();
        let end_pose = rt.scene().unwrap();
        rt.process(MateEvent::Tick {
            delta: Duration::from_millis(700),
        })
        .unwrap();
        let moving = rt.scene().unwrap();
        assert_ne!(moving, end_pose);
        rt.process(MateEvent::Tick {
            delta: Duration::from_millis(700),
        })
        .unwrap();
        assert_ne!(rt.scene().unwrap(), moving);
        rt.process(MateEvent::Tick {
            delta: Duration::from_millis(1400),
        })
        .unwrap();
        assert_eq!(rt.scene().unwrap(), end_pose);
        rt.process(MateEvent::Custom {
            name: Namespace::new("mochi.desktop.idle").unwrap(),
            payload: Value::Object(BTreeMap::new()),
        })
        .unwrap();
        assert_eq!(rt.state().current_animation, "idle");
        assert_ne!(rt.scene().unwrap(), end_pose);
    }
}
