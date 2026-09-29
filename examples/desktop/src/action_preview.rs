//! One isolated, bounded preview stream. Uses the same renderer and gesture logic
//! as desktop pets, without X11 windows, global input, plugins, or Theme ownership.
use crate::{actions, drag::Drag, gaze::Gaze, jump::Jump, shake::Shake};
use mochi_core::{Behavior, MateAction, MateContext, MateEvent, SceneContent, Vec2};
use mochi_package::{LoadOptions, LoadedPackage};
use mochi_preview::Viewport;
use mochi_renderer_software::{
    CanvasRenderer, RenderError, RenderLayer, RenderOptions, SamplingMode,
};
use std::{
    error::Error,
    io::{self, Write},
    time::{Duration, Instant},
};

struct PreviewBehavior(String);
impl Behavior for PreviewBehavior {
    fn on_event(&self, _: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        if matches!(event, MateEvent::PointerClick { .. }) {
            actions::play(&self.0)
        } else {
            vec![]
        }
    }
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(2).collect();
    if args.len() != 3 {
        return Err("preview expects package, action, kind".into());
    }
    let package = LoadedPackage::load_with_options(
        &args[0],
        LoadOptions {
            max_files: 256,
            ..Default::default()
        },
    )?;
    let action = &args[1];
    let interaction = args[2] == "interaction";
    if !["clip", "interaction"].contains(&args[2].as_str()) {
        return Err("invalid preview kind".into());
    }
    let alias = if interaction { "idle" } else { action };
    if !package.has_animation(alias) {
        return Err("preview animation missing from package".into());
    }
    let mut runtime = package.instantiate_with_behavior("manager-preview", |_| {
        Box::new(PreviewBehavior(alias.into()))
    })?;
    let play = || MateEvent::PointerClick {
        position: Vec2::default(),
        button: 1,
    };
    runtime.process(play())?;
    let scene = runtime.scene()?;
    let logical = match &scene.nodes.first().ok_or("empty preview")?.content {
        SceneContent::Sprite { size, .. } | SceneContent::Skeletal { size, .. } => {
            [size.x.ceil() as u32, size.y.ceil() as u32]
        }
        _ => return Err("unsupported preview appearance".into()),
    };
    let size = 256u32;
    let viewport = Viewport::new(logical, [size; 2])?.ok_or("empty preview")?;
    let spine = package.spine("spine.toml").ok();
    let mut gaze = spine.and_then(|s| s.follow.clone().map(|f| Gaze::new(f, s.camera)));
    let mut drag = spine.and_then(|s| {
        s.drag
            .clone()
            .zip(s.follow.clone())
            .map(|(d, f)| Drag::new(d, f, s.camera))
            .or_else(|| {
                s.pull
                    .clone()
                    .zip(s.follow.clone())
                    .map(|(p, f)| Drag::new_pull(p, f, s.camera))
            })
    });
    let mut shake = spine.and_then(|s| {
        Some(Shake::new(
            s.shake.clone()?,
            s.drag.clone()?,
            s.follow.clone()?,
        ))
    });
    let mut jump =
        spine.and_then(|s| Some(Jump::new(s.jump.clone()?, s.follow.clone()?, s.camera)));
    if interaction
        && !match action.as_str() {
            "gaze" => gaze.is_some(),
            "drag" => drag.is_some(),
            "shake" => shake.is_some(),
            "jump" => jump.is_some(),
            _ => false,
        }
    {
        return Err("interaction missing from package".into());
    }
    let mut renderer = CanvasRenderer::new(RenderOptions {
        width: size,
        height: size,
        background: [0; 4],
        sampling: SamplingMode::Coverage,
        ..Default::default()
    })?;
    if spine.is_some() && std::env::var_os("MOCHI_SPINE_SOFTWARE").is_none() {
        if let Err(error) = renderer.enable_spine_gpu() {
            eprintln!("Preview uses CPU: {error}");
        }
    }
    let mut output = io::stdout().lock();
    output.write_all(b"MPR1")?;
    output.write_all(&size.to_le_bytes())?;
    output.write_all(&size.to_le_bytes())?;
    let mut frame = 0u32;
    let mut released = false;
    let epoch = Instant::now();
    let mut next = epoch;
    loop {
        let step = frame % 300; // Gesture + recovery; replay every ten seconds.
        let dt = Duration::from_secs_f64(1.0 / 30.0);
        runtime.process(MateEvent::Tick { delta: dt })?;
        if frame > 0 && ((!interaction && runtime.state().current_animation != alias) || step == 0)
        {
            runtime.process(play())?;
        }
        let mut scene = runtime.scene()?;
        if let Some(gaze) = &mut gaze {
            gaze.advance(dt.as_secs_f64());
            if interaction && action == "gaze" {
                let t = f64::from(step) / 30.0;
                gaze.input(
                    &scene,
                    (step < 210).then(|| {
                        Vec2::new(
                            f64::from(logical[0]) * (0.5 + (t * 1.7).sin() * 2.0),
                            f64::from(logical[1]) * (0.4 + t.cos()),
                        )
                    }),
                );
            }
            gaze.project(&mut scene, interaction);
        }
        if interaction && step == 0 {
            released = false;
            if action == "drag" || action == "shake" {
                if let Some(drag) = &mut drag {
                    drag.clear();
                    drag.begin(
                        &scene,
                        Vec2::new(f64::from(logical[0]) * 0.5, f64::from(logical[1]) * 0.3),
                        None,
                    );
                }
                if action == "shake" {
                    shake
                        .as_mut()
                        .expect("validated shake")
                        .begin((epoch.elapsed().as_millis() as u32).max(1));
                }
            }
            if action == "jump" {
                jump.as_mut()
                    .expect("validated jump")
                    .start(gaze.as_ref().map_or(0.0, Gaze::pose_time));
            }
        }
        if interaction && (action == "drag" || action == "shake") {
            if step < 90 {
                let t = f64::from(step) / 30.0;
                let delta = if action == "shake" {
                    [((t * 18.0).sin() * 110.0) as i32, -35]
                } else {
                    [((t * 2.0).sin() * 45.0) as i32, -40]
                };
                if let Some(drag) = &mut drag {
                    drag.move_to(delta, size as u16, false);
                }
                if action == "shake" {
                    shake.as_mut().expect("validated shake").move_to(
                        delta,
                        1.0,
                        (epoch.elapsed().as_millis() as u32).max(1),
                    );
                }
            } else if !released {
                released = true;
                if let Some(drag) = &mut drag {
                    drag.release();
                }
                if action == "shake" {
                    shake.as_mut().expect("validated shake").release();
                }
            }
            if let Some(drag) = &mut drag {
                drag.advance(dt.as_secs_f64());
                drag.project(&mut scene);
            }
            if action == "shake" {
                shake.as_ref().expect("validated shake").project(&mut scene);
            }
        }
        if interaction && action == "jump" {
            jump.as_ref().expect("validated jump").project(&mut scene);
        }
        let scene = viewport.project_scene(&scene);
        let layers = [RenderLayer {
            package: &package,
            scene: &scene,
        }];
        if let Err(error) = renderer.render_layers(&layers) {
            match error {
                RenderError::GpuUnavailable(_) => renderer.render_layers(&layers)?,
                _ => return Err(error.into()),
            }
        }
        if let Err(error) = output
            .write_all(renderer.frame().rgba8())
            .and_then(|_| output.flush())
        {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(error.into());
        }
        frame = frame.wrapping_add(1);
        next += dt;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        // No catch-up burst after a blocked pipe or an unusually slow render.
        if Instant::now().saturating_duration_since(next) > dt {
            next = Instant::now();
        }
    }
}
