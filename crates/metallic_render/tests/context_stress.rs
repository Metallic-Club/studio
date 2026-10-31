//! #276: setting up devices on many threads at once (what parallel GPU tests do).
//!
//! The quick tests run always. The stress run is ignored and started by hand with
//! `cargo test -p metallic_render --test context_stress -- --ignored --nocapture`;
//! `STRESS_BACKENDS` picks the backends (`all`, `primary`, `gl`, `vulkan`, `dx12`),
//! `STRESS_MODE` what each round makes (`instance`, `adapter`, `create-locked`, `drop-locked`,
//! or a whole device), `STRESS_THREADS` and `STRESS_ROUNDS` how much. Raw wgpu instances with
//! GL on 8 threads crashed 7 or 8 runs in 10 on Windows/AMD (`0xc0000005`); serialising only
//! their creation made it 0 in 10, serialising only their drop did not help.

use metallic_render::RenderContext;

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `None`: exactly what `RenderContext::new_headless` does.
fn backends() -> Option<wgpu::Backends> {
    match std::env::var("STRESS_BACKENDS").as_deref() {
        Ok("all") => Some(wgpu::Backends::all()),
        Ok("primary") => Some(wgpu::Backends::PRIMARY),
        Ok("gl") => Some(wgpu::Backends::GL),
        Ok("vulkan") => Some(wgpu::Backends::VULKAN),
        Ok("dx12") => Some(wgpu::Backends::DX12),
        _ => None,
    }
}

#[test]
#[ignore = "stress run"]
fn many_threads_create_and_drop_devices() {
    let threads: usize = std::env::var("STRESS_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(8);
    let rounds: usize = std::env::var("STRESS_ROUNDS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(20);
    let backends = backends();
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            std::thread::spawn(move || {
                let mode = std::env::var("STRESS_MODE").unwrap_or_default();
                for _ in 0..rounds {
                    // `instance`: only create and drop an instance; `adapter`: also pick an
                    // adapter; anything else: a whole device with a little work.
                    // `create-locked` / `drop-locked`: only one side of an instance's life
                    // runs under a process-wide lock, to see which side races.
                    if mode == "create-locked" || mode == "drop-locked" {
                        let make = || {
                            wgpu::Instance::new(wgpu::InstanceDescriptor {
                                backends: backends.unwrap_or(wgpu::Backends::all()),
                                ..wgpu::InstanceDescriptor::new_without_display_handle()
                            })
                        };
                        let instance = if mode == "create-locked" {
                            let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
                            make()
                        } else {
                            make()
                        };
                        if mode == "drop-locked" {
                            let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
                            drop(instance);
                        } else {
                            drop(instance);
                        }
                        continue;
                    }
                    if mode == "instance" || mode == "adapter" {
                        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                            backends: backends.unwrap_or(wgpu::Backends::all()),
                            ..wgpu::InstanceDescriptor::new_without_display_handle()
                        });
                        if mode == "adapter" {
                            let _ = pollster::block_on(
                                instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
                            );
                        }
                        continue;
                    }
                    let ctx = match backends {
                        Some(b) => RenderContext::new_headless_with_backends(b),
                        None => RenderContext::new_headless(),
                    }
                    .expect("a device");
                    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: 256,
                        usage: wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    ctx.queue.write_buffer(&buffer, 0, &[1; 256]);
                    ctx.queue.submit([]);
                    let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("a stress thread panicked");
    }
}

/// OpenGL devices set up on 8 threads at once through `RenderContext` (which serialises the
/// setup). Before the fix this crashed the test process now and then on Windows. Skipped where
/// there is no OpenGL adapter.
#[test]
fn gl_devices_set_up_on_many_threads_at_once() {
    if RenderContext::new_headless_with_backends(wgpu::Backends::GL).is_err() {
        eprintln!("no OpenGL adapter; skipping");
        return;
    }
    let threads: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..6 {
                    let ctx = RenderContext::new_headless_with_backends(wgpu::Backends::GL)
                        .expect("an OpenGL device");
                    ctx.queue.submit([]);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().expect("a setup thread panicked");
    }
}

/// A headless device is Vulkan, DX12 or Metal whenever one of them has an adapter, so
/// OpenGL is never set up next to it.
#[test]
fn headless_prefers_a_primary_backend() {
    let Ok(primary) = RenderContext::new_headless_with_backends(wgpu::Backends::PRIMARY) else {
        eprintln!("no Vulkan, DX12 or Metal adapter; skipping");
        return;
    };
    let expected = primary.adapter.as_ref().map(|a| a.get_info().backend);
    let ctx = RenderContext::new_headless().expect("a device");
    let backend = ctx.adapter.as_ref().map(|a| a.get_info().backend);
    assert_eq!(backend, expected);
    assert_ne!(backend, Some(wgpu::Backend::Gl));
}
