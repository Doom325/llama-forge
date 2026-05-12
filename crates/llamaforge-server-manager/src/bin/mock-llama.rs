#![cfg(feature = "test-utils")]

use axum::{response::IntoResponse, routing::get, Json, Router};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;

#[derive(Default)]
struct MockState {
    health_calls: AtomicU32,
    slots_calls: AtomicU32,
}

#[tokio::main]
async fn main() {
    let mut mock_port = 8080;
    let mut health_fail_after: Option<u32> = None;
    let mut hang_after: Option<u32> = None;
    let mut exit_after_secs: Option<u64> = None;
    let mut model_alias = "mock".to_string();
    let mut tps = 10.0;

    // Parse --port and --alias from args
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(p) = args.next() {
                    mock_port = p.parse().unwrap();
                }
            }
            "--alias" => {
                if let Some(a) = args.next() {
                    // For tests, we pack mock config into the alias field separated by commas
                    // Format: "alias_name,health_fail_after=1,tps=0.1"
                    let parts: Vec<&str> = a.split(',').collect();
                    if !parts.is_empty() {
                        model_alias = parts[0].to_string();
                    }
                    for part in parts.iter().skip(1) {
                        let kv: Vec<&str> = part.split('=').collect();
                        if kv.len() == 2 {
                            match kv[0] {
                                "health_fail_after" => health_fail_after = kv[1].parse().ok(),
                                "hang_after" => hang_after = kv[1].parse().ok(),
                                "exit_after_secs" => exit_after_secs = kv[1].parse().ok(),
                                "tps" => tps = kv[1].parse().ok().unwrap_or(10.0),
                                _ => {}
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let port = mock_port;
    let state = Arc::new(MockState::default());

    if let Some(secs) = exit_after_secs {
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            std::process::exit(0);
        });
    }

    let state_health = state.clone();
    let model_alias_clone = model_alias.clone();
    let health = get(move || {
        let state = state_health.clone();
        let alias = model_alias_clone.clone();
        async move {
            let count = state.health_calls.fetch_add(1, Ordering::Relaxed);
            if let Some(fail_after) = health_fail_after {
                if count >= fail_after {
                    return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
            Json(json!({"status": "ok", "model": alias})).into_response()
        }
    });

    let state_slots = state.clone();
    let slots = get(move || {
        let state = state_slots.clone();
        async move {
            let count = state.slots_calls.fetch_add(1, Ordering::Relaxed);
            let n_decoded = if let Some(hang_after) = hang_after {
                if count >= hang_after {
                    hang_after * 10
                } else {
                    count * 10
                }
            } else {
                count * 10
            };

            Json(json!([
                {
                    "id": 0,
                    "is_processing": true,
                    "n_past": 1000,
                    "n_decoded": n_decoded,
                    "prompt_hash": 12345
                }
            ]))
            .into_response()
        }
    });

    let state_metrics = state.clone();
    let metrics = get(move || {
        let state = state_metrics.clone();
        async move {
            let count = state.slots_calls.load(Ordering::Relaxed);
            let decoded = (count as f64 * tps) as u32; // Simplified
            format!("llamacpp:n_decode_total {}\n", decoded)
        }
    });

    let app =
        Router::new().route("/health", health).route("/slots", slots).route("/metrics", metrics);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
