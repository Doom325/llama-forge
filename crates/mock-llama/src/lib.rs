//! In-process double for `llama-server`.
//!
//! Exposes the four endpoints LlamaForge talks to: `/health`, `/slots`,
//! `/metrics`, and `/completion`. Slot state is tracked so the
//! `POST /slots/{id}?action=erase` reconciliation contract can be
//! verified by integration tests.
//!
//! Used both by the standalone `mock-llama` binary (run as a
//! subprocess in legacy tests) and by `llamaforge-e2e`, which spawns
//! the server in-process on an ephemeral port.

use axum::{
    extract::{Path, Query, State},
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_util::stream;
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

/// Runtime-tunable behaviour for a `MockLlama`.
#[derive(Debug, Clone)]
pub struct MockConfig {
    pub model_alias: String,
    /// Number of token chunks the `/completion` endpoint emits per
    /// request before issuing `data: [DONE]`. Default 3.
    pub completion_chunks: u32,
    /// Per-chunk delay, simulating decode time. Default 1ms.
    pub chunk_delay: Duration,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            model_alias: "mock".to_string(),
            completion_chunks: 3,
            chunk_delay: Duration::from_millis(1),
        }
    }
}

#[derive(Default)]
struct MockState {
    health_calls: AtomicU32,
    completions: AtomicU32,
    /// Per-slot `(is_processing, n_decoded)`. Indexed by slot id.
    slots: Mutex<HashMap<u32, SlotState>>,
    erase_calls: AtomicU32,
}

#[derive(Default, Clone, Copy)]
struct SlotState {
    is_processing: bool,
    n_decoded: u32,
}

/// Spawned mock-llama instance bound to an ephemeral port.
pub struct MockLlama {
    pub addr: std::net::SocketAddr,
    pub config: MockConfig,
    state: Arc<MockState>,
    handle: tokio::task::JoinHandle<()>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl MockLlama {
    /// Bind on an ephemeral local port and start serving.
    pub async fn start(config: MockConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(MockState::default());
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let app = router(state.clone(), config.clone());
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        Ok(Self { addr, config, state, handle, shutdown_tx })
    }

    /// Returns `http://127.0.0.1:<port>` — the base URL clients should use.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Number of times `POST /slots/{id}?action=erase` has been called.
    pub fn erase_call_count(&self) -> u32 {
        self.state.erase_calls.load(Ordering::Relaxed)
    }

    /// Shut down the server gracefully and join the task.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.handle.await;
    }
}

fn router(state: Arc<MockState>, config: MockConfig) -> Router {
    let app_state = AppState { state, config: Arc::new(config) };
    Router::new()
        .route("/health", get(health))
        .route("/slots", get(slots_get))
        .route("/slots/{id}", post(slots_post))
        .route("/metrics", get(metrics))
        .route("/completion", post(completion))
        .with_state(app_state)
}

#[derive(Clone)]
struct AppState {
    state: Arc<MockState>,
    config: Arc<MockConfig>,
}

async fn health(State(s): State<AppState>) -> impl IntoResponse {
    s.state.health_calls.fetch_add(1, Ordering::Relaxed);
    Json(serde_json::json!({"status": "ok", "model": s.config.model_alias}))
}

async fn slots_get(State(s): State<AppState>) -> impl IntoResponse {
    let slots = s.state.slots.lock();
    let payload: Vec<serde_json::Value> = slots
        .iter()
        .map(|(id, st)| {
            serde_json::json!({
                "id": id,
                "is_processing": st.is_processing,
                "n_past": 0,
                "n_decoded": st.n_decoded,
                "prompt_hash": 0,
            })
        })
        .collect();
    Json(payload).into_response()
}

#[derive(Debug, Deserialize)]
struct SlotsPostQuery {
    action: Option<String>,
}

