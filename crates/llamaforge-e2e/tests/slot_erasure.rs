//! End-to-end test: `ReqwestClientStore` as `SlotErasure` actually
//! POSTs to `/slots/{id}?action=erase` on the registered server.

use llamaforge_core::types::ports::SlotErasure;
use llamaforge_core::types::{ServerId, SlotId};
use llamaforge_api::dispatch::client::ReqwestClientStore;
use llamaforge_e2e::Harness;
use std::sync::Arc;

#[tokio::test]
async fn real_slot_erasure_hits_mock_llama() {
    let h = Harness::start().await.expect("harness start");
    let store = Arc::new(ReqwestClientStore::new());

    // Register a single server pointing at the live mock-llama.
    let server_id = ServerId::new(uuid::Uuid::now_v7());
    let cfg = llamaforge_core::config::ServerConfig {
        port: h.mock.addr.port(),
        parallel_slots: 1,
        ..Default::default()
    };
    store.add_server(server_id, &cfg).expect("add_server");

    // First, populate a slot via a completion against mock-llama so
    // /slots returns a populated entry. Then call erase_slot through
    // the SlotErasure port and confirm the mock observed it.
    let client = reqwest::Client::new();
    let _ = client
        .post(format!("{}/completion", h.mock_base_url()))
        .json(&serde_json::json!({"prompt":"x","stream":false,"id_slot":0}))
        .send()
        .await
        .unwrap();

    assert_eq!(h.mock.erase_call_count(), 0);

    store.erase_slot(&server_id, &SlotId::new(0)).await.expect("erase_slot ok");

    assert_eq!(h.mock.erase_call_count(), 1);

    llamaforge_e2e::shutdown(h).await;
}

#[tokio::test]
async fn erase_slot_unregistered_server_returns_unreachable() {
    let store = ReqwestClientStore::new();
    let server_id = ServerId::new(uuid::Uuid::now_v7());

    let err = store
        .erase_slot(&server_id, &SlotId::new(0))
        .await
        .expect_err("unregistered server should fail");

    match err {
        llamaforge_core::types::ports::SlotError::ServerUnreachable { .. } => {}
        other => panic!("expected ServerUnreachable, got {:?}", other),
    }
}
