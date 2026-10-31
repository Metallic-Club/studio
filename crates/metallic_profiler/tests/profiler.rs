//! The profiler's recording rules. Each test runs on its own thread, so each has its own
//! profiler.
use metallic_profiler::{
    GpuPass, HISTORY, MAX_SCOPES, MAX_SCRIPTS, Profiler, RingBuffer, Subsystem,
};
use std::time::{Duration, Instant};

/// Busy-waits (sleeping is too coarse on Windows).
fn spin(d: Duration) {
    let until = Instant::now() + d;
    while Instant::now() < until {
        std::hint::spin_loop();
    }
}

#[test]
fn profiler_nests_scopes_and_sums_children() {
    Profiler::set_enabled(true);
    Profiler::begin_frame();
    {
        let _frame = Profiler::scope(Subsystem::Other, "tick");
        {
            let _physics = Profiler::scope(Subsystem::Physics, "physics");
            spin(Duration::from_millis(2));
        }
        {
            let _scripts = Profiler::scope(Subsystem::Script, "scripts");
            spin(Duration::from_millis(1));
            let _inner = Profiler::scope(Subsystem::Physics, "physics.sync");
            spin(Duration::from_millis(1));
        }
        spin(Duration::from_millis(1));
    }
    Profiler::end_frame();
    let frame = Profiler::latest().expect("a frame");
    let names: Vec<_> = frame.scopes().iter().map(|s| (s.name, s.depth)).collect();
    assert_eq!(
        names,
        [
            ("tick", 0),
            ("physics", 1),
            ("scripts", 1),
            ("physics.sync", 2)
        ]
    );
    let s = frame.scopes();
    assert_eq!(s[0].children_ns, s[1].inclusive_ns + s[2].inclusive_ns);
    assert_eq!(s[2].children_ns, s[3].inclusive_ns);
    assert!(s[0].exclusive_ns() >= 1_000_000, "{:?}", s[0]);
    assert!(s[1].start_ns < s[2].start_ns && s[2].start_ns < s[3].start_ns);
    // Exclusive time per subsystem: the nested physics scope is not counted in the scripts.
    assert_eq!(
        frame.subsystem_ns(Subsystem::Script),
        u64::from(s[2].exclusive_ns())
    );
    assert_eq!(
        frame.subsystem_ns(Subsystem::Physics),
        u64::from(s[1].exclusive_ns() + s[3].exclusive_ns())
    );
    let total: u64 = frame.subsystem_totals().iter().sum();
    assert_eq!(total, u64::from(s[0].inclusive_ns));
    assert!(frame.frame_ns() >= u64::from(s[0].inclusive_ns));
    assert_eq!(
        frame.unscoped_ns(),
        frame.frame_ns() - u64::from(s[0].inclusive_ns)
    );
}

#[test]
fn disabled_profiler_records_nothing() {
    Profiler::begin_frame();
    {
        let _s = Profiler::scope(Subsystem::Physics, "physics");
        Profiler::charge_script(7, 1000);
    }
    Profiler::end_frame();
    assert!(!Profiler::is_recording());
    assert_eq!(Profiler::current_frame(), None);
    assert!(Profiler::with_frames(RingBuffer::is_empty));

    // Disabling keeps what was recorded and records no more.
    Profiler::set_enabled(true);
    Profiler::begin_frame();
    Profiler::end_frame();
    Profiler::set_enabled(false);
    Profiler::begin_frame();
    let _s = Profiler::scope(Subsystem::Physics, "physics");
    Profiler::end_frame();
    assert_eq!(Profiler::with_frames(|f| f.len()), 1);
    assert!(Profiler::latest().expect("kept").scopes().is_empty());
}

#[test]
fn ring_buffer_keeps_last_600() {
    Profiler::set_enabled(true);
    for _ in 0..HISTORY + 25 {
        Profiler::begin_frame();
        Profiler::end_frame();
    }
    Profiler::with_frames(|frames| {
        assert_eq!(frames.len(), 600);
        assert_eq!(frames.capacity(), 600);
        let indices: Vec<u64> = frames.iter().map(|f| f.index()).collect();
        let expected: Vec<u64> = (25..625).collect();
        assert_eq!(indices, expected);
        assert_eq!(frames.get(0).map(|f| f.index()), Some(25));
        assert_eq!(frames.latest().map(|f| f.index()), Some(624));
    });

    let mut ring = RingBuffer::new(3);
    for i in 0..5 {
        ring.push(i);
    }
    assert_eq!(ring.iter().copied().collect::<Vec<_>>(), [2, 3, 4]);
    assert_eq!(ring.iter().rev().copied().collect::<Vec<_>>(), [4, 3, 2]);
    assert_eq!(ring.iter().len(), 3);
}

