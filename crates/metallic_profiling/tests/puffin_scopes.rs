#![cfg(feature = "puffin")]

use metallic_profiling::backend::puffin;
use std::sync::{Arc, Mutex};

#[test]
fn backend_guards_export_nested_scopes_until_the_callers_block_ends() {
    let frames = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&frames);
    let sink = puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame| {
        captured.lock().unwrap().push(frame);
    }));
    metallic_profiling::set_puffin_enabled(true);
    {
        metallic_profiling::backend_scope!("fixture.outer");
        {
            metallic_profiling::backend_scope!("fixture.inner");
            std::hint::black_box(42);
        }
    }
    puffin::GlobalProfiler::lock().new_frame();
    metallic_profiling::set_puffin_enabled(false);
    puffin::GlobalProfiler::lock().remove_sink(sink);

    assert!(
        frames.lock().unwrap().iter().any(|frame| {
            frame
                .unpacked()
                .ok()
                .expect("captured frame must be unpacked")
                .thread_streams
                .values()
                .any(|stream| stream.num_scopes == 2 && stream.depth == 2)
        }),
        "exported outer guard must remain live while the inner scope runs"
    );
}
