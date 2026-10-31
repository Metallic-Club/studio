//! Diagnostics for real-adapter integration tests; no shared device or serialization.
#![allow(dead_code)]

use metallic_core::World;
use metallic_render::{Camera, RenderContext, RenderError, RenderSettings, Renderer};
use std::io::Write;
use std::ops::{Deref, DerefMut};

fn phase(name: &str, detail: &str) {
    let thread = std::thread::current();
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(
        stderr,
        "[metallic-gpu] pid={} test={:?} phase={name} {detail}",
        std::process::id(),
        thread.name().unwrap_or("unnamed")
    );
    let _ = stderr.flush();
}

/// Test-only selection. Dedicated backend tests bypass this override.
pub fn parse_backend(value: Option<&str>) -> Result<Option<wgpu::Backends>, String> {
    match value {
        None | Some("default") => Ok(None),
        Some("dx12") => Ok(Some(wgpu::Backends::DX12)),
        Some("vulkan") => Ok(Some(wgpu::Backends::VULKAN)),
        Some("gl") => Ok(Some(wgpu::Backends::GL)),
        Some(value) => Err(format!("invalid METALLIC_RENDER_TEST_BACKEND {value:?}")),
    }
}

/// Adapter absence is the only setup result that may skip a GPU test.
pub fn context_result<T>(result: Result<T, RenderError>) -> Option<T> {
    match result {
        Err(RenderError::NoAdapter(reason)) => {
            phase("context.unavailable", &format!("reason={reason:?}"));
            None
        }
        other => Some(require_result("context", other)),
    }
}

/// Rendering/readback errors always fail, even an unexpected NoAdapter result.
pub fn require_result<T>(operation: &str, result: Result<T, RenderError>) -> T {
    match result {
        Ok(value) => {
            phase(&format!("{operation}.complete"), "");
            value
        }
        Err(error) => {
            phase(&format!("{operation}.error"), &format!("error={error:?}"));
            panic!("headless GPU {operation} failed: {error}");
        }
    }
}

/// Owns exactly the context that this test requested and traces its handle drop.
pub struct TestContext(Option<RenderContext>);

impl TestContext {
    pub fn new_headless() -> Option<Self> {
        let value = match std::env::var("METALLIC_RENDER_TEST_BACKEND") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(error) => panic!("invalid METALLIC_RENDER_TEST_BACKEND: {error}"),
        };
        let backend = parse_backend(value.as_deref()).unwrap_or_else(|error| panic!("{error}"));
        Self::create(backend)
    }

    pub fn new_headless_with_backends(backends: wgpu::Backends) -> Option<Self> {
        Self::create(Some(backends))
    }

    fn create(backends: Option<wgpu::Backends>) -> Option<Self> {
        phase("context.begin", &format!("requested={backends:?}"));
        let result = match backends {
            Some(backends) => RenderContext::new_headless_with_backends(backends),
            None => RenderContext::new_headless(),
        };
        let context = context_result(result)?;
        let adapter = context.adapter.as_ref().expect("standalone adapter");
        let info = adapter.get_info();
        phase(
            "adapter.ready",
            &format!(
                "backend={:?} name={:?} vendor={:#x} device={:#x} type={:?} driver={:?} driver_info={:?}",
                info.backend,
                info.name,
                info.vendor,
                info.device,
                info.device_type,
                info.driver,
                info.driver_info
            ),
        );
        Some(Self(Some(context)))
    }
}

impl Deref for TestContext {
    type Target = RenderContext;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("live test context")
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        phase("context.handle-drop.begin", "");
        drop(self.0.take());
        phase("context.handle-drop.complete", "");
    }
}

/// Delegates to the real renderer, retaining all assertions and per-test ownership.
pub struct TestRenderer(Option<Renderer>);

impl TestRenderer {
    pub fn with_settings(
        context: &RenderContext,
        format: wgpu::TextureFormat,
        settings: RenderSettings,
    ) -> Self {
        phase(
            "renderer.begin",
            &format!("settings={settings:?} format={format:?}"),
        );
        let renderer = Renderer::with_settings(context, format, settings);
        phase("renderer.complete", "");
        Self(Some(renderer))
    }

    pub fn sync(&mut self, world: &World) {
        phase("sync.begin", "");
        self.deref_mut().sync(world);
        phase("sync.complete", "");
    }

    pub fn render(&mut self, view: &wgpu::TextureView, camera: &Camera, size: (u32, u32)) {
        phase("render.begin", &format!("size={size:?}"));
        self.deref_mut().render(view, camera, size);
        phase("render.complete", "");
    }

    pub fn capture(&mut self, camera: &Camera, size: (u32, u32)) -> Vec<u8> {
        phase("capture.begin", &format!("size={size:?}"));
        let result = self.deref_mut().capture(camera, size);
        require_result("capture", result)
    }
}

impl Deref for TestRenderer {
    type Target = Renderer;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("live test renderer")
    }
}

impl DerefMut for TestRenderer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("live test renderer")
    }
}

impl Drop for TestRenderer {
    fn drop(&mut self) {
        phase("renderer.handle-drop.begin", "");
        drop(self.0.take());
        phase("renderer.handle-drop.complete", "");
    }
}
