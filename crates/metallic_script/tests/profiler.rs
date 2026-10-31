//! #23: the profiler charges each script its own run time.
mod common;

use common::Rig;
use metallic_profiler::Profiler;

#[test]
fn per_script_time_attributed() {
    let mut rig = Rig::new();
    // One script is busy for about 3 ms on every Heartbeat, another for about 1 ms in a task
    // it spawned (resumed by the scheduler), and a third one never runs again after starting.
    let busy = rig.server_script(
        "Busy",
        r#"
game:GetService('RunService').Heartbeat:Connect(function()
    local t = os.clock()
    while os.clock() - t < 0.003 do end
end)
"#,
    );
    let light = rig.server_script(
        "Light",
        r#"
task.spawn(function()
    while true do
        task.wait()
        local t = os.clock()
        while os.clock() - t < 0.001 do end
    end
end)
"#,
    );
    let idle = rig.server_script("Idle", "local x = 1");
    rig.host.start().expect("scripts start");
    rig.host.step(1.0 / 60.0, 1.0 / 60.0);

    Profiler::set_enabled(true);
    Profiler::begin_frame();
    rig.host.step(2.0 / 60.0, 1.0 / 60.0);
    Profiler::end_frame();
    Profiler::set_enabled(false);

    let frame = Profiler::latest().expect("a frame");
    let time_of = |id: metallic_core::InstanceId| {
        frame
            .scripts()
            .iter()
            .find(|s| s.script == id.to_bits())
            .map(|s| s.ns)
    };
    let busy_ns = time_of(busy).expect("the busy script ran");
    let light_ns = time_of(light).expect("the light script ran");
    assert!(busy_ns >= 3_000_000, "busy {busy_ns}");
    assert!(light_ns >= 1_000_000, "light {light_ns}");
    assert!(busy_ns > light_ns, "busy {busy_ns} light {light_ns}");
    // Both together are not more than the frame's script time.
    assert!(busy_ns + light_ns <= frame.script_ns());
    assert_eq!(
        time_of(idle),
        None,
        "a script that did not run is not listed"
    );
    assert!(
        rig.lines().iter().all(|l| !l.starts_with("error")),
        "{:?}",
        rig.lines()
    );
}
