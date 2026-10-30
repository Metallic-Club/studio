//! Explicit CPU budgets and assessment of availability/completeness.
use crate::{FrameReport, Measurement, Subsystem};
use std::time::Duration;

/// An assessment of a duration against its configured limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetState {
    /// No limit is configured for this stage, even if a measurement exists.
    NotBudgeted,
    /// A limit exists, but the stage was not instrumented in this frame.
    NotMeasured,
    /// Time accounting is incomplete or unreliable; this is never a passing result.
    Incomplete,
    /// Complete measurement is less than or equal to the configured limit.
    Within,
    /// Complete measurement exceeds the configured limit.
    Exceeded,
}

/// One assessment. Keep measurement and limit alongside the state for honest reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetCheck {
    /// Measured duration, including its availability/completeness.
    pub measurement: Measurement,
    /// Configured limit; `None` is unbudgeted, not a zero budget.
    pub limit: Option<Duration>,
    /// Assessment of this measurement and limit.
    pub state: BudgetState,
}

impl BudgetCheck {
    fn new(measurement: Measurement, limit: Option<Duration>) -> Self {
        let state = match (measurement, limit) {
            (_, None) => BudgetState::NotBudgeted,
            (Measurement::Unavailable, Some(_)) => BudgetState::NotMeasured,
            (Measurement::Incomplete(_), Some(_)) => BudgetState::Incomplete,
            (Measurement::Complete(used), Some(limit)) if used > limit => BudgetState::Exceeded,
            (Measurement::Complete(_), Some(_)) => BudgetState::Within,
        };
        Self {
            measurement,
            limit,
            state,
        }
    }
}

/// A CPU frame limit and optional per-stage limits. Limits are policies, not measurements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetTable {
    frame: Duration,
    subsystems: [Option<Duration>; Subsystem::COUNT],
}

impl BudgetTable {
    /// Set a frame limit with all CPU stages initially unbudgeted.
    pub const fn new(frame: Duration) -> Self {
        Self {
            frame,
            subsystems: [None; Subsystem::COUNT],
        }
    }

    /// Configure an inclusive upper bound for one stage's exclusive CPU time.
    pub const fn with_subsystem(mut self, subsystem: Subsystem, limit: Duration) -> Self {
        self.subsystems[subsystem as usize] = Some(limit);
        self
    }

    /// The configured frame limit, including time outside recorded scopes.
    pub const fn frame_limit(&self) -> Duration {
        self.frame
    }
    /// The configured stage limit, or `None` when that stage is intentionally unbudgeted.
    pub const fn subsystem_limit(&self, subsystem: Subsystem) -> Option<Duration> {
        self.subsystems[subsystem as usize]
    }

    /// Proposed Dev/60-fps CPU limits from performance research section 3.2; not accepted targets.
    pub const fn proposed_dev_60() -> Self {
        Self::from_micros(
            16_667,
            [
                Some(200),
                Some(2000),
                Some(2000),
                Some(300),
                Some(1500),
                Some(500),
                Some(500),
                Some(100),
            ],
        )
    }
    /// Proposed Mid/60-fps CPU limits from the same table; no matching hardware is implied.
    pub const fn proposed_mid_60() -> Self {
        Self::from_micros(
            16_667,
            [
                Some(300),
                Some(2500),
                Some(2500),
                Some(500),
                Some(2500),
                Some(700),
                Some(500),
                Some(100),
            ],
        )
    }
    /// Proposed Low/30-fps CPU limits from the same table; integrated-GPU performance is unverified.
    pub const fn proposed_low_30() -> Self {
        Self::from_micros(
            33_333,
            [
                Some(500),
                Some(5000),
                Some(5000),
                Some(1000),
                Some(4000),
                Some(1500),
                Some(1000),
                Some(200),
            ],
        )
    }
    /// Proposed Server/60-Hz CPU limits. Rendering and game UI are unbudgeted, not zero-cost.
    pub const fn proposed_server_60() -> Self {
        Self::from_micros(
            16_667,
            [
                Some(500),
                Some(4000),
                Some(4000),
                None,
                None,
                None,
                Some(3000),
                Some(200),
            ],
        )
    }

    const fn from_micros(frame: u64, values: [Option<u64>; Subsystem::COUNT]) -> Self {
        let mut table = Self::new(Duration::from_micros(frame));
        let mut index = 0;
        while index < Subsystem::COUNT {
            table.subsystems[index] = match values[index] {
                Some(value) => Some(Duration::from_micros(value)),
                None => None,
            };
            index += 1;
        }
        table
    }
}

/// Independent frame and CPU-stage checks; no overall pass hides missing stage measurements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetReport {
    /// Whole-frame assessment, including unscoped work.
    pub frame: BudgetCheck,
    subsystems: [BudgetCheck; Subsystem::COUNT],
}

impl BudgetReport {
    pub(crate) fn new<const N: usize>(frame: &FrameReport<N>, table: &BudgetTable) -> Self {
        Self {
            frame: BudgetCheck::new(frame.frame_time(), Some(table.frame)),
            subsystems: std::array::from_fn(|index| {
                BudgetCheck::new(
                    frame.subsystem_time(Subsystem::ALL[index]),
                    table.subsystems[index],
                )
            }),
        }
    }

    /// Assessment for one CPU stage, retaining unavailable or incomplete data explicitly.
    pub const fn subsystem(&self, subsystem: Subsystem) -> &BudgetCheck {
        &self.subsystems[subsystem as usize]
    }
}
