use std::{fmt, time::Duration};

pub const FIXED_STEP: Duration = Duration::from_nanos(16_666_667);
pub const MAX_CATCH_UP: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockError;
impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("non-monotonic or overflowing preview time")
    }
}
impl std::error::Error for ClockError {}

/// Inject elapsed monotonic host time. At most four fixed ticks per wake;
/// excess whole ticks are discarded, avoiding a catch-up spiral after a stall.
#[derive(Default)]
pub struct FrameClock {
    last: Option<Duration>,
    remainder: Duration,
}
impl FrameClock {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn advance(&mut self, now: Duration) -> Result<u32, ClockError> {
        let elapsed = match self.last {
            Some(last) => now.checked_sub(last).ok_or(ClockError)?,
            None => Duration::ZERO,
        };
        let total = self.remainder.checked_add(elapsed).ok_or(ClockError)?;
        let steps = (total.as_nanos() / FIXED_STEP.as_nanos()).min(u128::from(MAX_CATCH_UP)) as u32;
        self.remainder = Duration::from_nanos((total.as_nanos() % FIXED_STEP.as_nanos()) as u64);
        self.last = Some(now);
        Ok(steps)
    }
    pub fn next_in(&self) -> Duration {
        FIXED_STEP - self.remainder
    }
}
