//! Opens a window and logs its input events.
//!
//! ```text
//! cargo run -p metallic_platform --example window
//! ```
//!
//! Keys, buttons, wheel, focus, touch and gamepad events are logged at debug level, cursor and raw mouse motion
//! at trace level (`METALLIC_LOG=trace` shows them). F11 toggles fullscreen, G toggles a locked, hidden cursor,
//! and Esc or closing the window quits.

use std::io::Write;

use log::{Level, LevelFilter, Log, Metadata, Record};
use metallic_platform::{
    AppHandler, AppId, ButtonState, CursorGrab, DeviceEvent, EventLoopContext, GamepadEvent, KeyCode, LoopMode,
    Platform, PlatformError, Window, WindowConfig, WindowEvent, WindowId, raw_window_handle::HasWindowHandle,
};

fn main() {
    let level = std::env::var("METALLIC_LOG")
        .ok()
        .and_then(|level| level.parse().ok())
        .unwrap_or(LevelFilter::Debug);
    log::set_max_level(level);
    if log::set_logger(&StderrLogger).is_err() {
        eprintln!("a logger was already installed");
    }
    if let Err(err) = run() {
        log::error!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), PlatformError> {
    let platform = Platform::new()?;
    let paths = platform.user_paths(&AppId::parse("dev.metallic.WindowExample")?)?;
    log::info!(
        "{} backend; config {}, data {}, cache {}",
        platform.name(),
        paths.config_dir().display(),
        paths.data_dir().display(),
        paths.cache_dir().display()
    );
    let mut app = Example::default();
    platform.run(&mut app)?;
    log::info!("event loop stopped cleanly");
    Ok(())
}

#[derive(Default)]
struct Example {
    window: Option<Window>,
    fullscreen: bool,
    grabbed: bool,
}

impl AppHandler for Example {
    fn resumed(&mut self, cx: &mut dyn EventLoopContext) {
        log::info!("resumed");
        // Nothing animates, so sleep until input arrives instead of spinning.
        cx.set_loop_mode(LoopMode::Wait);
        if self.window.is_some() {
            return;
        }
        match cx.create_window(&WindowConfig::new("Metallic: window example").with_size(960, 540)) {
            Ok(window) => {
                let handle = window.window_handle().map(|handle| format!("{:?}", handle.as_raw()));
                log::info!(
                    "opened {:?}: {:?} physical pixels, scale {}, handle {}",
                    window.id(),
                    window.inner_size(),
                    window.scale_factor(),
                    handle.unwrap_or_else(|err| err.to_string())
                );
                self.window = Some(window);
            }
            Err(err) => {
                log::error!("{err}");
                cx.exit();
            }
        }
    }

    fn suspended(&mut self, _cx: &mut dyn EventLoopContext) {
        log::info!("suspended");
    }

    fn window_event(&mut self, cx: &mut dyn EventLoopContext, window: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::CursorMoved(_) | WindowEvent::RedrawRequested => log::trace!("{window:?}: {event:?}"),
            _ => log::debug!("{window:?}: {event:?}"),
        }
        match event {
            WindowEvent::CloseRequested => {
                log::info!("close requested");
                cx.exit();
            }
            WindowEvent::Key(key) if key.state == ButtonState::Pressed && !key.repeat => match key.key_code {
                KeyCode::Escape => {
                    log::info!("Esc pressed");
                    cx.exit();
                }
                KeyCode::F11 => self.toggle_fullscreen(),
                KeyCode::KeyG => self.toggle_grab(),
                _ => {}
            },
            _ => {}
        }
    }

    fn device_event(&mut self, _cx: &mut dyn EventLoopContext, event: DeviceEvent) {
        match event {
            DeviceEvent::MouseMotion { .. } => log::trace!("device: {event:?}"),
            _ => log::debug!("device: {event:?}"),
        }
    }

    fn gamepad_event(&mut self, _cx: &mut dyn EventLoopContext, event: GamepadEvent) {
        log::debug!("gamepad: {event:?}");
    }

    fn exiting(&mut self) {
        log::info!("exiting");
    }
}

impl Example {
    fn toggle_fullscreen(&mut self) {
        let Some(window) = &self.window else { return };
        self.fullscreen = !self.fullscreen;
        match window.set_fullscreen(self.fullscreen) {
            Ok(()) => log::info!("fullscreen {}", self.fullscreen),
            Err(err) => log::warn!("{err}"),
        }
    }

    fn toggle_grab(&mut self) {
        let Some(window) = &self.window else { return };
        self.grabbed = !self.grabbed;
        let wanted = if self.grabbed {
            CursorGrab::Lock
        } else {
            CursorGrab::None
        };
        match window.set_cursor_grab(wanted) {
            Ok(applied) => log::info!("cursor grab {wanted:?} (applied {applied:?})"),
            Err(err) => log::warn!("{err}"),
        }
        window.set_cursor_visible(!self.grabbed);
    }
}

/// Prints log records to stderr; the example needs no logging crate.
struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            let tag = match record.level() {
                Level::Error => "ERROR",
                Level::Warn => "WARN ",
                Level::Info => "INFO ",
                Level::Debug => "DEBUG",
                Level::Trace => "TRACE",
            };
            let _ = writeln!(std::io::stderr(), "{tag} {}", record.args());
        }
    }

    fn flush(&self) {}
}
