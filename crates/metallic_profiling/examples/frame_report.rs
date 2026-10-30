//! Synthetic CPU-only smoke example. These durations are not Metallic gameplay benchmarks.
use metallic_profiling::{BudgetTable, FrameProfiler, Subsystem, SystemClock, cpu_scope};

fn main() {
    #[cfg(feature = "tracy")]
    let _tracy = metallic_profiling::start_tracy();

    #[cfg(feature = "puffin")]
    metallic_profiling::set_puffin_enabled(true);

    let mut profiler = FrameProfiler::<SystemClock, 64>::default();
    let frame = profiler.begin_frame();
    {
        cpu_scope!(frame, Subsystem::Scripts, "synthetic.work");
        let mut checksum = 0_u64;
        for value in 0..10_000 {
            checksum = checksum.wrapping_add(std::hint::black_box(value));
        }
        std::hint::black_box(checksum);
    }
    let report = frame.finish();
    let budgets = report.check_budgets(&BudgetTable::proposed_dev_60());
    println!("Synthetic CPU scope example; no game runtime or GPU measurement.");
    println!(
        "CPU interval: {:?}; proposed interval limit: {:?}",
        report.frame_time(),
        budgets.frame.state
    );
    println!(
        "overall proposed-policy assessment: {:?}",
        budgets.overall_state()
    );
    for subsystem in Subsystem::ALL {
        let check = budgets.subsystem(subsystem);
        println!(
            "{}: {:?}; {:?}",
            subsystem.name(),
            check.measurement,
            check.state
        );
    }
    println!(
        "trace samples: {}; dropped: {}",
        report.samples().count(),
        report.dropped_samples()
    );
}
