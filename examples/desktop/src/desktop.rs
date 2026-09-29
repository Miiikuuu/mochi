//! Linux desktop shell. X11/XWayland presentation stays outside Core/Runtime.
use crate::{
    drag::Drag,
    gaze::Gaze,
    idle::Idle,
    input_region::{InputRegion, Mode as InputMode, Pose as InputPose},
    jump::Jump,
    motion::{Bounds, Keys, Motion, SPEED},
    pixels,
    shake::Shake,
    upload::Upload,
    work::Work,
};
use mochi_core::{MateEvent, Scene, SceneContent, Vec2};
use mochi_host::{Host, InstanceHandle, Presentation};
use mochi_package::{LoadOptions, LoadedPackage};
use mochi_preview::{FrameClock, Viewport, FIXED_STEP};
use mochi_renderer_software::{
    CanvasRenderer, RenderError, RenderLayer, RenderOptions, SamplingMode,
};
use std::{
    collections::BTreeMap,
    error::Error,
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use x11rb::{
    connection::Connection,
    protocol::{
        randr::ConnectionExt as _,
        render::ConnectionExt as _,
        shape::{ConnectionExt as _, SK, SO},
        xkb::{self, ConnectionExt as _},
        xproto::*,
        Event,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
x11rb::atom_manager! {
    Atoms: AtomsCookie {
        WM_PROTOCOLS, WM_DELETE_WINDOW, WM_NORMAL_HINTS, WM_SIZE_HINTS, WM_HINTS,
        _NET_WM_NAME, UTF8_STRING, _NET_WM_STATE, _NET_WM_STATE_ABOVE,
        _NET_WM_STATE_SKIP_TASKBAR, _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_WINDOW_TYPE, _NET_WM_WINDOW_TYPE_UTILITY, _NET_WM_PID,
        _NET_WM_USER_TIME, _MOTIF_WM_HINTS,
        _MOCHI_DESKTOP_OWNER, _MOCHI_DESKTOP_CONTROL,
        _MOCHI_DESKTOP_CURSOR,
    }
}

#[derive(Clone, Copy)]
struct Press {
    root: [i32; 2],
    origin: [i32; 2],
    local: Vec2,
    dragging: bool,
    threshold: i32,
}
impl Press {
    fn moved(&mut self, root: [i32; 2]) -> ([i32; 2], bool) {
        let delta = [root[0] - self.root[0], root[1] - self.root[1]];
        let started = !self.dragging
            && i64::from(delta[0]).pow(2) + i64::from(delta[1]).pow(2)
                >= i64::from(self.threshold).pow(2);
        self.dragging |= started;
        (
            [self.origin[0] + delta[0], self.origin[1] + delta[1]],
            started,
        )
    }
}

struct Pet {
    policy: crate::actions::Policy,
    handle: InstanceHandle,
    gc: Gcontext,
    backing: Pixmap,
    viewport: Viewport,
    renderer: CanvasRenderer,
    last_scene: Option<Scene>,
    packed: Vec<u8>,
    shape: Vec<Rectangle>,
    shape_updates: u64,
    input_region: InputRegion,
    #[cfg(feature = "input-test-hooks")]
    freeze_input_shape: bool,
    repaint: Option<Rectangle>,
    copy_pixels: u64,
    press: Option<Press>,
    size: u16,
    paused: bool,
    drag_threshold: i32,
    drawn: u64,
    timing: [Duration; 4],
    alpha_bounds: Option<Rectangle>,
    pending_scene: Option<Scene>,
    upload: Upload,
    transfer: mochi_renderer_software::GpuTransferStats,
    upload_bytes: u64,
    gpu_failure_reported: bool,
    keys: Keys,
    motion: Motion,
    walking: bool,
    idle: Option<Idle>,
    work: Option<Work>,
    hovered: bool,
    window_position: [i32; 2],
    gaze: Option<Gaze>,
    drag: Option<Drag>,
    shake: Option<Shake>,
    jump: Option<Jump>,
}
impl Pet {
    fn jumping(&self) -> bool {
        self.jump.as_ref().is_some_and(Jump::active)
    }
    fn local(&self, x: i16, y: i16) -> Result<Vec2> {
        self.local_point(Vec2::new(f64::from(x), f64::from(y)))
    }
    fn local_point(&self, point: Vec2) -> Result<Vec2> {
        Ok(self.viewport.to_canvas(self.motion.unproject(point))?)
    }
    fn action(&mut self, host: &mut Host, alias: &str, trace: bool) -> Result<()> {
        if let Some(shake) = &mut self.shake {
            shake.cancel();
        }
        if let Some(drag) = &mut self.drag {
            drag.clear();
        }
        dispatch(
            host,
            &self.handle,
            MateEvent::Custom {
                name: mochi_core::Namespace::new(format!("mochi.desktop.{alias}"))?,
                payload: mochi_core::Value::Object(BTreeMap::new()),
            },
            trace,
        )
    }
    fn start_jump(&mut self, host: &mut Host, trace: bool) -> Result<()> {
        if self.jump.is_none()
            || self.jumping()
            || self.paused
            || self.press.is_some()
            || self.drag.as_ref().is_some_and(Drag::active)
            || self.shake.as_ref().is_some_and(Shake::active)
            || !["idle", "walk"].contains(
                &host
                    .get(&self.handle)?
                    .runtime()
                    .state()
                    .current_animation
                    .as_str(),
            )
        {
            return Ok(());
        }
        self.cancel_motion(host, trace)?;
        // A manually selected walk clip also returns to the grounded base.
        if host.get(&self.handle)?.runtime().state().current_animation != "idle" {
            self.action(host, "idle", trace)?;
        }
        let time = self
            .gaze
            .as_ref()
            .expect("validated jump follow")
            .pose_time();
        let accepted = self.jump.as_mut().expect("present").start(time);
        if trace {
            println!("JumpStart accepted={accepted}");
        }
        Ok(())
    }
    fn walk(&mut self, host: &mut Host, walking: bool, trace: bool) -> Result<()> {
        if walking != self.walking {
            self.action(host, if walking { "walk" } else { "idle" }, trace)?;
            self.walking = walking;
        }
        Ok(())
    }
    fn cancel_motion(&mut self, host: &mut Host, trace: bool) -> Result<()> {
        self.interrupt_idle(host, trace)?;
        self.keys.cancel_motion();
        self.walk(host, false, trace)
    }
    fn interrupt_idle(&mut self, host: &mut Host, trace: bool) -> Result<()> {
        let owned_action = self.idle.as_ref().is_some_and(|idle| {
            idle.owns_action(
                &host
                    .get(&self.handle)
                    .expect("live pet")
                    .runtime()
                    .state()
                    .current_animation,
            )
        });
        if self.idle.as_mut().is_some_and(Idle::interrupt) {
            // Never overwrite a newer interaction clip when yielding control.
            if host.get(&self.handle)?.runtime().state().current_animation == "walk" {
                self.walk(host, false, trace)?;
            } else if owned_action {
                self.action(host, "idle", trace)?;
            } else {
                self.walking = false;
            }
        }
        Ok(())
    }
    fn idle_eligible(&self, host: &Host) -> Result<bool> {
        let animation = &host.get(&self.handle)?.runtime().state().current_animation;
        Ok(!self.paused
            && !self.hovered
            && self.press.is_none()
            && !self.keys.controlling()
            && !self.jumping()
            && !self.drag.as_ref().is_some_and(Drag::active)
            && !self.shake.as_ref().is_some_and(Shake::active)
            && (animation == "idle"
                || self
                    .idle
                    .as_ref()
                    .is_some_and(|idle| idle.owns_action(animation))
                || (animation == "walk" && self.idle.as_ref().is_some_and(Idle::active))))
    }
    fn update_work(&mut self, host: &mut Host, trace: bool) -> Result<()> {
        let ready = !self.paused
            && self.press.is_none()
            && !self.keys.controlling()
            && !self.jumping()
            && !self.drag.as_ref().is_some_and(Drag::active)
            && !self.shake.as_ref().is_some_and(Shake::active);
        let current = host
            .get(&self.handle)?
            .runtime()
            .state()
            .current_animation
            .clone();
        if let Some(alias) = self
            .work
            .as_mut()
            .and_then(|work| work.transition(&current, ready))
        {
            if alias == "work" {
                self.interrupt_idle(host, trace)?;
            }
            self.action(host, alias, trace)?;
            if trace {
                println!("WorkAction action={alias}");
            }
        }
        Ok(())
    }
    fn sync_position(&mut self, conn: &RustConnection, window: Window, root: Window) -> Result<()> {
        // Read the actual WM position once per gesture, never feed asynchronous
        // ConfigureNotify acknowledgments back into the fractional integrator.
        let origin = conn.translate_coordinates(window, root, 0, 0)?.reply()?;
        let position = [i32::from(origin.dst_x), i32::from(origin.dst_y)];
        self.motion.place(position);
        self.window_position = position;
        let center = position.map(|p| f64::from(p) + f64::from(self.size) / 2.0);
        if let Some(bounds) = conn
            .randr_get_monitors(root, true)?
            .reply()?
            .monitors
            .iter()
            .map(monitor_bounds)
            .min_by(|a, b| {
                a.distance_squared(center)
                    .total_cmp(&b.distance_squared(center))
            })
        {
            self.motion.bounds = bounds;
        }
        Ok(())
    }
    fn hit(&self, x: i16, y: i16) -> bool {
        crate::input_region::contains(&self.shape, x, y)
    }
    fn prepare(&mut self, host: &Host) -> Result<()> {
        self.timing = [Duration::ZERO; 4];
        self.transfer = Default::default();
        self.upload_bytes = 0;
        let instance = host.get(&self.handle)?;
        let mut scene = instance.runtime().scene()?;
        if let Some(gaze) = &mut self.gaze {
            gaze.project(
                &mut scene,
                !self.paused
                    && self.policy.allows("gaze", "pointer")
                    && !self.press.is_some_and(|p| p.dragging)
                    && instance.runtime().state().current_animation == "idle",
            );
        }
        if let Some(drag) = &self.drag {
            drag.project(&mut scene);
        }
        if let Some(shake) = &self.shake {
            shake.project(&mut scene);
        }
        if let Some(jump) = &self.jump {
            jump.project(&mut scene);
        }
        let mut scene = self.viewport.project_scene(&scene);
        self.motion.project(&mut scene);
        if self.last_scene.as_ref() != Some(&scene) {
            let started = Instant::now();
            self.renderer.begin_render_layers(&[RenderLayer {
                package: instance.package(),
                scene: &scene,
            }])?;
            self.timing[0] = started.elapsed();
            self.pending_scene = Some(scene);
        }
        Ok(())
    }
    fn finish_readback(&mut self, host: &Host) -> Result<()> {
        if let Some(scene) = &self.pending_scene {
            let started = Instant::now();
            if let Err(error) = self.renderer.finish_render() {
                match error {
                    RenderError::GpuUnavailable(_) => {
                        let instance = host.get(&self.handle)?;
                        self.renderer.render_layers(&[RenderLayer {
                            package: instance.package(),
                            scene,
                        }])?;
                    }
                    error => return Err(error.into()),
                }
            }
            if !self.gpu_failure_reported {
                if let Some(reason) = self.renderer.gpu_fallback_reason() {
                    eprintln!(
                        "{}: Spine GPU fallback to CPU: {reason}",
                        host.get(&self.handle)?.runtime().instance_id()
                    );
                    self.gpu_failure_reported = true;
                }
            }
            self.transfer = self.renderer.gpu_transfer_stats();
            self.timing[0] += started.elapsed();
        }
        Ok(())
    }
    fn draw(&mut self, conn: &RustConnection, window: Window, host: &Host) -> Result<()> {
        if let Some(scene) = self.pending_scene.take() {
            let pose = InputPose {
                mirrored: self.motion.mirrored,
                dragging: self.drag.as_ref().is_some_and(Drag::active),
                reacting: self.shake.as_ref().is_some_and(Shake::active),
                jumping: self.jumping(),
            };
            let capture_input = self.input_region.refresh(
                &host.get(&self.handle)?.runtime().state().current_animation,
                pose,
                Instant::now(),
            ) || self.shape.is_empty();
            let started = Instant::now();
            let frame = self.renderer.frame();
            let packed = if let Some(bytes) = frame.premultiplied_bgra8() {
                bytes
            } else {
                self.packed = pixels::argb(
                    frame.rgba8(),
                    conn.setup().image_byte_order == ImageOrder::LSB_FIRST,
                );
                &self.packed
            };
            self.timing[1] = started.elapsed();
            let started = Instant::now();
            // Alpha is byte 3 in RGBA/BGRA; big-endian packed ARGB needs RGBA.
            let alpha_pixels = if conn.setup().image_byte_order == ImageOrder::LSB_FIRST {
                packed
            } else {
                frame.rgba8()
            };
            let (shape, bounds) = if capture_input {
                pixels::shape_and_bounds(alpha_pixels, self.size)
            } else {
                (Vec::new(), pixels::alpha_bounds(alpha_pixels, self.size))
            };
            let damage = if self.last_scene.is_none() {
                Some(Rectangle {
                    x: 0,
                    y: 0,
                    width: self.size,
                    height: self.size,
                })
            } else {
                pixels::damage(self.alpha_bounds, bounds)
            };
            self.alpha_bounds = bounds;
            // A fully invisible frame must never leave a stale input island.
            let update_shape = (capture_input || bounds.is_none()) && shape != self.shape;
            #[cfg(feature = "input-test-hooks")]
            let update_shape = update_shape && (!self.freeze_input_shape || self.drawn == 0);
            if update_shape {
                self.shape_updates += 1;
                conn.shape_rectangles(
                    SO::SET,
                    SK::INPUT,
                    ClipOrdering::YX_SORTED,
                    window,
                    0,
                    0,
                    &shape,
                )?;
                self.shape = shape;
            }
            self.last_scene = Some(scene);
            self.drawn += 1;
            self.repaint = pixels::damage(self.repaint, damage);
            self.timing[2] = started.elapsed();
            let started = Instant::now();
            if let Some(damage) = damage {
                self.upload_bytes =
                    self.upload
                        .write(conn, self.backing, self.gc, packed, damage)?;
            }
            self.timing[3] = started.elapsed();
        }
        self.copy_pixels = 0;
        if let Some(damage) = self.repaint.take() {
            // The backing already contains the whole image. Copy only the dirty
            // rectangle so XWayland/Mutter do not damage the transparent margins.
            conn.copy_area(
                self.backing,
                window,
                self.gc,
                damage.x,
                damage.y,
                damage.x,
                damage.y,
                damage.width,
                damage.height,
            )?;
            self.copy_pixels = u64::from(damage.width) * u64::from(damage.height);
        }
        Ok(())
    }
}

fn dispatch(host: &mut Host, handle: &InstanceHandle, event: MateEvent, trace: bool) -> Result<()> {
    let tick = matches!(event, MateEvent::Tick { .. });
    let step = host.dispatch(handle, event)?;
    if !step.requests.is_empty() {
        return Err("desktop has no capability executor".into());
    }
    if trace
        && (!tick
            || step.output.trace.iter().any(|t| {
                t.actions
                    .iter()
                    .any(|a| matches!(a, mochi_core::MateAction::PlayAnimation(_)))
            }))
    {
        println!(
            "{}: {:?}",
            host.get(handle)?.runtime().instance_id(),
            step.output.trace
        );
    }
    Ok(())
}

fn remove(
    conn: &RustConnection,
    host: &mut Host,
    pets: &mut BTreeMap<Window, Pet>,
    window: Window,
) -> Result<()> {
    if let Some(pet) = pets.remove(&window) {
        host.remove(&pet.handle)?;
        conn.free_gc(pet.gc)?.check()?;
        conn.free_pixmap(pet.backing)?.check()?;
        conn.destroy_window(window)?.check()?;
        println!("Removed window={window}; live={}", host.len());
    }
    Ok(())
}

fn monitor_bounds(m: &x11rb::protocol::randr::MonitorInfo) -> Bounds {
    Bounds {
        origin: [i32::from(m.x), i32::from(m.y)],
        size: [u32::from(m.width), u32::from(m.height)],
    }
}

fn detectable_repeat(conn: &RustConnection) -> Result<bool> {
    if !conn.xkb_use_extension(1, 0)?.reply()?.supported {
        return Ok(false);
    }
    let flag = xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT;
    let reply = conn
        .xkb_per_client_flags(
            xkb::ID::USE_CORE_KBD.into(),
            flag,
            flag,
            0u32.into(),
            0u32.into(),
            0u32.into(),
        )?
        .reply()?;
    Ok(u32::from(reply.value) & u32::from(flag) != 0)
}

pub fn run() -> Result<()> {
    let mut paths = Vec::new();
    let mut size: u16 = 256;
    let mut trace = false;
    let mut stop = false;
    let mut metrics_path = None;
    let mut static_cursor = true;
    let mut input_mode = InputMode::Stable;
    let mut idle_after = 30.0_f64;
    #[cfg(feature = "input-test-hooks")]
    let mut freeze_input_shape = false;
    #[cfg(feature = "gpu-test-hooks")]
    let mut test_loss_after = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        #[cfg(feature = "input-test-hooks")]
        if arg == "--test-freeze-input-shape" {
            freeze_input_shape = true;
            continue;
        }
        #[cfg(feature = "gpu-test-hooks")]
        if arg == "--test-gpu-loss-after-ms" {
            let ms: u64 = args
                .next()
                .ok_or("missing loss delay")?
                .to_str()
                .ok_or("invalid loss delay")?
                .parse()?;
            if !(1..=60_000).contains(&ms) {
                return Err("loss delay must be 1..60000 ms".into());
            }
            test_loss_after = Some(Duration::from_millis(ms));
            continue;
        }
        if arg == "--help" {
            println!("mochi-desktop [--size 128..512] [--cursor static|inherit] [--input-shape stable|pixel] [--idle-walk-after seconds] [--trace] [--metrics file.csv] [one.mochi ... four.mochi]\nDefault: four Lime Squad PNG characters. Linux X11/XWayland, transparent and above normal windows.\nCursor: static frame of the current theme over pets (default); --cursor inherit restores the system cursor animation. No global cursor changes.\nInput shape: stable pose outline (default), refreshed on action/facing changes; moving edges can differ from the image. --input-shape pixel tracks every frame exactly.\nIdle walk: Spine characters stroll 48..96 logical px after 30..45 seconds without interaction; --idle-walk-after sets the minimum wait (0 disables). Hover and user actions take priority.\nLeft click: select/interact; hold arrow keys: move selected character; release: stop.\nDrag: move; right click: open character menu; Escape: quit all.\nSpine demo keys 1..5: idle, interact, walk in place, sit, sleep. PNG without walk bindings retains its current animation.\nNo autostart or desktop settings changes. File budget: 256 per package.");
            return Ok(());
        } else if arg == "--trace" {
            trace = true;
        } else if arg == "--stop" {
            stop = true;
        } else if arg == "--metrics" {
            metrics_path = Some(PathBuf::from(args.next().ok_or("missing metrics path")?));
        } else if arg == "--cursor" {
            let mode = args.next().ok_or("missing cursor mode")?;
            static_cursor = match mode.to_str() {
                Some("static") => true,
                Some("inherit") => false,
                _ => return Err("cursor mode must be static or inherit".into()),
            };
        } else if arg == "--input-shape" {
            let mode = args.next().ok_or("missing input shape mode")?;
            input_mode = match mode.to_str() {
                Some("stable") => InputMode::Stable,
                Some("pixel") => InputMode::Pixel,
                _ => return Err("input shape mode must be stable or pixel".into()),
            };
        } else if arg == "--idle-walk-after" {
            idle_after = args
                .next()
                .ok_or("missing idle walk delay")?
                .to_str()
                .ok_or("invalid idle walk delay")?
                .parse()?;
            if !idle_after.is_finite() || !(0.0..=3600.0).contains(&idle_after) {
                return Err("idle walk delay must be 0..3600 seconds (0 disables it)".into());
            }
        } else if arg == "--size" {
            size = args
                .next()
                .ok_or("missing size")?
                .to_str()
                .ok_or("invalid size")?
                .parse()?;
            if !(128..=512).contains(&size) {
                return Err("size must be 128..=512".into());
            }
        } else if arg.to_string_lossy().starts_with('-') {
            return Err("unknown option; use --help".into());
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    if paths.is_empty() {
        paths = crate::install_paths::default_packages();
    }
    if paths.len() > 4 {
        return Err("desktop prototype supports at most four packages".into());
    }
    let (conn, screen_num) = x11rb::connect(None)?;
    let atoms = Atoms::new(&conn)?.reply()?;
    let owner = conn
        .get_selection_owner(atoms._MOCHI_DESKTOP_OWNER)?
        .reply()?
        .owner;
    if stop {
        if owner != 0 {
            conn.send_event(
                false,
                owner,
                EventMask::NO_EVENT,
                ClientMessageEvent::new(32, owner, atoms._MOCHI_DESKTOP_CONTROL, [1, 0, 0, 0, 0]),
            )?
            .check()?;
            conn.flush()?;
        }
        println!("Desktop stop requested");
        return Ok(());
    }
    if owner != 0 {
        return Err("desktop characters are already running; use --stop to close them".into());
    }
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let xwayland = conn.query_extension(b"XWAYLAND")?.reply()?.present;
    let resource_manager = conn
        .get_property(
            false,
            root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            0,
            16384,
        )?
        .reply()?;
    let resources = String::from_utf8_lossy(&resource_manager.value);
    let dpi = resources
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim() == "Xft.dpi")
                .then(|| value.trim().parse::<f64>().ok())
                .flatten()
        })
        .filter(|dpi| dpi.is_finite() && (96.0..=384.0).contains(dpi))
        .unwrap_or(96.0);
    let scale = dpi / 96.0;
    size = (f64::from(size) * scale).round() as u16;
    println!("X11 desktop scale={scale}; physical buffer={size}px");
    println!(
        "Desktop input shape: {}",
        if input_mode == InputMode::Stable {
            "stable pose"
        } else {
            "per-frame pixels"
        }
    );
    let shape = conn.shape_query_version()?.reply()?;
    if (shape.major_version, shape.minor_version) < (1, 1) {
        return Err("X Shape 1.1 input regions are required".into());
    }
    let formats = conn.render_query_pict_formats()?.reply()?;
    let format = formats
        .formats
        .iter()
        .find(|f| {
            f.depth == 32
                && f.direct.alpha_mask == 255
                && f.direct.alpha_shift == 24
                && f.direct.red_mask == 255
                && f.direct.red_shift == 16
                && f.direct.green_mask == 255
                && f.direct.green_shift == 8
                && f.direct.blue_mask == 255
                && f.direct.blue_shift == 0
        })
        .ok_or("no ARGB32 X11 visual available")?
        .id;
    let visual = formats.screens[screen_num]
        .depths
        .iter()
        .flat_map(|d| &d.visuals)
        .find(|v| v.format == format)
        .ok_or("no ARGB32 visual on this screen")?
        .visual;
    let cursor = if static_cursor {
        match crate::cursor::load(&conn, root, format, &resources, scale) {
            Ok(cursor) => cursor,
            Err(error) => {
                eprintln!(
                    "Desktop cursor: inheriting system cursor; static theme unavailable: {error}"
                );
                0
            }
        }
    } else {
        println!("Desktop cursor: inherit system cursor");
        0
    };
    let monitors = conn.randr_get_monitors(root, true)?.reply()?.monitors;
    let monitor = monitors
        .iter()
        .find(|m| m.primary)
        .or(monitors.first())
        .ok_or("no active monitor")?;
    let colormap = conn.generate_id()?;
    conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual)?
        .check()?;
    let control = conn.generate_id()?;
    conn.create_window(
        0,
        control,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new(),
    )?
    .check()?;
    conn.set_selection_owner(control, atoms._MOCHI_DESKTOP_OWNER, x11rb::CURRENT_TIME)?
        .check()?;
    let mut host = Host::default();
    let policies = crate::actions::Policy::load()?;
    let mut pets = BTreeMap::new();
    let count = paths.len() as i32;
    let gap = (8.0 * scale).round() as i32;
    let total = count * (i32::from(size) + gap) - gap;
    if total > i32::from(monitor.width) {
        return Err("characters do not fit this display; reduce --size".into());
    }
    for (index, path) in paths.into_iter().enumerate() {
        let package = Arc::new(LoadedPackage::load_with_options(
            path,
            LoadOptions {
                max_files: 256,
                ..LoadOptions::default()
            },
        )?);
        let title = format!(
            "Mochi · {} · 右键菜单 / Esc 退出全部",
            package.manifest().name
        );
        let policy = policies.get(index).cloned().unwrap_or_default();
        let behavior_policy = policy.clone();
        let handle = host.spawn_with_behavior(
            format!("desktop-{}", index + 1),
            package,
            Presentation::default(),
            |inner| behavior_policy.wrap(inner),
        )?;
        let initial = host.get(&handle)?.runtime().scene()?;
        let logical = match initial.nodes.first().map(|n| &n.content) {
            Some(SceneContent::Sprite { size, .. } | SceneContent::Skeletal { size, .. }) => {
                [size.x.ceil() as u32, size.y.ceil() as u32]
            }
            _ => return Err("desktop requires a sprite or skeletal package".into()),
        };
        let viewport = Viewport::new(logical, [u32::from(size); 2])?.ok_or("empty sprite")?;
        let window = conn.generate_id()?;
        let gc = conn.generate_id()?;
        let backing = conn.generate_id()?;
        let x = i32::from(monitor.x)
            + (i32::from(monitor.width) - total).max(0) / 2
            + index as i32 * (i32::from(size) + gap);
        let y = i32::from(monitor.y)
            + (i32::from(monitor.height) - i32::from(size) - (24.0 * scale).round() as i32).max(0);
        conn.create_window(
            32,
            window,
            root,
            x as i16,
            y as i16,
            size,
            size,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .colormap(colormap)
                .background_pixel(0)
                .border_pixel(0)
                .cursor(cursor)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::STRUCTURE_NOTIFY
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        // Gaze samples the pointer separately. Unpressed high-rate
                        // motion must not wake/rebuild every character's scene.
                        | EventMask::BUTTON1_MOTION
                        | EventMask::ENTER_WINDOW
                        | EventMask::LEAVE_WINDOW
                        | EventMask::KEY_PRESS
                        | EventMask::KEY_RELEASE
                        | EventMask::FOCUS_CHANGE,
                ),
        )?
        .check()?;
        // Expose the owned cursor ID for reversible native diagnostics.
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._MOCHI_DESKTOP_CURSOR,
            AtomEnum::CARDINAL,
            &[cursor],
        )?
        .check()?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_NAME,
            atoms.UTF8_STRING,
            title.as_bytes(),
        )?
        .check()?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"mochi-desktop\0MochiDesktop\0",
        )?
        .check()?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms.WM_PROTOCOLS,
            AtomEnum::ATOM,
            &[atoms.WM_DELETE_WINDOW],
        )?
        .check()?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_STATE,
            AtomEnum::ATOM,
            &[
                atoms._NET_WM_STATE_ABOVE,
                atoms._NET_WM_STATE_SKIP_TASKBAR,
                atoms._NET_WM_STATE_SKIP_PAGER,
            ],
        )?
        .check()?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_WINDOW_TYPE,
            AtomEnum::ATOM,
            &[atoms._NET_WM_WINDOW_TYPE_UTILITY],
        )?
        .check()?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_PID,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?
        .check()?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_USER_TIME,
            AtomEnum::CARDINAL,
            &[0],
        )?
        .check()?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._MOTIF_WM_HINTS,
            atoms._MOTIF_WM_HINTS,
            &[2, 0, 0, 0, 0],
        )?
        .check()?;
        let mut hints = [0; 18];
        hints[0] = 1 | (1 << 4) | (1 << 5);
        hints[1] = x as u32;
        hints[2] = y as u32;
        for value in &mut hints[5..9] {
            *value = u32::from(size);
        }
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms.WM_NORMAL_HINTS,
            atoms.WM_SIZE_HINTS,
            &hints,
        )?
        .check()?;
        conn.create_gc(gc, window, &CreateGCAux::new().graphics_exposures(0))?
            .check()?;
        conn.create_pixmap(32, backing, window, size, size)?
            .check()?;
        let gaze = host
            .get(&handle)?
            .package()
            .spine("spine.toml")
            .ok()
            .and_then(|s| s.follow.clone().map(|f| Gaze::new(f, s.camera)));
        let drag = host
            .get(&handle)?
            .package()
            .spine("spine.toml")
            .ok()
            .and_then(|s| {
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
            })
            .filter(|_| policy.allows("drag", "drag"));
        let shake = host
            .get(&handle)?
            .package()
            .spine("spine.toml")
            .ok()
            .and_then(|s| {
                Some(Shake::new(
                    s.shake.clone()?,
                    s.drag.clone()?,
                    s.follow.clone()?,
                ))
            })
            .filter(|_| policy.allows("shake", "shake") && policy.allows("drag", "drag"));
        let jump = host
            .get(&handle)?
            .package()
            .spine("spine.toml")
            .ok()
            .and_then(|s| Some(Jump::new(s.jump.clone()?, s.follow.clone()?, s.camera)))
            .filter(|_| policy.enabled("jump"));
        let idle = (idle_after > 0.0 && host.get(&handle)?.package().spine("spine.toml").is_ok())
            .then(|| {
                Idle::new(
                    idle_after,
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64
                        ^ u64::from(window),
                )
                .with_action(
                    host.get(&handle)
                        .expect("live pet")
                        .package()
                        .spine("spine.toml")
                        .expect("Spine pet")
                        .idle_action
                        .clone()
                        .filter(|alias| policy.allows(alias, "idle")),
                )
                .with_walk(policy.allows("walk", "idle"))
            });
        let work = (policy.allows("work", "task")
            && host.get(&handle)?.package().has_animation("work"))
        .then(Work::default);
        let mut pet = Pet {
            policy,
            handle,
            gc,
            backing,
            viewport,
            renderer: CanvasRenderer::with_scaled_cache(
                RenderOptions {
                    width: u32::from(size),
                    height: u32::from(size),
                    background: [0; 4],
                    sampling: SamplingMode::Coverage,
                    ..RenderOptions::default()
                },
                4 * 1024 * 1024,
            )?,
            last_scene: None,
            packed: Vec::new(),
            shape: Vec::new(),
            shape_updates: 0,
            input_region: InputRegion::new(input_mode),
            #[cfg(feature = "input-test-hooks")]
            freeze_input_shape,
            repaint: Some(Rectangle {
                x: 0,
                y: 0,
                width: size,
                height: size,
            }),
            copy_pixels: 0,
            press: None,
            size,
            paused: false,
            drag_threshold: (4.0 * scale).round() as i32,
            drawn: 0,
            timing: [Duration::ZERO; 4],
            alpha_bounds: None,
            pending_scene: None,
            upload: Upload::new(size),
            transfer: Default::default(),
            upload_bytes: 0,
            gpu_failure_reported: false,
            keys: Keys::default(),
            motion: Motion::new([x, y], monitor_bounds(monitor), size),
            walking: false,
            idle,
            work,
            hovered: false,
            window_position: [x, y],
            gaze,
            drag,
            shake,
            jump,
        };
        if host.get(&pet.handle)?.package().spine("spine.toml").is_ok()
            && std::env::var_os("MOCHI_SPINE_SOFTWARE").is_none()
        {
            match pet.renderer.enable_spine_gpu() {
                Ok(name) => {
                    pet.renderer.prefer_premultiplied_bgra(
                        conn.setup().image_byte_order == ImageOrder::LSB_FIRST,
                    );
                    println!("Spine raster: GPU ({name}); evaluator: Wasm");
                }
                Err(error) => eprintln!("Spine GPU unavailable: {error}; using software raster"),
            }
        }
        pet.prepare(&host)?;
        pet.finish_readback(&host)?;
        pet.draw(&conn, window, &host)?;
        conn.map_window(window)?.check()?;
        pets.insert(window, pet);
        println!(
            "Desktop window={window} role={} x={x} y={y} size={size}",
            index + 1
        );
    }
    conn.flush()?;
    let first_key = conn.setup().min_keycode;
    let bubble_roles: Vec<_> = pets
        .iter()
        .map(|(&window, pet)| {
            host.get(&pet.handle)
                .map(|instance| (window, instance.package().manifest().name.clone()))
        })
        .collect::<std::result::Result<_, _>>()?;
    let primary = bubble_roles.first().map(|role| role.0);
    let mut bubbles = match crate::bubbles::Bubbles::start(&bubble_roles) {
        Ok(bubbles) => Some(bubbles),
        Err(error) => {
            eprintln!("Desktop bubbles unavailable: {error}");
            None
        }
    };
    let key_count = conn.setup().max_keycode - first_key + 1;
    let mut keyboard = conn.get_keyboard_mapping(first_key, key_count)?.reply()?;
    // Per-client XKB flag; never changes the desktop's global repeat settings.
    let detectable_repeat = detectable_repeat(&conn).unwrap_or(false);
    println!("Keyboard: detectable repeat={detectable_repeat}; arrow speed={SPEED} logical px/s");
    println!("Idle walk: after {idle_after}..{} seconds without character interaction; 48..96 logical px (0 delay disables)", idle_after * 1.5);
    let epoch = Instant::now();
    let mut metrics = metrics_path
        .map(|path| {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map(std::io::BufWriter::new)
        })
        .transpose()?;
    if let Some(writer) = &mut metrics {
        writeln!(
            writer,
            "wall_ms,render_ms,total_drawn,live,raster_ms,pack_ms,shape_ms,upload_ms,gpu_wait_ms,gpu_copy_ms,gpu_bytes,upload_bytes,events,motion_events,pointer_queries,shape_updates,shape_rectangles,copy_pixels"
        )?;
    }
    let mut clock = FrameClock::default();
    let mut quit = false;
    let mut queued = None;
    let mut event_count = 0u64;
    let mut motion_count = 0u64;
    let mut pointer_queries = 0u64;
    let mut next_gaze_poll = Duration::ZERO;
    let desktop_pointer =
        if xwayland && (bubbles.is_some() || pets.values().any(|p| p.gaze.is_some())) {
            match crate::pointer::DesktopPointer::start() {
                Ok(pointer) => Some(pointer),
                Err(error) => {
                    eprintln!("Desktop gaze: could not start GNOME pointer bridge: {error}");
                    None
                }
            }
        } else {
            None
        };
    println!(
        "Ready: {} transparent desktop characters; click / drag / right-click menu / Esc quit",
        host.len()
    );
    while !quit && !pets.is_empty() {
        if let Some(ui) = bubbles.as_mut() {
            for command in ui.commands() {
                match command {
                    crate::bubbles::UiCommand::Remove(window) => {
                        remove(&conn, &mut host, &mut pets, window)?;
                    }
                    crate::bubbles::UiCommand::Work { window, active } => {
                        if let Some(pet) = pets.get_mut(&window) {
                            if let Some(work) = &mut pet.work {
                                work.request(active);
                                if active {
                                    pet.interrupt_idle(&mut host, trace)?;
                                }
                                pet.update_work(&mut host, trace)?;
                            }
                        }
                    }
                }
            }
        }
        // Bound input work so continuous mouse motion cannot starve animation.
        for _ in 0..256 {
            let event = match queued.take() {
                Some(event) => Some(event),
                None => conn.poll_for_event()?,
            };
            let Some(event) = event else {
                break;
            };
            event_count += 1;
            motion_count += u64::from(matches!(&event, Event::MotionNotify(_)));
            if trace {
                if let Event::ClientMessage(message) = &event {
                    println!("Desktop control: {message:?}");
                }
                if matches!(
                    &event,
                    Event::ButtonPress(_)
                        | Event::ButtonRelease(_)
                        | Event::FocusOut(_)
                        | Event::MotionNotify(_)
                ) {
                    println!("Desktop input: {event:?}");
                }
            }
            match event {
                Event::Expose(e) => {
                    if let Some(pet) = pets.get_mut(&e.window) {
                        pet.repaint = pixels::damage(
                            pet.repaint,
                            Some(Rectangle {
                                x: e.x as i16,
                                y: e.y as i16,
                                width: e.width,
                                height: e.height,
                            }),
                        );
                    }
                }
                // The GTK menu runs in a separate process. Opening it on press lets the
                // matching release arrive at its grab and immediately close it.
                Event::ButtonRelease(e) if e.detail == 3 => {
                    if pets
                        .get(&e.event)
                        .is_some_and(|pet| pet.press.is_none() && pet.hit(e.event_x, e.event_y))
                    {
                        if let Some(ui) = bubbles.as_mut() {
                            ui.context_menu(e.event, e.root_x, e.root_y, e.time);
                        }
                    }
                }
                Event::ButtonPress(e) if e.detail == 1 => {
                    if let Some(pet) = pets.get_mut(&e.event) {
                        pet.interrupt_idle(&mut host, trace)?;
                        if !pet.jumping() && pet.press.is_none() && pet.hit(e.event_x, e.event_y) {
                            pet.cancel_motion(&mut host, trace)?;
                            let origin =
                                conn.translate_coordinates(e.event, root, 0, 0)?.reply()?;
                            pet.press = Some(Press {
                                root: [i32::from(e.root_x), i32::from(e.root_y)],
                                origin: [i32::from(origin.dst_x), i32::from(origin.dst_y)],
                                local: pet.local(e.event_x, e.event_y)?,
                                dragging: false,
                                threshold: pet.drag_threshold,
                            });
                            conn.set_input_focus(InputFocus::PARENT, e.event, e.time)?
                                .check()?;
                        }
                    }
                }
                Event::MotionNotify(e) => {
                    if let Some(pet) = pets.get_mut(&e.event) {
                        if let Some(mut press) = pet.press {
                            let (position, started) =
                                press.moved([i32::from(e.root_x), i32::from(e.root_y)]);
                            if started {
                                if pet.drag.is_some()
                                    && !["idle", "wave"].contains(
                                        &host
                                            .get(&pet.handle)?
                                            .runtime()
                                            .state()
                                            .current_animation
                                            .as_str(),
                                    )
                                {
                                    pet.action(&mut host, "idle", trace)?;
                                }
                                if let Some(shake) = &mut pet.shake {
                                    shake.begin(e.time);
                                }
                                if let Some(drag) = &mut pet.drag {
                                    drag.begin(
                                        &host.get(&pet.handle)?.runtime().scene()?,
                                        press.local,
                                        pet.last_scene.as_ref(),
                                    );
                                }
                                dispatch(
                                    &mut host,
                                    &pet.handle,
                                    MateEvent::DragStart {
                                        position: press.local,
                                    },
                                    trace,
                                )?;
                            }
                            if press.dragging {
                                if let Some(shake) = &mut pet.shake {
                                    shake.move_to(
                                        [
                                            i32::from(e.root_x) - press.root[0],
                                            i32::from(e.root_y) - press.root[1],
                                        ],
                                        scale,
                                        e.time,
                                    );
                                }
                                if let Some(drag) = &mut pet.drag {
                                    drag.move_to(
                                        [
                                            i32::from(e.root_x) - press.root[0],
                                            i32::from(e.root_y) - press.root[1],
                                        ],
                                        pet.size,
                                        pet.motion.mirrored,
                                    );
                                }
                                pet.motion.place(position);
                                pet.window_position = position;
                                conn.configure_window(
                                    e.event,
                                    &ConfigureWindowAux::new().x(position[0]).y(position[1]),
                                )?
                                .check()?;
                                dispatch(
                                    &mut host,
                                    &pet.handle,
                                    MateEvent::DragMove {
                                        position: pet.local(e.event_x, e.event_y)?,
                                    },
                                    trace,
                                )?;
                            }
                            pet.press = Some(press);
                        }
                    }
                }
                Event::ButtonRelease(e) if e.detail == 1 => {
                    if let Some(pet) = pets.get_mut(&e.event) {
                        pet.interrupt_idle(&mut host, trace)?;
                        if let Some(press) = pet.press.take() {
                            let position = pet.local(e.event_x, e.event_y)?;
                            if press.dragging {
                                if let Some(shake) = &mut pet.shake {
                                    shake.release();
                                }
                                if let Some(drag) = &mut pet.drag {
                                    drag.release();
                                }
                                dispatch(
                                    &mut host,
                                    &pet.handle,
                                    MateEvent::DragEnd { position },
                                    trace,
                                )?;
                            } else if pet.hit(e.event_x, e.event_y) {
                                if let Some(shake) = &mut pet.shake {
                                    shake.cancel();
                                }
                                dispatch(
                                    &mut host,
                                    &pet.handle,
                                    MateEvent::PointerClick {
                                        position,
                                        button: 1,
                                    },
                                    trace,
                                )?;
                            }
                        }
                    }
                }
                Event::EnterNotify(e) => {
                    if let Some(pet) = pets.get_mut(&e.event) {
                        pet.hovered = true;
                        pet.interrupt_idle(&mut host, trace)?;
                        dispatch(&mut host, &pet.handle, MateEvent::PointerEnter, trace)?;
                    }
                }
                Event::LeaveNotify(e) => {
                    if let Some(pet) = pets.get_mut(&e.event) {
                        pet.hovered = false;
                        pet.interrupt_idle(&mut host, trace)?;
                        dispatch(&mut host, &pet.handle, MateEvent::PointerLeave, trace)?;
                    }
                }
                Event::FocusOut(e)
                    if (e.mode == NotifyMode::NORMAL || e.mode == NotifyMode::WHILE_GRABBED)
                        && e.detail != NotifyDetail::INFERIOR
                        // After closing a bubble, focus can revert to PointerRoot.
                        // Selecting this pet then emits a Pointer FocusOut before
                        // its explicit FocusIn. It must not cancel the new drag.
                        && !(e.detail == NotifyDetail::POINTER
                            && conn.get_input_focus()?.reply()?.focus == e.event) =>
                {
                    if let Some(pet) = pets.get_mut(&e.event) {
                        pet.cancel_motion(&mut host, trace)?;
                        if let Some(press) = pet.press.take().filter(|p| p.dragging) {
                            if let Some(shake) = &mut pet.shake {
                                shake.release();
                            }
                            if let Some(drag) = &mut pet.drag {
                                drag.release();
                            }
                            dispatch(
                                &mut host,
                                &pet.handle,
                                MateEvent::DragEnd {
                                    position: press.local,
                                },
                                trace,
                            )?;
                        }
                    }
                }
                Event::FocusOut(e) => {
                    if let Some(pet) = pets.get_mut(&e.event) {
                        pet.cancel_motion(&mut host, trace)?;
                    }
                }
                Event::FocusIn(e) if e.mode == NotifyMode::NORMAL => {
                    if let Some(pet) = pets.get_mut(&e.event) {
                        pet.keys.focus(conn.query_keymap()?.reply()?.keys);
                    }
                }
                Event::KeyPress(e) if pets.contains_key(&e.event) => {
                    let start = usize::from(e.detail.saturating_sub(first_key))
                        * usize::from(keyboard.keysyms_per_keycode);
                    if let Some(key) = keyboard.keysyms.get(start) {
                        let pet = pets.get_mut(&e.event).expect("known window");
                        pet.interrupt_idle(&mut host, trace)?;
                        let was_still = pet.keys.direction() == [0, 0];
                        if !pet.keys.press(
                            e.detail,
                            *key,
                            !pet.paused
                                && pet.press.is_none()
                                && !pet.jumping()
                                && pet.policy.allows("walk", "keyboard"),
                        ) {
                            continue;
                        }
                        if was_still && pet.keys.direction() != [0, 0] {
                            pet.sync_position(&conn, e.event, root)?;
                        }
                        if *key == 0xff1b {
                            quit = true;
                        } else if let Some(alias) = pet.policy.key_action(*key).filter(|_| {
                            !pet.jumping()
                                && !(pet.drag.is_some() && pet.press.is_some_and(|p| p.dragging))
                        }) {
                            if alias == "jump" {
                                pet.start_jump(&mut host, trace)?;
                            } else {
                                pet.cancel_motion(&mut host, trace)?;
                                pet.action(&mut host, &alias, trace)?;
                            }
                        }
                    }
                }
                Event::KeyRelease(e) => {
                    if let Some(pet) = pets.get_mut(&e.event) {
                        // Old servers synthesize releases during auto-repeat.
                        // The physical key bitmap distinguishes them without
                        // disabling repeat globally or adding a per-frame query.
                        let still_down = !detectable_repeat && {
                            let keys = conn.query_keymap()?.reply()?.keys;
                            keys[usize::from(e.detail) / 8] & (1 << (e.detail % 8)) != 0
                        };
                        if !still_down {
                            pet.interrupt_idle(&mut host, trace)?;
                            pet.keys.release(e.detail);
                            if pet.keys.direction() == [0, 0] {
                                pet.walk(&mut host, false, trace)?;
                            }
                        }
                    }
                }
                Event::MappingNotify(_) => {
                    for pet in pets.values_mut() {
                        pet.cancel_motion(&mut host, trace)?;
                    }
                    keyboard = conn.get_keyboard_mapping(first_key, key_count)?.reply()?;
                }
                Event::ClientMessage(e)
                    if e.window == control
                        && e.type_ == atoms._MOCHI_DESKTOP_CONTROL
                        && e.format == 32
                        && e.data.as_data32()[0] == 1 =>
                {
                    quit = true;
                }
                Event::ClientMessage(e)
                    if e.type_ == atoms.WM_PROTOCOLS
                        && e.format == 32
                        && e.data.as_data32()[0] == atoms.WM_DELETE_WINDOW =>
                {
                    remove(&conn, &mut host, &mut pets, e.window)?;
                }
                Event::UnmapNotify(e) => {
                    if let Some(pet) = pets.get_mut(&e.window) {
                        pet.cancel_motion(&mut host, trace)?;
                        if let Some(press) = pet.press.filter(|p| p.dragging) {
                            dispatch(
                                &mut host,
                                &pet.handle,
                                MateEvent::DragEnd {
                                    position: press.local,
                                },
                                trace,
                            )?;
                        }
                        pet.paused = true;
                        if let Some(shake) = &mut pet.shake {
                            shake.release();
                        }
                        if let Some(drag) = &mut pet.drag {
                            drag.release();
                        }
                        pet.press = None;
                    }
                }
                Event::MapNotify(e) => {
                    if let Some(pet) = pets.get_mut(&e.window) {
                        pet.interrupt_idle(&mut host, trace)?;
                        pet.paused = false;
                        pet.repaint = Some(Rectangle {
                            x: 0,
                            y: 0,
                            width: pet.size,
                            height: pet.size,
                        });
                    }
                }
                Event::Error(error) => return Err(format!("X11 error: {error:?}").into()),
                _ => {}
            }
        }
        // Gaze and the primary role's menu share the same 30 Hz pointer sample.
        let sampled_gaze = epoch.elapsed() >= next_gaze_poll;
        if sampled_gaze {
            // Send all opted-in queries before waiting, preserving the server's
            // actual window-local coordinates with one batch of round trips.
            let queries: Vec<_> = pets
                .iter()
                .filter(|(window, pet)| {
                    !pet.paused
                        && ((pet.gaze.is_some() && pet.policy.allows("gaze", "pointer"))
                            || (bubbles.is_some() && Some(**window) == primary))
                })
                .map(|(&window, _)| conn.query_pointer(window).map(|cookie| (window, cookie)))
                .collect::<std::result::Result<_, _>>()?;
            // XWayland retains old coordinates over native Wayland surfaces.
            // A root query with no child means those coordinates are not a live
            // pointer target. Native X11 may legitimately point at its root.
            let root_pointer = (xwayland && !queries.is_empty())
                .then(|| conn.query_pointer(root))
                .transpose()?;
            let desktop_sample = if queries.is_empty() {
                None
            } else {
                desktop_pointer.as_ref().and_then(|p| p.poll())
            };
            pointer_queries += queries.len() as u64;
            pointer_queries += u64::from(root_pointer.is_some());
            let pointer_available = root_pointer
                .map(|query| query.reply())
                .transpose()?
                .is_none_or(|p| p.same_screen && p.child != 0);
            let mut menu_sample = (None, None, true);
            for (window, query) in queries {
                let pointer = query.reply()?;
                let pet = pets.get_mut(&window).expect("queried live window");
                let origin = [
                    i32::from(pointer.root_x) - i32::from(pointer.win_x),
                    i32::from(pointer.root_y) - i32::from(pointer.win_y),
                ];
                let root_point = match desktop_sample {
                    Some(Some(root)) => Some([root.x, root.y]),
                    Some(None) => None,
                    None if pointer_available && pointer.same_screen => {
                        Some([f64::from(pointer.root_x), f64::from(pointer.root_y)])
                    }
                    None => None,
                };
                if Some(window) == primary {
                    let bounds = pet
                        .shape
                        .iter()
                        .fold(None, |bounds, rect| pixels::damage(bounds, Some(*rect)));
                    menu_sample = (
                        bounds.map(|b| {
                            [
                                origin[0] + i32::from(b.x),
                                origin[1] + i32::from(b.y),
                                i32::from(b.width),
                                i32::from(b.height),
                            ]
                        }),
                        root_point,
                        pet.press.is_some() || pet.keys.direction() != [0, 0],
                    );
                }
                let point = match desktop_sample {
                    Some(Some(root)) => {
                        // XWayland's pointer may be stale, but root - local
                        // still gives the queried window's actual WM origin.
                        Some(pet.local_point(Vec2::new(
                            root.x - (f64::from(pointer.root_x) - f64::from(pointer.win_x)),
                            root.y - (f64::from(pointer.root_y) - f64::from(pointer.win_y)),
                        ))?)
                    }
                    Some(None) => None,
                    None if pointer_available && pointer.same_screen => {
                        Some(pet.local(pointer.win_x, pointer.win_y)?)
                    }
                    None => None,
                };
                if let Some(gaze) = &mut pet.gaze {
                    let scene = host.get(&pet.handle)?.runtime().scene()?;
                    gaze.input(&scene, point);
                }
            }
            if let Some(bubbles) = &mut bubbles {
                bubbles.pointer(menu_sample.0, menu_sample.1, menu_sample.2);
            }
            next_gaze_poll = epoch.elapsed() + Duration::from_secs_f64(1.0 / 30.0);
        }
        let advanced_at = epoch.elapsed();
        let steps = clock.advance(advanced_at)?;
        for _ in 0..steps {
            for (window, pet) in &mut pets {
                if !pet.paused && (pet.drag.is_some() || !pet.press.is_some_and(|p| p.dragging)) {
                    if let Some(drag) = &mut pet.drag {
                        drag.advance(FIXED_STEP.as_secs_f64());
                    }
                    if let Some(gaze) = &mut pet.gaze {
                        gaze.advance(FIXED_STEP.as_secs_f64());
                    }
                    pet.update_work(&mut host, trace)?;
                    if pet.idle.as_mut().is_some_and(|idle| {
                        idle.finish_action(
                            &host
                                .get(&pet.handle)
                                .expect("live pet")
                                .runtime()
                                .state()
                                .current_animation,
                        )
                    }) && trace
                    {
                        println!("IdleActionStop window={window}");
                    }
                    if !pet.idle_eligible(&host)? {
                        pet.interrupt_idle(&mut host, trace)?;
                    } else if pet
                        .idle
                        .as_mut()
                        .is_some_and(|idle| idle.ready(FIXED_STEP.as_secs_f64()))
                    {
                        if let Some(alias) = pet.idle.as_mut().and_then(Idle::begin_action) {
                            pet.action(&mut host, &alias, trace)?;
                            if host.get(&pet.handle)?.runtime().state().current_animation != alias {
                                pet.idle = None;
                            } else if trace {
                                println!("IdleActionStart window={window} action={alias}");
                            }
                        } else {
                            // The user/WM may have moved the window since the last
                            // gesture. Sample actual placement once before a stroll.
                            pet.sync_position(&conn, *window, root)?;
                            if let Some((direction, distance)) = pet
                                .idle
                                .as_mut()
                                .and_then(|idle| idle.begin(&pet.motion, scale))
                            {
                                pet.walk(&mut host, true, trace)?;
                                if host.get(&pet.handle)?.runtime().state().current_animation
                                    != "walk"
                                {
                                    // Packages without the desktop walk binding must
                                    // not slide across the desktop on their idle clip.
                                    pet.idle = None;
                                    pet.walking = false;
                                } else if trace {
                                    println!("IdleWalkStart window={window} origin={:?} direction={direction} distance={distance:.3}", pet.motion.origin());
                                }
                            }
                        }
                    }
                    let auto = pet
                        .idle
                        .as_mut()
                        .and_then(|idle| idle.step(FIXED_STEP.as_secs_f64(), scale));
                    let direction = auto
                        .as_ref()
                        .map_or_else(|| pet.keys.direction(), |step| step.direction);
                    let distance = auto
                        .as_ref()
                        .map_or(SPEED * scale * FIXED_STEP.as_secs_f64(), |step| {
                            step.distance
                        });
                    let moving = pet.motion.advance(direction, distance);
                    let finished = auto.as_ref().is_some_and(|step| step.finished || !moving);
                    if finished {
                        pet.idle.as_mut().expect("automatic step").interrupt();
                        if trace {
                            println!(
                                "IdleWalkStop window={window} origin={:?}",
                                pet.motion.origin()
                            );
                        }
                    }
                    pet.walk(&mut host, moving && !finished, trace)?;
                    dispatch(
                        &mut host,
                        &pet.handle,
                        MateEvent::Tick { delta: FIXED_STEP },
                        trace,
                    )?;
                }
            }
        }
        #[cfg(feature = "gpu-test-hooks")]
        if test_loss_after.is_some_and(|delay| epoch.elapsed() >= delay) {
            if let Some(pet) = pets.values().next() {
                pet.renderer.test_lose_spine_device_on_readback()?;
                eprintln!("Test: armed shared wgpu device loss at readback");
            }
            test_loss_after = None;
        }
        let render_started = Instant::now();
        // Interleave GPU and X11 work: submitting every pet first caused
        // sustained upload stalls on the measured XWayland desktop.
        for (window, pet) in &mut pets {
            let position = pet.motion.origin();
            if position != pet.window_position {
                conn.configure_window(
                    *window,
                    &ConfigureWindowAux::new().x(position[0]).y(position[1]),
                )?;
                pet.window_position = position;
            }
            pet.prepare(&host)?;
            pet.finish_readback(&host)?;
            pet.draw(&conn, *window, &host)?;
            if trace && sampled_gaze {
                if let Some(jump) = &pet.jump {
                    println!("JumpPose window={window} {}", jump.trace());
                }
                if let Some(shake) = &pet.shake {
                    println!("ShakePose window={window} {}", shake.trace());
                }
                if let Some(drag) = &pet.drag {
                    println!("DragPose window={window} {}", drag.trace());
                }
            }
            if trace && sampled_gaze && pet.gaze.is_some() {
                if let Some(SceneContent::Skeletal {
                    offsets, overlay, ..
                }) = pet
                    .last_scene
                    .as_ref()
                    .and_then(|s| s.nodes.first())
                    .map(|n| &n.content)
                {
                    let mut head = offsets.first().map_or(0.0, |o| o.rotation_degrees);
                    let mut count = offsets.len();
                    let mut eye = offsets
                        .last()
                        .map_or(Vec2::default(), |o| o.world_translation);
                    if let Some(gaze) = &overlay.gaze {
                        head = gaze.rotations.first().map_or(0.0, |r| r.degrees);
                        eye = gaze.eyes.last().map_or(Vec2::default(), |e| e.translation);
                        count += gaze.rotations.len() + gaze.eyes.len();
                    }
                    println!("Gaze window={window} head={head:.4} eye_x={:.4} eye_y={:.4} offsets={} mirrored={}",eye.x,eye.y,count,pet.motion.mirrored);
                }
            }
        }
        conn.flush()?;
        if let Some(writer) = &mut metrics {
            writeln!(
                writer,
                "{:.6},{:.6},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{},{},{},{},{},{}",
                epoch.elapsed().as_secs_f64() * 1000.0,
                render_started.elapsed().as_secs_f64() * 1000.0,
                pets.values().map(|p| p.drawn).sum::<u64>(),
                pets.len(),
                pets.values()
                    .map(|p| p.timing[0].as_secs_f64() * 1000.)
                    .sum::<f64>(),
                pets.values()
                    .map(|p| p.timing[1].as_secs_f64() * 1000.)
                    .sum::<f64>(),
                pets.values()
                    .map(|p| p.timing[2].as_secs_f64() * 1000.)
                    .sum::<f64>(),
                pets.values()
                    .map(|p| p.timing[3].as_secs_f64() * 1000.)
                    .sum::<f64>(),
                pets.values()
                    .map(|p| p.transfer.wait.as_secs_f64() * 1000.)
                    .sum::<f64>(),
                pets.values()
                    .map(|p| p.transfer.copy.as_secs_f64() * 1000.)
                    .sum::<f64>(),
                pets.values().map(|p| p.transfer.bytes).sum::<u64>(),
                pets.values().map(|p| p.upload_bytes).sum::<u64>(),
                event_count,
                motion_count,
                pointer_queries,
                pets.values().map(|p| p.shape_updates).sum::<u64>(),
                pets.values().map(|p| p.shape.len()).sum::<usize>(),
                pets.values().map(|p| p.copy_pixels).sum::<u64>(),
            )?;
        }
        if quit || pets.is_empty() {
            break;
        }
        queued = conn.poll_for_event()?;
        if queued.is_some() {
            continue;
        }
        let delay = clock
            .next_in()
            .saturating_sub(epoch.elapsed().saturating_sub(advanced_at));
        let timeout = rustix::event::Timespec::try_from(delay)?;
        let wake = if let Some(stream) = bubbles.as_ref().and_then(crate::bubbles::Bubbles::stream)
        {
            let mut fds = [
                rustix::event::PollFd::new(conn.stream(), rustix::event::PollFlags::IN),
                rustix::event::PollFd::new(stream, rustix::event::PollFlags::IN),
            ];
            rustix::event::poll(&mut fds, Some(&timeout))
        } else {
            let mut fds = [rustix::event::PollFd::new(
                conn.stream(),
                rustix::event::PollFlags::IN,
            )];
            rustix::event::poll(&mut fds, Some(&timeout))
        };
        match wake {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
    for window in pets.keys().copied().collect::<Vec<_>>() {
        remove(&conn, &mut host, &mut pets, window)?;
    }
    host.clear();
    if let Some(writer) = &mut metrics {
        writer.flush()?;
    }
    conn.destroy_window(control)?.check()?;
    if cursor != 0 {
        conn.free_cursor(cursor)?.check()?;
    }
    conn.free_colormap(colormap)?.check()?;
    conn.flush()?;
    println!(
        "Closed: live={}, pending={}",
        host.len(),
        host.broker().pending_count()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_uses_root_delta_without_feedback_from_the_moving_window() {
        let mut press = Press {
            root: [100, 200],
            origin: [60, 80],
            local: Vec2::default(),
            dragging: false,
            threshold: 4,
        };
        assert_eq!(press.moved([102, 202]), ([62, 82], false));
        assert!(!press.dragging);
        assert_eq!(press.moved([104, 200]), ([64, 80], true));
        assert_eq!(press.moved([104, 200]), ([64, 80], false));
        assert_eq!(press.moved([80, 220]), ([40, 100], false));
        assert!(press.dragging);
    }
}
