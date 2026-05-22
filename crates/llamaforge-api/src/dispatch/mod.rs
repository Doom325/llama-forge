//! Dispatch & Proxy: streams completions from the chosen backend to
//! the client; owns SSE keepalive, cancellation, and timeout policy.
//!
//! Implements design.md §5.1 (Transport-side dispatch surface) and
//! §8 (Request Lifecycle). Component spec:
//! `docs/component-designs/06_DISPATCH_AND_PROXY.md`.

pub mod client;
pub mod dispatcher;
pub mod error;
pub mod proxy;
pub mod slot_map;
pub mod token;
pub mod tracker;
