use chrono::Utc;
use llamaforge_config::{ConfigReader, ServerConfig, ServerId};
use llamaforge_metrics::{ManagedProcess, ProcessId, ProcessStore};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use tracing::{error, info, warn};

#[cfg(unix)]
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};

use crate::manager::ServerManager;
use crate::types::ServerLifecycle;
use crate::types::SlotState;
use crate::vram_estimate::{parse_gguf, GgufError, GgufMetadata};

pub fn compute_backoff_delay(restart_count: u32, cooldown_base: f64) -> f64 {
    let max_delay = cooldown_base * 4.0;
    let mut base_delay = cooldown_base * 2.0_f64.powi(restart_count.saturating_sub(1) as i32);
    if base_delay > max_delay {
        base_delay = max_delay;
    }

    let jitter_range = base_delay * 0.2;
    let jitter = (rand::random::<f64>() * 2.0 * jitter_range) - jitter_range;
    (base_delay + jitter).max(1.0)
}

pub(crate) fn release_permit_with_stagger(
    permit: tokio::sync::OwnedSemaphorePermit,
    stagger_secs: u64,
) {
    if stagger_secs > 0 {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(stagger_secs)).await;
            drop(permit);
        });
    } else {
        drop(permit);
    }
}

pub async fn spawn_server_task(
    mgr: Arc<ServerManager>,
    server_id: ServerId,
    config: ServerConfig,
    mut orphan_pid: Option<u32>,
) {
    let mut restart_count = 0;
    let max_restarts = config.max_restart_attempts;

    loop {
        // Parse GGUF or fallback
        let meta = match parse_gguf(&config.model_path).await {
            Ok(m) => m,
            Err(GgufError::Io(e)) => {
                error!(
                    server.id = %server_id,
                    server.gguf_unreadable = %e,
                    "Failed to read model file: {}",
                    e
                );
                transition_to(&mgr, &server_id, ServerLifecycle::Failed);
                break;
            }
            Err(e @ GgufError::Malformed) | Err(e @ GgufError::UnsupportedVersion(_)) => {
                if let Some(est_mb) = config.estimated_vram_mb {
                    warn!(
                        server.id = %server_id,
                        server.gguf_parse_failed = %e,
                        "GGUF parse failed, using estimated_vram_mb override ({} MB)",
                        est_mb
                    );
                    // Minimal fallback struct
                    GgufMetadata {
                        architecture: crate::types::ModelArchitecture::Other("unknown".into()),
                        block_count: 0,
                        embedding_length: 0,
                        head_count: 0,
                        head_count_kv: 0,
                        quantization: crate::vram_estimate::Quantization::Other("unknown".into()),
                        file_size_bytes: 0,
                    }
                } else {
                    error!(
                        server.id = %server_id,
                        server.gguf_required = %e,
                        "GGUF metadata required for VRAM estimation, but parsing failed"
                    );
                    transition_to(&mgr, &server_id, ServerLifecycle::Failed);
                    break;
                }
            }
        };

        // Cache metadata
        if let Some(mut state) = mgr.servers.get_mut(&server_id) {
            state.gguf_metadata = Some(Arc::new(meta));
        }

        let meta_ref = match mgr.servers.get(&server_id) {
            Some(s) => match &s.gguf_metadata {
                Some(m) => m.clone(),
                None => continue,
            },
            None => break,
        };

        let gpu_id = match mgr.servers.get(&server_id) {
            Some(s) => s.gpu_id.clone(),
            None => break,
        };

        let mut child;
        let pid;
        let mut permit: Option<tokio::sync::OwnedSemaphorePermit> = None;

        if let Some(opid) = orphan_pid.take() {
            pid = opid;
            child = crate::process::ProcessHandle::Orphan(pid);
            if let Some(mut state) = mgr.servers.get_mut(&server_id) {
                state.pid = Some(pid);
                state.lifecycle = ServerLifecycle::Running;
                state.first_health_ok_at = Some(std::time::Instant::now());
            }
        } else {
            let gpu_semaphore = {
                let entry = mgr
                    .gpu_semaphores
                    .entry(gpu_id.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(1)));
                entry.value().clone()
            };
            // INVARIANT: The Arc<Semaphore> is owned by the DashMap which has the same lifetime as ServerManager.
            // We only insert and acquire, never close the semaphore.
            permit = Some(gpu_semaphore.acquire_owned().await.expect("Semaphore closed"));

            if let Err(e) = mgr.pre_spawn_vram_check(&server_id, &config, &gpu_id, &meta_ref) {
                error!(server.id = %server_id, "Insufficient VRAM: {}", e);
                let stagger = mgr.config.snapshot().global.restart_stagger_secs;
                if let Some(p) = permit.take() {
                    release_permit_with_stagger(p, stagger);
                }
                transition_to(&mgr, &server_id, ServerLifecycle::Failed);
                break;
            }

            // Spawn child
            child = match crate::process::spawn_child_process(&config) {
                Ok(c) => c,
                Err(e) => {
                    error!(server.id = %server_id, "Failed to spawn process: {}", e);
                    mgr.clear_vram_reservation(&server_id);
                    let stagger = mgr.config.snapshot().global.restart_stagger_secs;
                    if let Some(p) = permit.take() {
                        release_permit_with_stagger(p, stagger);
                    }
                    transition_to(&mgr, &server_id, ServerLifecycle::Failed);
                    break;
                }
            };

            pid = match child.id() {
                Some(id) => id,
                None => {
                    error!(server.id = %server_id, "Child has no PID immediately after spawn");
                    mgr.clear_vram_reservation(&server_id);
                    let stagger = mgr.config.snapshot().global.restart_stagger_secs;
                    if let Some(p) = permit.take() {
                        release_permit_with_stagger(p, stagger);
                    }
                    transition_to(&mgr, &server_id, ServerLifecycle::Failed);
                    break;
                }
            };

            if let Some(mut state) = mgr.servers.get_mut(&server_id) {
                state.pid = Some(pid);
                state.restart_count = restart_count;
                state.lifecycle = ServerLifecycle::Starting;
            }
        }

        let model_alias = config.alias.clone().unwrap_or_else(|| "default".to_string());

        // Upsert to SQLite BEFORE entering monitor loop
        let process_record = ManagedProcess {
            pid: ProcessId(pid),
            server_id: server_id.clone(),
            started_at: Utc::now(),
            cmdline: format!(
                "{} --model {}",
                config.binary_path.display(),
                config.model_path.display()
            ),
            port: config.port,
            model_alias,
        };

        if let Err(e) = mgr.metrics_store.upsert_managed_process(&process_record).await {
            error!(server.id = %server_id, "Failed to persist managed process row: {}", e);
        }

        // Monitor loop
        let should_restart =
            monitor_loop(&mgr, &server_id, &config, &mut child, pid, &mut permit).await;

        // Cleanup after loop
        #[cfg(unix)]
        {
            // Kill child if still alive
            if child.id().is_some() {
                let _ = kill(Pid::from_raw(-(pid as i32)), Signal::SIGKILL);
            }
        }

        let _ = child.wait().await;

        mgr.clear_vram_reservation(&server_id);

        let stagger = mgr.config.snapshot().global.restart_stagger_secs;
        if let Some(p) = permit.take() {
            release_permit_with_stagger(p, stagger);
        }

        if let Err(e) = mgr.metrics_store.remove_managed_process(ProcessId(pid)).await {
            error!(server.id = %server_id, "Failed to remove managed process row: {}", e);
        }

        if !should_restart {
            transition_to(&mgr, &server_id, ServerLifecycle::Stopped);
            break;
        }

        restart_count += 1;
        if restart_count >= max_restarts {
            error!(server.id = %server_id, "Max restarts exceeded");
            transition_to(&mgr, &server_id, ServerLifecycle::Failed);
            break;
        }

        info!(server.id = %server_id, "Backing off before restart {}", restart_count);

        // Stamp the restart wall-clock for the next spawn cycle so
        // `ServerStateSnapshot::last_restart_at` reflects when the
        // restart was initiated.
        if let Some(mut state) = mgr.servers.get_mut(&server_id) {
            state.last_restart_at = Some(chrono::Utc::now());
        }

        let cooldown_base = config.restart_cooldown_secs as f64;
        let delay_secs = compute_backoff_delay(restart_count, cooldown_base);

        sleep(Duration::from_secs_f64(delay_secs)).await;
    }
}

