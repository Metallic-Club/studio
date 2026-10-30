#![cfg(feature = "tracy")]

use metallic_profiling::{FrameProfiler, Subsystem, SystemClock};

#[test]
fn uninitialized_tracy_scopes_and_frame_marks_do_not_start_a_client() {
    assert!(!metallic_profiling::backend::tracy_client::Client::is_running());
    let mut profiler = FrameProfiler::<SystemClock, 4>::default();
    let frame = profiler.begin_frame();
    {
        metallic_profiling::backend_scope!("inactive.external");
        metallic_profiling::cpu_scope!(frame, Subsystem::Scripts, "inactive.cpu");
    }
    assert_eq!(frame.finish().scope_count(Subsystem::Scripts), 1);
    assert!(!metallic_profiling::backend::tracy_client::Client::is_running());
}
