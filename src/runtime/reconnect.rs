const RECONNECT_BACKOFF_CAP_SECS: u64 = 60;

struct ReconnectBackoff {
    attempt: AtomicU32,
}

impl ReconnectBackoff {
    fn new() -> Self {
        Self {
            attempt: AtomicU32::new(0),
        }
    }

    fn next_delay(&self, base_secs: u64) -> Duration {
        let attempt = self.attempt.fetch_add(1, Ordering::Relaxed);
        Duration::from_millis(reconnect_delay_ms(
            base_secs,
            attempt,
            reconnect_jitter_unit(),
        ))
    }

    fn reset(&self) {
        self.attempt.store(0, Ordering::Relaxed);
    }
}

/// Equal jitter over an exponential slot: `base * 2^attempt`, capped, then
/// spread across the upper half of that slot so a retry is never immediate.
fn reconnect_delay_ms(base_secs: u64, attempt: u32, jitter_unit: u64) -> u64 {
    let base = base_secs.max(1);
    let multiplier = 1u64 << attempt.min(16);
    let cap = RECONNECT_BACKOFF_CAP_SECS.max(base);
    let slot_ms = base.saturating_mul(multiplier).min(cap).saturating_mul(1_000);
    let half = slot_ms / 2;
    let unit = jitter_unit % 1_000;
    half.saturating_add(half.saturating_mul(unit) / 1_000)
}

fn reconnect_jitter_unit() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::from(duration.subsec_nanos()))
        .unwrap_or(0)
        % 1_000
}
