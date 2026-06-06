//! Server-Sent Events hub for the dashboard live feed (#290).
//!
//! `GET /api/events` streams the named SSE events the web UI's `LiveDataStore`
//! consumes (`server-update`, `model-update`, `heartbeat`). Each carries a
//! monotonic `id:`, so a reconnecting client replays missed events via
//! `?last_event_id=<id>` (EventSource also sends the `Last-Event-ID` header, but
//! the frontend appends the query param explicitly — we honour the param).
//!
//! Producers poll the existing `ServerControl` / `ServerStateView` ports on a
//! short interval and publish only when the projected snapshot changed
//! (push-on-change), so no new domain subscription surface is required. Live
//! per-server `metrics-update` and `queue-update` pushes are intentionally not
//! emitted yet (the queue list endpoint is deferred and per-server metrics have
//! no change signal); the frontend still refreshes those via TanStack polling.
//! Heartbeats keep the client's connection-health timer alive and are streamed
//! live only — never retained for replay.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use futures_util::stream::{self, Stream, StreamExt};
use llamaforge_core::routing::types::ServerStateView;
use llamaforge_core::types::traits::ServerControl;
use serde::Deserialize;
use tokio::sync::{broadcast, watch};

use super::management::{model_infos_from_snapshots, server_summary_to_dto};

/// SSE event names the frontend `LiveDataStore` registers listeners for.
const EV_SERVER_UPDATE: &str = "server-update";
const EV_MODEL_UPDATE: &str = "model-update";
const EV_HEARTBEAT: &str = "heartbeat";

/// Recent replayable events retained for Last-Event-ID replay.
const REPLAY_CAPACITY: usize = 256;
/// Broadcast backlog; a client lagging past this is dropped (and reconnects).
const BROADCAST_CAPACITY: usize = 256;
/// Heartbeat cadence. The client treats ~30s of silence (two missed beats) as a
/// disconnect, so 15s leaves one beat of slack.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
/// Poll cadence for the change-detecting server/model producers.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct HubEvent {
    id: u64,
    name: &'static str,
    data: String,
}

/// Fan-out hub: producers `publish`, each `/api/events` connection subscribes.
/// Replayable events are also kept in a bounded ring for Last-Event-ID replay.
pub struct SseHub {
    tx: broadcast::Sender<HubEvent>,
    next_id: AtomicU64,
    replay: Mutex<VecDeque<HubEvent>>,
}

impl SseHub {
    pub fn new() -> Arc<Self> {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Arc::new(Self { tx, next_id: AtomicU64::new(1), replay: Mutex::new(VecDeque::new()) })
    }

    fn publish(&self, name: &'static str, data: String, replayable: bool) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let ev = HubEvent { id, name, data };
        if replayable {
            let mut buf = self.replay.lock().expect("sse replay lock poisoned");
            buf.push_back(ev.clone());
            while buf.len() > REPLAY_CAPACITY {
                buf.pop_front();
            }
        }
        // `Err` only means there are no subscribers right now — not a failure.
        let _ = self.tx.send(ev);
    }

    fn replay_since(&self, last_id: u64) -> Vec<HubEvent> {
        self.replay
            .lock()
            .expect("sse replay lock poisoned")
            .iter()
            .filter(|e| e.id > last_id)
            .cloned()
            .collect()
    }

    fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.tx.subscribe()
    }
}

#[derive(Deserialize, Default)]
struct EventsQuery {
    #[serde(default)]
    last_event_id: Option<String>,
}

fn to_sse_event(ev: HubEvent) -> Event {
    Event::default().id(ev.id.to_string()).event(ev.name).data(ev.data)
}

/// `GET /api/events` — replay any buffered events newer than `last_event_id`,
/// then stream live events. Lagged subscribers skip the dropped span (the next
/// push-on-change re-syncs them); a closed channel ends the stream.
async fn events_handler(
    State(hub): State<Arc<SseHub>>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = hub.subscribe();
    let last_id = query.last_event_id.as_deref().and_then(|s| s.parse::<u64>().ok());
    let backlog = last_id.map(|id| hub.replay_since(id)).unwrap_or_default();

    let live = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => return Some((ev, rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let merged = stream::iter(backlog).chain(live).map(|ev| Ok(to_sse_event(ev)));
    Sse::new(merged).keep_alive(KeepAlive::default())
}

/// The `/api/events` sub-router with the hub as its own state, ready to merge
/// into the API router so it inherits the shared middleware stack.
pub fn events_router(hub: Arc<SseHub>) -> Router {
    Router::new().route("/api/events", get(events_handler)).with_state(hub)
}

/// Spawn the background producers: a heartbeat ticker and a change-detecting
/// poller for `server-update` / `model-update`. Both stop when `shutdown_rx`
/// fires.
pub fn spawn_producers(
    hub: Arc<SseHub>,
    server_control: Arc<dyn ServerControl>,
    server_state_view: Arc<dyn ServerStateView>,
    shutdown_rx: watch::Receiver<()>,
) {
    spawn_heartbeat(hub.clone(), shutdown_rx.clone());
    spawn_change_poller(hub, server_control, server_state_view, shutdown_rx);
}

fn spawn_heartbeat(hub: Arc<SseHub>, mut shutdown_rx: watch::Receiver<()>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => hub.publish(EV_HEARTBEAT, "{}".to_string(), false),
                _ = shutdown_rx.changed() => break,
            }
        }
    });
}

