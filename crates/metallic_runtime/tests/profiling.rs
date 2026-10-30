//! Profiling must describe real simulation work without changing gameplay or claiming render time.
use metallic_runtime::profiling::{BudgetState, BudgetTable, Measurement, Subsystem};
use metallic_runtime::{FrameInput, Game, TICK};
use metallic_script::OutputSink;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

struct Silent;
impl OutputSink for Silent {
    fn print(&self, _: &str) {}
    fn warn(&self, _: &str) {}
    fn error(&self, _: &str) {}
}

fn parkour() -> Game {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../templates/parkour");
    let mut game = Game::open(&path, None, Rc::new(Silent)).unwrap();
    game.start().unwrap();
    game
}

#[test]
fn simulation_profiling_is_opt_in_and_does_not_change_frame_output() {
    let mut plain = parkour();
    let mut profiled = parkour();
    assert!(!plain.cpu_profiling_enabled());
    assert!(plain.latest_simulation_profile().is_none());
    profiled.set_cpu_profiling_enabled(true);
    let input = FrameInput {
        movement: [0.25, 0.75],
        ..FrameInput::default()
    };
    let expected = plain.frame(&input, TICK);
    let measured = profiled.frame(&input, TICK);
    assert_eq!(measured, expected);
    assert_eq!(plain.time(), profiled.time());
    assert!(plain.latest_simulation_profile().is_none());
    let report = profiled.latest_simulation_profile().unwrap();
    assert!(matches!(report.frame_time(), Measurement::Complete(_)));
    for stage in [Subsystem::Input, Subsystem::Physics, Subsystem::Scripts] {
        assert!(matches!(
            report.subsystem_time(stage),
            Measurement::Complete(_)
        ));
        assert!(report.scope_count(stage) > 0);
    }
    assert_eq!(
        report.subsystem_time(Subsystem::RenderCpu),
        Measurement::Unavailable
    );
    assert_eq!(
        report.subsystem_time(Subsystem::GameUi),
        Measurement::Unavailable
    );
}

#[test]
fn a_simulation_interval_does_not_certify_a_whole_displayed_frame_budget() {
    let mut game = parkour();
    game.set_cpu_profiling_enabled(true);
    game.frame(&FrameInput::default(), TICK);
    let table = BudgetTable::new(Duration::MAX)
        .with_subsystem(Subsystem::Input, Duration::MAX)
        .with_subsystem(Subsystem::Physics, Duration::MAX)
        .with_subsystem(Subsystem::Scripts, Duration::MAX)
        .with_subsystem(Subsystem::RenderCpu, Duration::MAX);
    let checks = game.latest_simulation_budget(&table).unwrap();
    assert_eq!(checks.frame.state, BudgetState::Within);
    assert_eq!(
        checks.subsystem(Subsystem::RenderCpu).state,
        BudgetState::NotMeasured
    );
    assert_eq!(checks.overall_state(), BudgetState::NotMeasured);
}

#[test]
fn stopped_restarted_disabled_and_reloaded_games_never_expose_stale_reports() {
    let mut game = parkour();
    game.set_cpu_profiling_enabled(true);
    game.frame(&FrameInput::default(), TICK);
    assert!(game.latest_simulation_profile().is_some());
    game.stop();
    assert!(game.latest_simulation_profile().is_none());
    game.frame(&FrameInput::default(), TICK);
    assert!(game.latest_simulation_profile().is_none());
    game.start().unwrap();
    assert!(game.latest_simulation_profile().is_none());
    game.frame(&FrameInput::default(), TICK);
    assert!(game.latest_simulation_profile().is_some());
    game.set_cpu_profiling_enabled(false);
    assert!(game.latest_simulation_profile().is_none());
    game.frame(&FrameInput::default(), TICK);
    assert!(
        game.latest_simulation_budget(&BudgetTable::proposed_dev_60())
            .is_none()
    );
    let reloaded = parkour();
    assert!(!reloaded.cpu_profiling_enabled());
    assert!(reloaded.latest_simulation_profile().is_none());
}

#[test]
fn a_zero_tick_frame_reports_camera_work_without_inventing_simulation_stages() {
    let mut game = parkour();
    game.set_cpu_profiling_enabled(true);
    let output = game.frame(&FrameInput::default(), 0.0);
    assert_eq!(output.ticks, 0);
    let report = game.latest_simulation_profile().unwrap();
    assert!(matches!(
        report.subsystem_time(Subsystem::Input),
        Measurement::Complete(_)
    ));
    assert_eq!(
        report.subsystem_time(Subsystem::Physics),
        Measurement::Unavailable
    );
    assert_eq!(
        report.subsystem_time(Subsystem::Scripts),
        Measurement::Unavailable
    );
}