pub(crate) enum DegradationAction {
    None,
    WarnOnly,
    Restart,
}

pub(crate) fn check_degradation(
    current_tps: f64,
    sample_count: usize,
    baseline: Option<f64>,
    slots: &[SlotState],
    ctx_size: u32,
    restart_on_degradation: bool,
    consecutive_degraded_ticks: &mut u32,
) -> DegradationAction {
    if sample_count < 30 {
        *consecutive_degraded_ticks = 0;
        return DegradationAction::None;
    }

    for slot in slots {
        if slot.is_processing && slot.n_past < (ctx_size / 10) {
            *consecutive_degraded_ticks = 0;
            return DegradationAction::None;
        }
    }

    if let Some(base) = baseline {
        if current_tps < 0.5 * base {
            *consecutive_degraded_ticks += 1;
            if *consecutive_degraded_ticks >= 3 {
                if restart_on_degradation {
                    return DegradationAction::Restart;
                } else {
                    return DegradationAction::WarnOnly;
                }
            }
        } else {
            *consecutive_degraded_ticks = 0;
        }
    } else {
        *consecutive_degraded_ticks = 0;
    }

    DegradationAction::None
}
async fn monitor_loop(
    mgr: &ServerManager,
    server_id: &ServerId,
    config: &ServerConfig,
    child: &mut crate::process::ProcessHandle,
    pid: u32,
    _permit: &mut Option<tokio::sync::OwnedSemaphorePermit>,
) -> bool {
    use crate::health::{create_health_client, RollingTpsWindow};
    use crate::types::{ClientRefreshEvent, SlotState};
    use tokio::time::{interval, MissedTickBehavior};

    let client = match create_health_client() {
        Ok(c) => c,
        Err(e) => {
            error!(server.id = %server_id, "Failed to build reqwest client: {}", e);
            transition_to(mgr, server_id, ServerLifecycle::Failed);
            return false; // Do not restart, something is fundamentally wrong
        }
    };

    let port = mgr.servers.get(server_id).map(|s| s.port).unwrap_or(8080);
    let base_url = format!("http://127.0.0.1:{}", port);

    let mut consecutive_health_failures = 0;
    let restart_on_degradation = config.restart_on_degradation;
    let mut consecutive_degraded_ticks = 0;

    let mut tps_window = RollingTpsWindow::default();
    let mut last_total_tokens_decoded: u64 = 0;

    let mut health_interval = interval(Duration::from_secs(2));
    health_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    // Initial jitter
    sleep(Duration::from_millis((rand::random::<f64>() * 500.0) as u64)).await;

    let mut slots_interval = interval(Duration::from_secs(5));
    slots_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut metrics_interval = interval(Duration::from_secs(10));
    metrics_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let task_shutdown_token =
        mgr.servers.get(server_id).map(|s| s.task_shutdown_token.clone()).unwrap_or_default();

    loop {
        tokio::select! {
            // Cancellation / Shutdown
            _ = mgr.shutdown_token.cancelled() => {
                info!(server.id = %server_id, "Shutdown requested, draining");
                transition_to(mgr, server_id, ServerLifecycle::Draining);

                // Drain logic per design Section 6 and PR7 Task 7.2
                let drain_timeout = crate::manager::resolve_drain_timeout(config);
                let _ = timeout(drain_timeout, async {
                    loop {
                        if mgr.pending_dispatches(server_id) == 0 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }).await;

                #[cfg(unix)]
                {
                    let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
                }

                // Wait up to 10 seconds for graceful exit
                if timeout(Duration::from_secs(10), child.wait()).await.is_err() {
                    warn!(server.id = %server_id, "Child did not exit gracefully, SIGKILLing process group");
                    #[cfg(unix)]
                    {
                        let _ = kill(Pid::from_raw(-(pid as i32)), Signal::SIGKILL);
                    }
                }

                return false; // Do not restart
            }
            _ = task_shutdown_token.cancelled() => {
                info!(server.id = %server_id, "Server task shutdown requested, draining");
                transition_to(mgr, server_id, ServerLifecycle::Draining);

                let drain_timeout = crate::manager::resolve_drain_timeout(config);
                let _ = timeout(drain_timeout, async {
                    loop {
                        if mgr.pending_dispatches(server_id) == 0 {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }).await;

                #[cfg(unix)]
                {
                    let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
                }

                // Wait up to 10 seconds for graceful exit
                if timeout(Duration::from_secs(10), child.wait()).await.is_err() {
                    warn!(server.id = %server_id, "Child did not exit gracefully, SIGKILLing process group");
                    #[cfg(unix)]
                    {
                        let _ = kill(Pid::from_raw(-(pid as i32)), Signal::SIGKILL);
                    }
                }

                return false; // Do not restart
            }

            // Child exit
            status = child.wait() => {
                let code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                info!(server.id = %server_id, server.stopped = true, "Server process exited with code {}", code);

                if !mgr.shutdown_token.is_cancelled() {
                    info!(server.id = %server_id, server.restarted = true, reason = "crash", "Server restarting");
                }

                mgr.clear_vram_reservation(server_id);
                transition_to(mgr, server_id, ServerLifecycle::Restarting);
                return true; // Restart
            }

            // Health tick
            _ = health_interval.tick() => {
                let mut is_dl = false;
                let dl_args = mgr.servers.get(server_id).map(|s| {
                    (s.slots.clone(), s.last_n_decoded.clone(), s.first_health_ok_at, s.gpu_id.clone())
                });

                if let Some((slots, last_n_decoded, first_health_ok_at, gpu_id)) = dl_args {
                    is_dl = crate::health::is_deadlocked(
                        pid,
                        mgr.pending_dispatches(server_id),
                        &slots,
                        &last_n_decoded,
                        first_health_ok_at,
                        &*mgr.gpu_telemetry,
                        &gpu_id,
                    ).await;
                }

                if is_dl {
                    error!(
                        server.id = %server_id,
                        server.deadlock_detected = true,
                        "Server deadlock detected. Sending SIGKILL."
                    );
                    info!(server.id = %server_id, server.restarted = true, reason = "deadlock", "Server restarting");
                    #[cfg(unix)]
                    {
                        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
                    }
                    // Transition to Restarting to match reason, but let the child.wait() branch handle the actual restart
                    // to ensure unified cleanup.
                    transition_to(mgr, server_id, ServerLifecycle::Restarting);
                    continue;
                }

                let url = format!("{}/health", base_url);
                match client.get(&url).send().await {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            consecutive_health_failures = 0;

                            if let Some(state) = mgr.servers.get(server_id) {
                                if state.lifecycle == ServerLifecycle::Starting {
                                    // Drop dashmap lock before calling clear_vram_reservation
                                    drop(state);
                                    mgr.clear_vram_reservation(server_id);

                                    let stagger = mgr.config().snapshot().global.restart_stagger_secs;
                                    if let Some(p) = _permit.take() {
                                        release_permit_with_stagger(p, stagger);
                                    }

                                    if let Some(mut state2) = mgr.servers.get_mut(server_id) {
                                        state2.lifecycle = ServerLifecycle::Running;
                                        state2.first_health_ok_at = Some(std::time::Instant::now());
                                    }

                                    let _ = mgr.client_refresh_tx.send(ClientRefreshEvent {
                                        server_id: server_id.clone(),
                                        new_port: port,
                                    });
                                }
                            }
                        } else {
                            error!(server.id = %server_id, "Health check failed with status: {}", resp.status());
                            consecutive_health_failures += 1;
                            if consecutive_health_failures >= 3 {
                                warn!(server.id = %server_id, server.health_fail = true, "Server failed health checks 3 times consecutively");
                                transition_to(mgr, server_id, ServerLifecycle::Restarting);
                                info!(server.id = %server_id, server.restarted = true, reason = "health_fail", "Server restarting");
                                return true; // Restart
                            }
                        }
                    }
                    Err(e) => {
                        error!(server.id = %server_id, "Health check error: {}", e);
                        consecutive_health_failures += 1;
                        if consecutive_health_failures >= 3 {
                            warn!(server.id = %server_id, server.health_fail = true, "Server failed health checks 3 times consecutively");
                            transition_to(mgr, server_id, ServerLifecycle::Restarting);
                            info!(server.id = %server_id, server.restarted = true, reason = "health_fail", "Server restarting");
                            return true; // Restart
                        }
                    }
                }
            }

            // Slots tick
            _ = slots_interval.tick() => {
                let url = format!("{}/slots", base_url);
                if let Ok(resp) = client.get(&url).send().await {
                    if let Ok(slots) = resp.json::<Vec<SlotState>>().await {
                        if let Some(mut state) = mgr.servers.get_mut(server_id) {
                            state.slots = slots.clone();
                            for slot in slots {
                                state.last_n_decoded.insert(slot.id, slot.n_decoded);
                            }
                        }
                    }
                }
            }

            // Metrics tick
            _ = metrics_interval.tick() => {
                #[cfg(any(test, feature = "test-utils"))]
                {
                    if let Some(mut state) = mgr.servers.get_mut(server_id) {
                        if let Some((samples, baseline)) = state.test_warm_tps_samples.take() {
                            for _ in 0..samples {
                                tps_window.record(baseline);
                            }
                        }
                    }
                }

                let url = format!("{}/metrics", base_url);
                if let Ok(resp) = client.get(&url).send().await {
                    if let Ok(body) = resp.text().await {
                        // Extract llamacpp:n_decode_total
                        let mut n_decode_total: Option<u64> = None;
                        for line in body.lines() {
                            if line.starts_with("llamacpp:n_decode_total ") {
                                if let Some(val_str) = line.split_whitespace().nth(1) {
                                    n_decode_total = val_str.parse().ok();
                                }
                                break;
                            }
                        }

                        if let Some(total) = n_decode_total {
                            let diff = total.saturating_sub(last_total_tokens_decoded);
                            last_total_tokens_decoded = total;

                            // It's a 10s interval
                            let current_tps = (diff as f64 / 10.0).max(0.0);
                            tps_window.record(current_tps);

                            let mut slots = Vec::new();
                            let mut ctx_size = 4096;
                            if let Some(state) = mgr.servers.get(server_id) {
                                slots = state.slots.clone();
                                ctx_size = state.ctx_size;
                            }

                            let action = check_degradation(
                                current_tps,
                                tps_window.sample_count(),
                                tps_window.mean_lifetime(),
                                &slots,
                                ctx_size,
                                restart_on_degradation,
                                &mut consecutive_degraded_ticks,
                            );

                            match action {
                                DegradationAction::Restart => {
                                    let mean_5min = tps_window.mean_5min().unwrap_or(0.0);
                                    warn!(
                                        server.id = %server_id,
                                        server.performance_degraded = true,
                                        current_tps = current_tps,
                                        rolling_avg = mean_5min,
                                        "Server performance significantly degraded, restarting."
                                    );
                                    transition_to(mgr, server_id, ServerLifecycle::Restarting);
                                    info!(server.id = %server_id, server.restarted = true, reason = "degradation", "Server restarting");
                                    return true;
                                }
                                DegradationAction::WarnOnly => {
                                    let mean_5min = tps_window.mean_5min().unwrap_or(0.0);
                                    warn!(
                                        server.id = %server_id,
                                        server.performance_degraded = true,
                                        current_tps = current_tps,
                                        rolling_avg = mean_5min,
                                        "Server performance significantly degraded."
                                    );
                                }
                                DegradationAction::None => {}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn transition_to(mgr: &ServerManager, server_id: &ServerId, new_state: ServerLifecycle) {
    if let Some(mut state) = mgr.servers.get_mut(server_id) {
        state.lifecycle = new_state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SlotState;

    #[test]
    fn test_degradation_logic_table() {
        // Case 1: Cold start guard
        let mut consec = 0;
        let action = check_degradation(1.0, 20, Some(10.0), &[], 4096, true, &mut consec);
        assert!(matches!(action, DegradationAction::None));
        assert_eq!(consec, 0);

        // Case 2: Prefill guard
        let prefill_slot =
            SlotState { id: 0, is_processing: true, n_past: 100, n_decoded: 0, prompt_hash: None };
        let action =
            check_degradation(1.0, 50, Some(10.0), &[prefill_slot], 4096, true, &mut consec);
        assert!(matches!(action, DegradationAction::None));
        assert_eq!(consec, 0);

        // Case 3: Degraded tick (consecutive = 1)
        let normal_slot = SlotState {
            id: 0,
            is_processing: true,
            n_past: 1000,
            n_decoded: 10,
            prompt_hash: None,
        };
        let action = check_degradation(
            1.0,
            50,
            Some(10.0),
            std::slice::from_ref(&normal_slot),
            4096,
            true,
            &mut consec,
        );
        assert!(matches!(action, DegradationAction::None));
        assert_eq!(consec, 1);

        // Case 4: Degraded tick (consecutive = 2)
        let action = check_degradation(
            1.0,
            50,
            Some(10.0),
            std::slice::from_ref(&normal_slot),
            4096,
            true,
            &mut consec,
        );
        assert!(matches!(action, DegradationAction::None));
        assert_eq!(consec, 2);

        // Case 5: Degraded tick (consecutive = 3) with restart
        let action = check_degradation(
            1.0,
            50,
            Some(10.0),
            std::slice::from_ref(&normal_slot),
            4096,
            true,
            &mut consec,
        );
        assert!(matches!(action, DegradationAction::Restart));
        assert_eq!(consec, 3);

        // Case 6: Degraded tick (consecutive = 3) without restart
        consec = 2; // Reset to 2
        let action = check_degradation(
            1.0,
            50,
            Some(10.0),
            std::slice::from_ref(&normal_slot),
            4096,
            false,
            &mut consec,
        );
        assert!(matches!(action, DegradationAction::WarnOnly));
        assert_eq!(consec, 3);

        // Case 7: Recovery resets counter
        let action =
            check_degradation(8.0, 50, Some(10.0), &[normal_slot], 4096, true, &mut consec);
        assert!(matches!(action, DegradationAction::None));
        assert_eq!(consec, 0);
    }

    #[test]
    fn test_backoff_math() {
        let base = 10.0;

        // Attempt 1
        for _ in 0..100 {
            let delay = compute_backoff_delay(1, base);
            assert!((8.0..=12.0).contains(&delay));
        }

        // Attempt 2
        for _ in 0..100 {
            let delay = compute_backoff_delay(2, base);
            assert!((16.0..=24.0).contains(&delay));
        }

        // Attempt 3
        for _ in 0..100 {
            let delay = compute_backoff_delay(3, base);
            assert!((32.0..=48.0).contains(&delay));
        }

        // Attempt 4 (capped)
        for _ in 0..100 {
            let delay = compute_backoff_delay(4, base);
            assert!((32.0..=48.0).contains(&delay));
        }
    }
}