fn spawn_change_poller(
    hub: Arc<SseHub>,
    server_control: Arc<dyn ServerControl>,
    server_state_view: Arc<dyn ServerStateView>,
    mut shutdown_rx: watch::Receiver<()>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Last published JSON per server id, and the last models-list JSON, so we
        // only push when something actually changed.
        let mut last_servers: HashMap<String, String> = HashMap::new();
        let mut last_models: Option<String> = None;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    poll_once(&hub, &*server_control, &*server_state_view, &mut last_servers, &mut last_models).await;
                }
                _ = shutdown_rx.changed() => break,
            }
        }
    });
}

/// One poll pass: diff each server's `ServerState` and the models list against
/// the last published JSON, publishing only the deltas.
async fn poll_once(
    hub: &SseHub,
    server_control: &dyn ServerControl,
    server_state_view: &dyn ServerStateView,
    last_servers: &mut HashMap<String, String>,
    last_models: &mut Option<String>,
) {
    if let Ok(summaries) = server_control.list_servers().await {
        for summary in summaries {
            let dto = server_summary_to_dto(summary);
            if let Ok(json) = serde_json::to_string(&dto) {
                if last_servers.get(&dto.id) != Some(&json) {
                    last_servers.insert(dto.id.clone(), json.clone());
                    hub.publish(EV_SERVER_UPDATE, json, true);
                }
            }
        }
    }

    let models = model_infos_from_snapshots(server_state_view.all_servers());
    if let Ok(json) = serde_json::to_string(&models) {
        if last_models.as_ref() != Some(&json) {
            *last_models = Some(json.clone());
            hub.publish(EV_MODEL_UPDATE, json, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_since_returns_only_newer_replayable_events() {
        let hub = SseHub::new();
        hub.publish(EV_SERVER_UPDATE, "a".to_string(), true); // id 1
        hub.publish(EV_SERVER_UPDATE, "b".to_string(), true); // id 2
        hub.publish(EV_SERVER_UPDATE, "c".to_string(), true); // id 3

        let since_1: Vec<_> = hub.replay_since(1).into_iter().map(|e| e.data).collect();
        assert_eq!(since_1, vec!["b".to_string(), "c".to_string()]);
        assert!(hub.replay_since(3).is_empty());
        // No filter (fresh connect) replays nothing because last_id is absent;
        // replay_since(0) returns the whole buffer.
        assert_eq!(hub.replay_since(0).len(), 3);
    }

    #[test]
    fn heartbeats_are_not_retained_for_replay() {
        let hub = SseHub::new();
        hub.publish(EV_SERVER_UPDATE, "a".to_string(), true); // id 1, replayable
        hub.publish(EV_HEARTBEAT, "{}".to_string(), false); // id 2, transient
        hub.publish(EV_SERVER_UPDATE, "b".to_string(), true); // id 3, replayable

        let names: Vec<_> = hub.replay_since(0).into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec![EV_SERVER_UPDATE, EV_SERVER_UPDATE]);
        // ids stay monotonic across the (non-retained) heartbeat.
        let ids: Vec<_> = hub.replay_since(0).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn replay_buffer_is_bounded() {
        let hub = SseHub::new();
        for i in 0..(REPLAY_CAPACITY + 50) {
            hub.publish(EV_SERVER_UPDATE, i.to_string(), true);
        }
        assert_eq!(hub.replay_since(0).len(), REPLAY_CAPACITY);
    }

    #[tokio::test]
    async fn publish_reaches_a_live_subscriber() {
        let hub = SseHub::new();
        let mut rx = hub.subscribe();
        hub.publish(EV_MODEL_UPDATE, "[]".to_string(), true);
        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.name, EV_MODEL_UPDATE);
        assert_eq!(ev.data, "[]");
        assert_eq!(ev.id, 1);
    }

    #[tokio::test]
    async fn handler_replays_backlog_then_streams_live() {
        use axum::extract::{Query, State};
        use axum::response::IntoResponse;

        let hub = SseHub::new();
        hub.publish(EV_SERVER_UPDATE, "first".to_string(), true); // id 1
        hub.publish(EV_SERVER_UPDATE, "second".to_string(), true); // id 2
        hub.publish(EV_SERVER_UPDATE, "third".to_string(), true); // id 3

        // Reconnect from after id 1 → expect the replay of ids 2 and 3, then a
        // live event published after the handler subscribed.
        let resp = events_handler(
            State(hub.clone()),
            Query(EventsQuery { last_event_id: Some("1".to_string()) }),
        )
        .await
        .into_response();

        let mut stream = resp.into_body().into_data_stream();
        let mut seen = String::new();

        // Drain the immediately-ready replay frames.
        while !(seen.contains("second") && seen.contains("third")) {
            let frame = tokio::time::timeout(Duration::from_secs(2), stream.next())
                .await
                .expect("timed out waiting for replay");
            let bytes = frame.expect("stream ended early").expect("frame error");
            seen.push_str(&String::from_utf8_lossy(&bytes));
        }
        assert!(!seen.contains("first"), "id 1 must not be replayed (last_event_id=1)");
        // SSE framing carries the id and event name.
        assert!(seen.contains("event:server-update") || seen.contains("event: server-update"));
        assert!(seen.contains("id:2") || seen.contains("id: 2"));

        // A live event published now must also reach the open stream.
        hub.publish(EV_MODEL_UPDATE, "live".to_string(), true);
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(2), stream.next())
                .await
                .expect("timed out waiting for live event");
            let bytes = frame.expect("stream ended early").expect("frame error");
            seen.push_str(&String::from_utf8_lossy(&bytes));
            if seen.contains("live") {
                break;
            }
        }
    }
}
