use std::time::Duration;

const RETRY_DELAY_BASE_MS: u64 = 500;
const RETRY_DELAY_MAX_MS: u64 = 30_000;

pub(crate) fn retry_backoff_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(16);
    let factor = 1u64 << exponent;
    let delay_ms = RETRY_DELAY_BASE_MS
        .saturating_mul(factor)
        .min(RETRY_DELAY_MAX_MS);
    Duration::from_millis(delay_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_starts_at_base_delay() {
        assert_eq!(retry_backoff_delay(1), Duration::from_millis(500));
    }

    #[test]
    fn backoff_is_capped() {
        assert_eq!(retry_backoff_delay(30), Duration::from_millis(30_000));
    }
}
