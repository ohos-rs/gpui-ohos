use std::{error::Error, fmt};

/// An invalid native refresh request or a window absent from the calling UI thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeRefreshError {
    InvalidRate(u32),
    WindowUnavailable(i64),
}

impl fmt::Display for NativeRefreshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRate(hz) => {
                write!(f, "NativeVSync requires a rate in 1..=i32::MAX, got {hz}")
            }
            Self::WindowUnavailable(id) => write!(
                f,
                "No live GPUI frame owner for window {id} on this UI thread"
            ),
        }
    }
}

impl Error for NativeRefreshError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeRefresh {
    pub(crate) window_id: u64,
    pub(crate) hz: i32,
}

impl NativeRefresh {
    pub(crate) fn new(window_id: u64, hz: u32) -> Result<Self, NativeRefreshError> {
        let rate = i32::try_from(hz)
            .ok()
            .filter(|rate| *rate > 0)
            .ok_or(NativeRefreshError::InvalidRate(hz))?;
        Ok(Self {
            window_id,
            hz: rate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_and_rates_that_overflow_the_native_api() {
        for hz in [0, i32::MAX as u32 + 1, u32::MAX] {
            assert_eq!(
                NativeRefresh::new(7, hz),
                Err(NativeRefreshError::InvalidRate(hz))
            );
        }
    }

    #[test]
    fn preserves_window_and_valid_rate_without_a_fixed_target() {
        for hz in [1, 60, 90, 120, 144, i32::MAX as u32] {
            assert_eq!(
                NativeRefresh::new(42, hz),
                Ok(NativeRefresh {
                    window_id: 42,
                    hz: hz as i32
                })
            );
        }
    }
}