async fn slots_post(
    State(s): State<AppState>,
    Path(id): Path<u32>,
    Query(q): Query<SlotsPostQuery>,
) -> impl IntoResponse {
    match q.action.as_deref() {
        Some("erase") => {
            s.state.erase_calls.fetch_add(1, Ordering::Relaxed);
            let mut slots = s.state.slots.lock();
            slots.remove(&id);
            axum::http::StatusCode::OK.into_response()
        }
        _ => axum::http::StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn metrics(State(s): State<AppState>) -> impl IntoResponse {
    let n = s.state.completions.load(Ordering::Relaxed);
    format!("llamacpp:n_decode_total {}\n", n)
}

#[derive(Debug, Deserialize)]
struct CompletionRequest {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    n_predict: Option<u32>,
    #[serde(default)]
    id_slot: Option<i32>,
}

async fn completion(
    State(s): State<AppState>,
    Json(req): Json<CompletionRequest>,
) -> impl IntoResponse {
    let count = s.state.completions.fetch_add(1, Ordering::Relaxed);
    let _ = req.prompt; // silence unused (we don't echo the prompt)
    let slot_id = req.id_slot.unwrap_or(count as i32).max(0) as u32;

    {
        let mut slots = s.state.slots.lock();
        slots.insert(slot_id, SlotState { is_processing: true, n_decoded: 0 });
    }

    let n_chunks =
        req.n_predict.unwrap_or(s.config.completion_chunks).min(s.config.completion_chunks);
    let delay = s.config.chunk_delay;
    let stream_state = s.state.clone();

    if req.stream {
        let body = async_stream_stream(n_chunks, delay, stream_state, slot_id);
        Sse::new(body).into_response()
    } else {
        // Synthesize the same chunk sequence inline for the non-streaming path.
        let mut text = String::new();
        for i in 0..n_chunks {
            tokio::time::sleep(delay).await;
            text.push_str(&format!("tok{} ", i));
            let mut slots = stream_state.slots.lock();
            if let Some(s) = slots.get_mut(&slot_id) {
                s.n_decoded = i + 1;
            }
        }
        {
            let mut slots = stream_state.slots.lock();
            if let Some(s) = slots.get_mut(&slot_id) {
                s.is_processing = false;
            }
        }
        Json(serde_json::json!({"content": text, "stop": true})).into_response()
    }
}

/// Per-request SSE chunk stream emitting `data: {…}` lines followed by `data: [DONE]`.
fn async_stream_stream(
    n_chunks: u32,
    delay: Duration,
    state: Arc<MockState>,
    slot_id: u32,
) -> impl futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static {
    stream::unfold(0u32, move |i| {
        let state = state.clone();
        async move {
            if i < n_chunks {
                tokio::time::sleep(delay).await;
                {
                    let mut slots = state.slots.lock();
                    if let Some(s) = slots.get_mut(&slot_id) {
                        s.n_decoded = i + 1;
                    }
                }
                let payload = serde_json::json!({
                    "content": format!("tok{} ", i),
                    "stop": false,
                })
                .to_string();
                Some((Ok::<_, Infallible>(Event::default().data(payload)), i + 1))
            } else if i == n_chunks {
                let payload = serde_json::json!({
                    "content": "",
                    "stop": true,
                })
                .to_string();
                // Mark the slot as no longer processing.
                {
                    let mut slots = state.slots.lock();
                    if let Some(s) = slots.get_mut(&slot_id) {
                        s.is_processing = false;
                    }
                }
                Some((Ok(Event::default().data(payload)), i + 1))
            } else if i == n_chunks + 1 {
                Some((Ok(Event::default().data("[DONE]")), i + 1))
            } else {
                None
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_responds_ok() {
        let mock = MockLlama::start(MockConfig::default()).await.unwrap();
        let res = reqwest::get(format!("{}/health", mock.base_url())).await.unwrap();
        assert_eq!(res.status(), 200);
        mock.shutdown().await;
    }

    #[tokio::test]
    async fn slot_erase_endpoint_increments_count() {
        let mock = MockLlama::start(MockConfig::default()).await.unwrap();
        let client = reqwest::Client::new();
        let res =
            client.post(format!("{}/slots/0?action=erase", mock.base_url())).send().await.unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(mock.erase_call_count(), 1);
        mock.shutdown().await;
    }
}
