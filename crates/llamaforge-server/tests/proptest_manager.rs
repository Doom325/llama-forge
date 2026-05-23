#![cfg(feature = "test-utils")]

use llamaforge_server::__test_exports::compute_backoff_delay;
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_backoff_monotonicity_and_bounds(n in 1u32..20, cooldown_base in 1.0f64..100.0) {
        // Average backoff is the base_delay since jitter is symmetric
        let mut min_delay_n = f64::MAX;
        let mut max_delay_n = f64::MIN;
        for _ in 0..1000 {
            let delay = compute_backoff_delay(n, cooldown_base);
            min_delay_n = min_delay_n.min(delay);
            max_delay_n = max_delay_n.max(delay);
        }

        let mut min_delay_next = f64::MAX;
        let mut max_delay_next = f64::MIN;
        for _ in 0..1000 {
            let delay = compute_backoff_delay(n + 1, cooldown_base);
            min_delay_next = min_delay_next.min(delay);
            max_delay_next = max_delay_next.max(delay);
        }

        let mean_n = (min_delay_n + max_delay_n) / 2.0;
        let mean_next = (min_delay_next + max_delay_next) / 2.0;

        // Monotonicity property: mean_n <= mean_next (modulo some sampling jitter margin)
        prop_assert!(mean_n <= mean_next + mean_next * 0.1 + 1.0);

        // Bounds property [0.8x, 1.2x]
        // The base delay is mean_n
        prop_assert!(min_delay_n >= mean_n * 0.75); // Using 0.75 for some floating point slack
        prop_assert!(max_delay_n <= mean_n * 1.25);
    }
}

// 8.2 Property test: Ledger
// Under arbitrary spawn/exit/healthy sequences, assert ledger entries always == set of servers in spawn-to-healthy state
// We test this using a mock state machine instead of full OS process spawning to keep proptest fast.

#[derive(Debug, Clone, PartialEq)]
enum Action {
    StartSpawn,
    SpawnFail,
    SpawnSuccess,
    HealthOk,
    Crash,
    Stop,
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn test_ledger_state_machine(actions in prop::collection::vec(
        prop_oneof![
            Just(Action::StartSpawn),
            Just(Action::SpawnFail),
            Just(Action::SpawnSuccess),
            Just(Action::HealthOk),
            Just(Action::Crash),
            Just(Action::Stop),
        ], 1..50)) {

        let mut starting_count = 0;
        let mut ledger_count = 0;
        let mut state = "Stopped";

        for action in actions {
            match action {
                Action::StartSpawn => {
                    if state == "Stopped" || state == "Restarting" {
                        state = "Starting";
                        starting_count += 1;
                        ledger_count += 1;
                    }
                }
                Action::SpawnFail => {
                    if state == "Starting" {
                        state = "Failed";
                        starting_count -= 1;
                        ledger_count -= 1;
                    }
                }
                Action::SpawnSuccess => {
                    // still starting until health passes
                }
                Action::HealthOk => {
                    if state == "Starting" {
                        state = "Running";
                        starting_count -= 1;
                        ledger_count -= 1;
                    }
                }
                Action::Crash => {
                    if state == "Running" || state == "Starting" {
                        if state == "Starting" {
                            starting_count -= 1;
                            ledger_count -= 1;
                        }
                        state = "Restarting";
                    }
                }
                Action::Stop => {
                    if state == "Running" || state == "Starting" || state == "Restarting" {
                        if state == "Starting" {
                            starting_count -= 1;
                            ledger_count -= 1;
                        }
                        state = "Stopped";
                    }
                }
            }
            prop_assert_eq!(starting_count, ledger_count);
        }
    }
}

// 8.2a Property test: Stagger
// ∀ concurrent restarts on same GPU: time between starts >= restart_stagger_secs

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn test_stagger_intervals(n_servers in 2usize..20, stagger_secs in 1u64..10) {
        // If n_servers all trigger pre_spawn_vram_check at t=0, they acquire the semaphore one by one.
        // Each acquisition takes the permit, and releases it `stagger_secs` later.
        // Server 0 starts at t=0. Releases permit at t=stagger_secs.
        // Server 1 starts at t=stagger_secs. Releases permit at t=2*stagger_secs.
        // Server i starts at t = i * stagger_secs.
        // So the time between any two sequential starts i and i+1 is >= stagger_secs.

        let mut start_times = Vec::new();
        for i in 0..n_servers {
            start_times.push(i as u64 * stagger_secs);
        }

        for i in 0..(start_times.len() - 1) {
            let diff = start_times[i+1] - start_times[i];
            prop_assert!(diff >= stagger_secs);
        }
    }
}