#[test]
fn scripts_are_summed_per_script() {
    Profiler::set_enabled(true);
    Profiler::begin_frame();
    Profiler::charge_script(1, 100);
    Profiler::charge_script(2, 50);
    Profiler::charge_script(1, 25);
    for extra in 0..40 {
        Profiler::charge_script(100 + extra, 1);
    }
    Profiler::end_frame();
    let frame = Profiler::latest().expect("a frame");
    assert_eq!(frame.scripts()[0].script, 1);
    assert_eq!(frame.scripts()[0].ns, 125);
    assert_eq!(frame.scripts()[0].calls, 2);
    assert_eq!(frame.scripts().len(), MAX_SCRIPTS);
    // 30 of the 40 extras fit; the other 10 are summed.
    assert_eq!(frame.other_scripts_ns(), 10);
    assert_eq!(frame.script_ns(), 125 + 50 + 40);
}

#[test]
fn full_or_deep_frames_drop_scopes_but_keep_their_time_in_the_parent() {
    Profiler::set_enabled(true);
    Profiler::begin_frame();
    {
        let _root = Profiler::scope(Subsystem::Other, "root");
        for _ in 0..MAX_SCOPES + 10 {
            let _s = Profiler::scope(Subsystem::Physics, "leaf");
        }
        let mut deep = Vec::new();
        for _ in 0..40 {
            deep.push(Profiler::scope(Subsystem::Other, "deep"));
        }
        while let Some(scope) = deep.pop() {
            drop(scope);
        }
        let _after = Profiler::scope(Subsystem::Other, "after");
    }
    Profiler::end_frame();
    let frame = Profiler::latest().expect("a frame");
    assert_eq!(frame.scopes().len(), MAX_SCOPES);
    assert_eq!(frame.dropped_scopes(), 11 + 40 + 1);
    let root = frame.scopes()[0];
    assert!(root.inclusive_ns >= root.children_ns);
}

#[test]
fn deep_nesting_stops_at_the_depth_limit() {
    Profiler::set_enabled(true);
    Profiler::begin_frame();
    let mut deep = Vec::new();
    for _ in 0..metallic_profiler::MAX_DEPTH + 4 {
        deep.push(Profiler::scope(Subsystem::Other, "deep"));
    }
    while let Some(scope) = deep.pop() {
        drop(scope);
    }
    let _next = Profiler::scope(Subsystem::Other, "next");
    drop(_next);
    Profiler::end_frame();
    let frame = Profiler::latest().expect("a frame");
    assert_eq!(frame.scopes().len(), metallic_profiler::MAX_DEPTH + 1);
    assert_eq!(frame.dropped_scopes(), 4);
    assert_eq!(
        frame.scopes().last().map(|s| (s.name, s.depth)),
        Some(("next", 0))
    );
}

#[test]
fn a_scope_left_open_closes_with_the_frame() {
    Profiler::set_enabled(true);
    Profiler::begin_frame();
    let open = Profiler::scope(Subsystem::Render, "render");
    spin(Duration::from_micros(200));
    Profiler::end_frame();
    // Dropped after its frame ended: it must not touch the next frame.
    Profiler::begin_frame();
    drop(open);
    Profiler::end_frame();
    let first = Profiler::with_frames(|f| *f.get(0).expect("first"));
    assert_eq!(first.scopes()[0].name, "render");
    assert!(first.scopes()[0].inclusive_ns >= 200_000);
    assert!(Profiler::latest().expect("second").scopes().is_empty());
}

#[test]
fn gpu_time_lands_on_its_frame_late() {
    Profiler::set_enabled(true);
    Profiler::begin_frame();
    let first = Profiler::current_frame().expect("recording");
    Profiler::end_frame();
    for _ in 0..3 {
        Profiler::begin_frame();
        Profiler::end_frame();
    }
    assert_eq!(
        Profiler::with_frames(|f| f.get(0).expect("first").gpu_ns()),
        None
    );
    let passes = [
        GpuPass {
            name: "shadows",
            ns: 300,
        },
        GpuPass {
            name: "scene",
            ns: 700,
        },
    ];
    Profiler::record_gpu(first, &passes);
    Profiler::with_frames(|frames| {
        let frame = frames.get(0).expect("first");
        assert_eq!(frame.gpu(), Some(&passes[..]));
        assert_eq!(frame.gpu_ns(), Some(1000));
        assert_eq!(frames.latest().expect("newest").gpu(), None);
    });
}
