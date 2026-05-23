#![cfg(feature = "test-utils")]

use llamaforge_server::__test_exports::compute_backoff_delay;
use std::time::Duration;
use tokio::time::Instant;

#[tokio::test]
async fn test_backoff_schedule() {
    tokio::time::pause();

    let cooldown_base = 10.0;

    let mut attempts = vec![];

    // Simulate backoff sequence
    for restart_count in 1..=4 {
        let delay_secs = compute_backoff_delay(restart_count, cooldown_base);
        let start = Instant::now();
        tokio::time::sleep(Duration::from_secs_f64(delay_secs)).await;
        attempts.push(start.elapsed().as_secs_f64());
    }

    // attempt 1 ∈ [8s, 12s]
    assert!(attempts[0] >= 8.0 && attempts[0] <= 12.0);
    // attempt 2 ∈ [16s, 24s]
    assert!(attempts[1] >= 16.0 && attempts[1] <= 24.0);
    // attempt 3 ∈ [32s, 48s]
    assert!(attempts[2] >= 32.0 && attempts[2] <= 48.0);
    // attempt 4 ∈ [32s, 48s]
    assert!(attempts[3] >= 32.0 && attempts[3] <= 48.0);
}
