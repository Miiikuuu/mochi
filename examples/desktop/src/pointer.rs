//! Optional GNOME pointer bridge. All D-Bus and monitor discovery runs outside
//! the render thread; one cached sample serves every pet.
use mochi_core::Vec2;
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection as _,
    protocol::{randr::ConnectionExt as _, xproto::ConnectionExt as _},
    rust_connection::RustConnection,
};

const MAX_AGE: Duration = Duration::from_millis(150);
const RETRY: Duration = Duration::from_secs(2);
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone)]
struct Monitor {
    connector: String,
    origin: [f64; 2],
    size: [f64; 2],
}
impl Monitor {
    fn point(&self, x: f64, y: f64) -> Option<Vec2> {
        if !x.is_finite() || !y.is_finite() || !(0.0..1.0).contains(&x) || !(0.0..1.0).contains(&y)
        {
            return None;
        }
        Some(Vec2::new(
            self.origin[0] + x * self.size[0],
            self.origin[1] + y * self.size[1],
        ))
    }
}

struct Sample {
    at: Instant,
    point: Option<Vec2>,
}
impl Sample {
    fn fresh_point(&self, now: Instant) -> Option<Vec2> {
        (now.saturating_duration_since(self.at) <= MAX_AGE)
            .then_some(self.point)
            .flatten()
    }
}

pub struct DesktopPointer {
    requests: mpsc::SyncSender<()>,
    sample: Arc<Mutex<Option<Sample>>>,
}
impl DesktopPointer {
    pub fn start() -> std::io::Result<Self> {
        let (requests, receive) = mpsc::sync_channel(1);
        let sample = Arc::new(Mutex::new(None));
        let worker_sample = Arc::clone(&sample);
        thread::Builder::new()
            .name("mochi-pointer".into())
            .spawn(move || {
                if let Err(error) = worker(receive, Arc::clone(&worker_sample)) {
                    if let Ok(mut target) = worker_sample.lock() {
                        *target = None;
                    }
                    eprintln!("Desktop gaze: GNOME pointer bridge unavailable: {error}");
                }
            })?;
        Ok(Self { requests, sample })
    }

    // None: bridge unavailable, use normal X11 fallback. Some(None): bridge
    // reports unavailable input (e.g. lock screen), or its sample expired.
    pub fn poll(&self) -> Option<Option<Vec2>> {
        let _ = self.requests.try_send(());
        self.sample
            .lock()
            .ok()?
            .as_ref()
            .map(|s| s.fresh_point(Instant::now()))
    }
}

fn monitors(conn: &RustConnection, root: u32) -> Result<Vec<Monitor>> {
    conn.randr_get_monitors(root, true)?
        .reply()?
        .monitors
        .into_iter()
        .take(32)
        .map(|m| {
            Ok(Monitor {
                connector: String::from_utf8(conn.get_atom_name(m.name)?.reply()?.name)?,
                origin: [f64::from(m.x), f64::from(m.y)],
                size: [f64::from(m.width), f64::from(m.height)],
            })
        })
        .collect()
}

fn worker(receive: mpsc::Receiver<()>, sample: Arc<Mutex<Option<Sample>>>) -> Result<()> {
    let bus = dbus::blocking::Connection::new_session()?;
    let (x11, screen) = x11rb::connect(None)?;
    let root = x11.setup().roots[screen].root;
    let proxy = bus.with_proxy(
        "org.gnome.Shell",
        "/org/mochi/Desktop/Pointer",
        Duration::from_millis(100),
    );
    let mut mapping = monitors(&x11, root)?;
    let mut refresh_at = Instant::now() + RETRY;
    let mut retry_at = Instant::now();
    let mut connected = false;
    while receive.recv().is_ok() {
        let now = Instant::now();
        if now < retry_at {
            continue;
        }
        if now >= refresh_at {
            mapping = monitors(&x11, root)?;
            refresh_at = now + RETRY;
        }
        let connectors: Vec<_> = mapping.iter().map(|m| m.connector.as_str()).collect();
        let reply: std::result::Result<(bool, String, f64, f64), _> =
            proxy.method_call("org.mochi.Desktop.Pointer", "Read", (connectors,));
        let value = match reply {
            Ok((available, connector, x, y)) => {
                if !connected {
                    println!(
                        "Desktop gaze: GNOME pointer bridge connected (shared, at most 30 Hz)"
                    );
                }
                connected = true;
                Some(Sample {
                    // Use request time: a slow response must not look fresh.
                    at: now,
                    point: available
                        .then(|| mapping.iter().find(|m| m.connector == connector))
                        .flatten()
                        .and_then(|m| m.point(x, y)),
                })
            }
            Err(_) => {
                if connected {
                    eprintln!(
                        "Desktop gaze: GNOME pointer bridge disconnected; using X11 fallback"
                    );
                }
                connected = false;
                retry_at = now + RETRY;
                None
            }
        };
        if let Ok(mut target) = sample.lock() {
            *target = value;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_pointer_across_mixed_dpi_monitors_and_negative_origins() {
        // The same logical displacement has different physical sizes per monitor.
        let a = Monitor {
            connector: "DP-3".into(),
            origin: [0.0, 0.0],
            size: [5120.0, 2880.0],
        };
        let b = Monitor {
            connector: "eDP-1".into(),
            origin: [5120.0, 0.0],
            size: [3840.0, 2160.0],
        };
        let c = Monitor {
            connector: "left".into(),
            origin: [-1920.0, -1080.0],
            size: [1920.0, 1080.0],
        };
        assert_eq!(
            a.point(640.0 / 2560.0, 360.0 / 1440.0),
            Some(Vec2::new(1280.0, 720.0))
        );
        assert_eq!(
            b.point(640.0 / 2560.0, 360.0 / 1440.0),
            Some(Vec2::new(6080.0, 540.0))
        );
        assert_eq!(c.point(0.5, 0.5), Some(Vec2::new(-960.0, -540.0)));
        for (x, y) in [
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
            (-0.01, 0.5),
            (1.0, 0.5),
        ] {
            assert_eq!(a.point(x, y), None);
        }
    }

    #[test]
    fn expired_or_locked_samples_cannot_hold_the_last_gaze() {
        let at = Instant::now();
        let sample = Sample {
            at,
            point: Some(Vec2::new(1000.0, 700.0)),
        };
        assert_eq!(
            sample.fresh_point(at + Duration::from_millis(30)),
            sample.point
        );
        assert_eq!(sample.fresh_point(at + Duration::from_millis(151)), None);
        let locked = Sample { at, point: None };
        assert_eq!(locked.fresh_point(at), None);
    }
}
