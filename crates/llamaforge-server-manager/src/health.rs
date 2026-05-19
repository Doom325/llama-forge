//! Health, slots, and metrics HTTP polling.
//! See design doc Section 3.1 & Section 9.

use crate::telemetry::{GpuTelemetry, TelemetryError};
use crate::types::SlotState;
use llamaforge_config::GpuId;
use std::collections::HashMap;
use tokio::process::Command;

/// Gets the CPU usage percentage of a process by PID.
async fn process_cpu_usage(pid: u32) -> Result<f64, std::io::Error> {
    #[cfg(unix)]
    {
        let output = Command::new("ps")
            .arg("-p")
            .arg(pid.to_string())
            .arg("-o")
            .arg("%cpu=")
            .output()
            .await?;

        if !output.status.success() {
            return Err(std::io::Error::other("ps command failed"));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let usage = stdout
            .trim()
            .parse::<f64>()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        Ok(usage)
    }

    #[cfg(not(unix))]
    {
        // On non-Unix, assume 0.0 to avoid false positives
        Ok(0.0)
    }
}

/// Checks if a server process is deadlocked based on four criteria.
/// See design doc Section 3.4.
pub async fn is_deadlocked(
    pid: u32,
    pending_dispatches: u64,
    slots: &[SlotState],
    last_n_decoded: &HashMap<u32, Option<u32>>,
    first_health_ok_at: Option<Instant>,
    gpu_telemetry: &dyn GpuTelemetry,
    gpu_id: &GpuId,
) -> bool {
    // Condition 4: Previously healthy
    if first_health_ok_at.is_none() {
        return false;
    }

    // Condition 1: Active work
    let has_active_work = pending_dispatches > 0 || slots.iter().any(|s| s.is_processing);
    if !has_active_work {
        return false;
    }

    // Condition 2: No decode progress
    // If any active slot has progressed, we are not deadlocked.
    let mut no_progress = true;
    let mut active_slots = 0;
    let mut active_slots_missing_progress = 0;

    for slot in slots {
        if slot.is_processing {
            active_slots += 1;
            if let Some(n_dec) = slot.n_decoded {
                if let Some(&Some(last)) = last_n_decoded.get(&slot.id) {
                    if n_dec > last {
                        no_progress = false;
                        break;
                    }
                }
            } else {
                // If we have mixed-availability (some slots report progress, some omit it),
                // we treat the omitted slots as "not progressing" for the purpose of the
                // `no_progress` check. However, if ALL active slots omit progress, the
                // `active_slots_missing_progress == active_slots` condition below will
                // trigger and safely skip this deadlock check entirely.
                active_slots_missing_progress += 1;
            }
        }
    }

    if active_slots > 0 && active_slots_missing_progress == active_slots {
        // We have active slots, but NONE of them provide n_decoded.
        // We cannot reliably detect a slot-level deadlock. Skip this condition.
        no_progress = false;
    }

    if !no_progress {
        return false;
    }

    // Condition 3: GPU idle
    match gpu_telemetry.utilization_percent(gpu_id) {
        Ok(util) => {
            if util >= 10 {
                return false;
            }
        }
        Err(TelemetryError::Unavailable) => {
            // Fallback to CPU usage
            match process_cpu_usage(pid).await {
                Ok(cpu) => {
                    if cpu >= 50.0 {
                        return false;
                    }
                }
                Err(e) => {
                    tracing::debug!("Failed to get process CPU usage for pid {}: {}", pid, e);
                    return false; // Avoid false positive kills
                }
            }
        }
        Err(_) => {
            return false; // Other errors: avoid false positive kills
        }
    }

    true
}
use reqwest::Client;
use std::time::{Duration, Instant};

/// Creates a new long-lived reqwest client for a server task.
pub fn create_health_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(10)) // Max timeout across endpoints
        .build()
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

    #[tokio::test]
    async fn test_is_deadlocked() {
        struct MockTelemetry {
            util: Result<u8, TelemetryError>,
        }
        impl crate::telemetry::GpuTelemetry for MockTelemetry {
            fn free_vram_bytes(
                &self,
                _id: &llamaforge_config::GpuId,
            ) -> Result<u64, TelemetryError> {
                Ok(0)
            }
            fn utilization_percent(
                &self,
                _id: &llamaforge_config::GpuId,
            ) -> Result<u8, TelemetryError> {
                match &self.util {
                    Ok(v) => Ok(*v),
                    Err(TelemetryError::Unavailable) => Err(TelemetryError::Unavailable),
                    Err(_) => Err(TelemetryError::FfiError { message: "".into() }),
                }
            }
            fn list_devices(&self) -> Result<Vec<crate::telemetry::GpuDevice>, TelemetryError> {
                Ok(vec![])
            }
        }

        let gpu_id = llamaforge_config::GpuId(0);
        let pid = std::process::id(); // Use own PID for CPU check
        let mut last_n_decoded = std::collections::HashMap::new();
        last_n_decoded.insert(0, Some(10));

        let slots = vec![crate::types::SlotState {
            id: 0,
            is_processing: true,
            n_past: Some(100),
            n_decoded: Some(10),
            prompt_hash: None,
        }];

        // Condition 4: previously healthy must be true
        assert!(
            !is_deadlocked(
                pid,
                0,
                &slots,
                &last_n_decoded,
                None,
                &MockTelemetry { util: Ok(0) },
                &gpu_id
            )
            .await
        );

        let healthy_at = Some(Instant::now());

        // Condition 1: active work must be true
        let idle_slots = vec![crate::types::SlotState {
            id: 0,
            is_processing: false,
            n_past: Some(100),
            n_decoded: Some(10),
            prompt_hash: None,
        }];
        assert!(
            !is_deadlocked(
                pid,
                0,
                &idle_slots,
                &last_n_decoded,
                healthy_at,
                &MockTelemetry { util: Ok(0) },
                &gpu_id
            )
            .await
        );

        // Condition 2: no decode progress
        let progress_slots = vec![crate::types::SlotState {
            id: 0,
            is_processing: true,
            n_past: Some(100),
            n_decoded: Some(15),
            prompt_hash: None,
        }];
        assert!(
            !is_deadlocked(
                pid,
                0,
                &progress_slots,
                &last_n_decoded,
                healthy_at,
                &MockTelemetry { util: Ok(0) },
                &gpu_id
            )
            .await
        );

        // All 4 true
        assert!(
            is_deadlocked(
                pid,
                0,
                &slots,
                &last_n_decoded,
                healthy_at,
                &MockTelemetry { util: Ok(0) },
                &gpu_id
            )
            .await
        );

        // Condition 3: GPU util < 10
        assert!(
            !is_deadlocked(
                pid,
                0,
                &slots,
                &last_n_decoded,
                healthy_at,
                &MockTelemetry { util: Ok(15) },
                &gpu_id
            )
            .await
        );

        // Specific test from design Section 8.4: is_processing=true, n_decoded=0, GPU util=95% (legitimate prefill) -> false
        let prefill_slots = vec![crate::types::SlotState {
            id: 0,
            is_processing: true,
            n_past: Some(100),
            n_decoded: Some(0),
            prompt_hash: None,
        }];
        let mut empty_last = std::collections::HashMap::new();
        empty_last.insert(0, Some(0));
        assert!(
            !is_deadlocked(
                pid,
                0,
                &prefill_slots,
                &empty_last,
                healthy_at,
                &MockTelemetry { util: Ok(95) },
                &gpu_id
            )
            .await
        );

        // Telemetry FfiError -> false
        assert!(
            !is_deadlocked(
                pid,
                0,
                &slots,
                &last_n_decoded,
                healthy_at,
                &MockTelemetry { util: Err(TelemetryError::FfiError { message: "".into() }) },
                &gpu_id
            )
            .await
        );

        // Missing n_decoded in all slots -> skips deadlock check
        let missing_progress_slots = vec![crate::types::SlotState {
            id: 0,
            is_processing: true,
            n_past: None,
            n_decoded: None,
            prompt_hash: None,
        }];
        let mut missing_last = std::collections::HashMap::new();
        missing_last.insert(0, None);
        // Even with GPU idle, if we have no progress info at all, we shouldn't kill
        assert!(
            !is_deadlocked(
                pid,
                0,
                &missing_progress_slots,
                &missing_last,
                healthy_at,
                &MockTelemetry { util: Ok(0) },
                &gpu_id
            )
            .await
        );

        // Mixed availability: one missing, one progressing -> skips deadlock kill
        let mixed_progress_slots = vec![
            crate::types::SlotState {
                id: 0,
                is_processing: true,
                n_past: None,
                n_decoded: None,
                prompt_hash: None,
            },
            crate::types::SlotState {
                id: 1,
                is_processing: true,
                n_past: Some(100),
                n_decoded: Some(20),
                prompt_hash: None,
            },
        ];
        let mut mixed_last = std::collections::HashMap::new();
        mixed_last.insert(0, None);
        mixed_last.insert(1, Some(10));
        assert!(
            !is_deadlocked(
                pid,
                0,
                &mixed_progress_slots,
                &mixed_last,
                healthy_at,
                &MockTelemetry { util: Ok(0) },
                &gpu_id
            )
            .await
        );

        // Mixed availability: one missing, one NOT progressing -> kills
        let mixed_no_progress_slots = vec![
            crate::types::SlotState {
                id: 0,
                is_processing: true,
                n_past: None,
                n_decoded: None,
                prompt_hash: None,
            },
            crate::types::SlotState {
                id: 1,
                is_processing: true,
                n_past: Some(100),
                n_decoded: Some(10),
                prompt_hash: None,
            },
        ];
        // using the same mixed_last where slot 1 had 10 -> no progress
        assert!(
            is_deadlocked(
                pid,
                0,
                &mixed_no_progress_slots,
                &mixed_last,
                healthy_at,
                &MockTelemetry { util: Ok(0) },
                &gpu_id
            )
            .await
        );
    }

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
