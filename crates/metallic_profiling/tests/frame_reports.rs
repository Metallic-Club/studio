use metallic_profiling::{
    BudgetState, BudgetTable, Clock, FrameProfiler, MAX_SCOPE_DEPTH, Measurement, Subsystem,
};
use std::cell::Cell;
use std::time::Duration;

#[derive(Default)]
struct ManualClock(Cell<Duration>);

impl ManualClock {
    fn advance(&self, millis: u64) {
        self.0.set(self.0.get() + Duration::from_millis(millis));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

#[test]
fn nested_subsystems_use_exclusive_time_and_keep_inclusive_trace() {
    let clock = ManualClock::default();
    let mut profiler = FrameProfiler::<_, 8>::new(&clock);
    let frame = profiler.begin_frame();
    {
        let _scripts = frame.scope(Subsystem::Scripts, "scripts");
        clock.advance(1);
        {
            let _physics = frame.scope(Subsystem::Physics, "physics callback");
            clock.advance(2);
        }
        clock.advance(1);
    }
    clock.advance(6);
    let report = frame.finish();
    assert_eq!(report.frame_time(), Measurement::Complete(ms(10)));
    assert_eq!(
        report.subsystem_time(Subsystem::Scripts),
        Measurement::Complete(ms(2))
    );
    assert_eq!(
        report.subsystem_time(Subsystem::Physics),
        Measurement::Complete(ms(2))
    );
    assert_eq!(report.unscoped_time(), Some(ms(6)));
    let samples: Vec<_> = report.samples().collect();
    assert_eq!(samples[0].name, "physics callback");
    assert_eq!(samples[0].depth, 1);
    assert_eq!(samples[1].inclusive, ms(4));
    assert_eq!(samples[1].exclusive, ms(2));
}

#[test]
fn recursive_scopes_do_not_charge_a_subsystem_twice() {
    let clock = ManualClock::default();
    let mut profiler = FrameProfiler::<_, 8>::new(&clock);
    let frame = profiler.begin_frame();
    {
        let _outer = frame.scope(Subsystem::Scripts, "script");
        clock.advance(2);
        {
            let _inner = frame.scope(Subsystem::Scripts, "script");
            clock.advance(3);
        }
        clock.advance(5);
    }
    let report = frame.finish();
    assert_eq!(
        report.subsystem_time(Subsystem::Scripts),
        Measurement::Complete(ms(10))
    );
    assert_eq!(report.scope_count(Subsystem::Scripts), 2);
    assert_eq!(report.unscoped_time(), Some(Duration::ZERO));
}

#[test]
fn trace_capacity_does_not_truncate_budget_totals() {
    let clock = ManualClock::default();
    let mut profiler = FrameProfiler::<_, 1>::new(&clock);
    let frame = profiler.begin_frame();
    for _ in 0..3 {
        let _scope = frame.scope(Subsystem::Physics, "step");
        clock.advance(2);
    }
    let report = frame.finish();
    assert_eq!(report.samples().count(), 1);
    assert_eq!(report.dropped_samples(), 2);
    assert_eq!(
        report.subsystem_time(Subsystem::Physics),
        Measurement::Complete(ms(6))
    );
    let budgets = BudgetTable::new(ms(10)).with_subsystem(Subsystem::Physics, ms(5));
    assert_eq!(
        report
            .check_budgets(&budgets)
            .subsystem(Subsystem::Physics)
            .state,
        BudgetState::Exceeded
    );
}

#[test]
fn missing_scopes_are_unavailable_while_measured_zero_is_complete() {
    let clock = ManualClock::default();
    let mut profiler = FrameProfiler::<_, 0>::new(&clock);
    let frame = profiler.begin_frame();
    {
        let _scope = frame.scope(Subsystem::Input, "input");
    }
    let report = frame.finish();
    assert_eq!(
        report.subsystem_time(Subsystem::Input),
        Measurement::Complete(Duration::ZERO)
    );
    assert_eq!(
        report.subsystem_time(Subsystem::RenderCpu),
        Measurement::Unavailable
    );
    assert_eq!(report.dropped_samples(), 1);
    let budgets = BudgetTable::new(ms(10))
        .with_subsystem(Subsystem::Input, Duration::ZERO)
        .with_subsystem(Subsystem::RenderCpu, ms(1));
    let checks = report.check_budgets(&budgets);
    assert_eq!(
        checks.subsystem(Subsystem::Input).state,
        BudgetState::Within
    );
    assert_eq!(
        checks.subsystem(Subsystem::RenderCpu).state,
        BudgetState::NotMeasured
    );
    assert_eq!(
        checks.subsystem(Subsystem::GameUi).state,
        BudgetState::NotBudgeted
    );
}

#[test]
fn budget_equality_is_allowed_and_time_outside_scopes_counts_in_frame() {
    let clock = ManualClock::default();
    let mut profiler = FrameProfiler::<_, 4>::new(&clock);
    let frame = profiler.begin_frame();
    {
        let _scope = frame.scope(Subsystem::Scripts, "scripts");
        clock.advance(2);
    }
    clock.advance(3);
    let report = frame.finish();
    let budgets = BudgetTable::new(ms(4)).with_subsystem(Subsystem::Scripts, ms(2));
    let checks = report.check_budgets(&budgets);
    assert_eq!(checks.frame.state, BudgetState::Exceeded);
    assert_eq!(
        checks.subsystem(Subsystem::Scripts).state,
        BudgetState::Within
    );
    assert_eq!(
        checks.subsystem(Subsystem::Scripts).measurement,
        Measurement::Complete(ms(2))
    );
}

#[test]
fn a_backwards_clock_never_produces_a_passing_budget() {
    let clock = ManualClock(Cell::new(ms(20)));
    let mut profiler = FrameProfiler::<_, 4>::new(&clock);
    let frame = profiler.begin_frame();
    {
        let _scope = frame.scope(Subsystem::Scripts, "bad clock");
        clock.0.set(ms(10));
    }
    let report = frame.finish();
    assert!(report.clock_errors() > 0);
    assert!(matches!(report.frame_time(), Measurement::Incomplete(_)));
    assert_eq!(
        report.check_budgets(&BudgetTable::new(ms(100))).frame.state,
        BudgetState::Incomplete
    );
}

#[test]
fn out_of_order_or_forgotten_guards_make_attribution_incomplete() {
    let clock = ManualClock::default();
    let mut profiler = FrameProfiler::<_, 4>::new(&clock);
    let frame = profiler.begin_frame();
    let outer = frame.scope(Subsystem::Scripts, "outer");
    let inner = frame.scope(Subsystem::Physics, "inner");
    clock.advance(1);
    drop(outer);
    drop(inner);
    let report = frame.finish();
    assert!(report.nesting_errors() > 0);
    assert!(matches!(
        report.subsystem_time(Subsystem::Scripts),
        Measurement::Incomplete(_)
    ));
    assert_eq!(report.unscoped_time(), None);

    let frame = profiler.begin_frame();
    std::mem::forget(frame.scope(Subsystem::Scripts, "forgotten"));
    let report = frame.finish();
    assert_eq!(report.unclosed_scopes(), 1);
    assert!(matches!(
        report.subsystem_time(Subsystem::Scripts),
        Measurement::Incomplete(_)
    ));
}

#[test]
fn nesting_capacity_is_bounded_and_never_silently_underreports() {
    let clock = ManualClock::default();
    let mut profiler = FrameProfiler::<_, 1>::new(&clock);
    let frame = profiler.begin_frame();
    let mut guards = Vec::new();
    for _ in 0..MAX_SCOPE_DEPTH + 1 {
        guards.push(frame.scope(Subsystem::Scripts, "nested"));
    }
    clock.advance(1);
    while let Some(guard) = guards.pop() {
        drop(guard);
    }
    drop(guards);
    let report = frame.finish();
    assert_eq!(report.rejected_scopes(), 1);
    let budgets = BudgetTable::new(ms(100)).with_subsystem(Subsystem::Scripts, ms(100));
    assert_eq!(
        report
            .check_budgets(&budgets)
            .subsystem(Subsystem::Scripts)
            .state,
        BudgetState::Incomplete
    );
}

#[test]
fn unwind_closes_the_scope_and_next_frame_has_no_previous_samples() {
    let clock = ManualClock::default();
    let mut profiler = FrameProfiler::<_, 4>::new(&clock);
    let frame = profiler.begin_frame();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        metallic_profiling::cpu_scope!(frame, Subsystem::Scripts, "panicking script");
        clock.advance(3);
        panic!("fixture panic");
    }));
    assert!(result.is_err());
    assert_eq!(
        frame.finish().subsystem_time(Subsystem::Scripts),
        Measurement::Complete(ms(3))
    );
    let next = profiler.begin_frame().finish();
    assert_eq!(next.samples().count(), 0);
    assert_eq!(
        next.subsystem_time(Subsystem::Scripts),
        Measurement::Unavailable
    );
    assert_eq!(next.dropped_samples(), 0);
}
