#[cfg(target_os = "linux")]
mod action_preview;
#[cfg(target_os = "linux")]
mod actions;
#[cfg(target_os = "linux")]
mod bubbles;
#[cfg(target_os = "linux")]
mod cursor;
#[cfg(target_os = "linux")]
mod desktop;
#[cfg(target_os = "linux")]
mod drag;
#[cfg(target_os = "linux")]
mod gaze;
#[cfg(target_os = "linux")]
mod idle;
#[cfg(target_os = "linux")]
mod input_region;
#[cfg(target_os = "linux")]
mod install_paths;
#[cfg(target_os = "linux")]
mod jump;
#[cfg(target_os = "linux")]
mod motion;
#[cfg(target_os = "linux")]
mod pixels;
#[cfg(target_os = "linux")]
mod pointer;
#[cfg(target_os = "linux")]
mod pull;
#[cfg(target_os = "linux")]
mod shake;
#[cfg(target_os = "linux")]
mod upload;
#[cfg(target_os = "linux")]
mod work;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--build-info") {
        println!(
            "{}",
            serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "spine": cfg!(feature = "spine")})
        );
        return;
    }
    #[cfg(target_os = "linux")]
    if let Err(error) = if std::env::args().nth(1).as_deref() == Some("--preview-action") {
        action_preview::run()
    } else {
        desktop::run()
    } {
        eprintln!("Mochi desktop: {error}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("This desktop prototype requires Linux with X11/XWayland.");
        std::process::exit(1);
    }
}
