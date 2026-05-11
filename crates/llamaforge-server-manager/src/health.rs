//! Health, slots, and metrics HTTP polling.
//! See design doc Section 3.1 & Section 9.

use reqwest::Client;
use std::time::{Duration, Instant};

/// Creates a new long-lived reqwest client for a server task.
pub fn create_health_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(10)) // Max timeout across endpoints
        .build()
        .expect("Failed to build reqwest client")
}

/// A fixed-size ring buffer for tracking token-per-second (TPS) observations.
/// See design doc Section 9 (Rolling TPS window).
pub struct RollingTpsWindow {
    samples: std::collections::VecDeque<(Instant, f64)>,
    capacity: usize,
    lifetime_sum: f64,
    lifetime_count: usize,
}

impl Default for RollingTpsWindow {
    fn default() -> Self {
        Self::new(50) // 50 samples for a ~5 minute window at 10s intervals
    }
}

impl RollingTpsWindow {
    /// Creates a new rolling window with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: std::collections::VecDeque::with_capacity(capacity),
            capacity,
            lifetime_sum: 0.0,
            lifetime_count: 0,
        }
    }

    /// Records a new TPS observation and prunes samples older than 5 minutes.
    pub fn record(&mut self, tps: f64) {
        let now = Instant::now();
        let five_mins_ago = now.checked_sub(Duration::from_secs(300)).unwrap_or(now);

        // Prune old samples
        while let Some(&(timestamp, _)) = self.samples.front() {
            if timestamp < five_mins_ago {
                self.samples.pop_front();
            } else {
                break;
            }
        }

        // Add new sample
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back((now, tps));

        // Update lifetime stats
        self.lifetime_sum += tps;
        self.lifetime_count += 1;
    }

    /// Calculates the mean TPS over the 5-minute rolling window.
    pub fn mean_5min(&self) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        let sum: f64 = self.samples.iter().map(|&(_, val)| val).sum();
        Some(sum / self.samples.len() as f64)
    }

    /// Calculates the mean TPS over the entire lifetime.
    pub fn mean_lifetime(&self) -> Option<f64> {
        if self.lifetime_count == 0 {
            return None;
        }
        Some(self.lifetime_sum / self.lifetime_count as f64)
    }

    /// Returns the number of samples currently in the window.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rolling_tps_window() {
        let mut window = RollingTpsWindow::new(50);
        assert_eq!(window.sample_count(), 0);
        assert!(window.mean_5min().is_none());
        assert!(window.mean_lifetime().is_none());

        window.record(10.0);
        window.record(20.0);

        assert_eq!(window.sample_count(), 2);
        assert_eq!(window.mean_5min().unwrap(), 15.0);
        assert_eq!(window.mean_lifetime().unwrap(), 15.0);

        for _ in 0..50 {
            window.record(100.0);
        }

        // Capacity is 50, so the first two samples should be pushed out
        assert_eq!(window.sample_count(), 50);
        assert_eq!(window.mean_5min().unwrap(), 100.0);

        // Lifetime should include all 52 samples: (30 + 5000) / 52
        assert_eq!(window.mean_lifetime().unwrap(), 5030.0 / 52.0);
    }
}
