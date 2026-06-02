//! Serves the embedded web UI at `/ui/*` with strict security response
//! headers (FRONTEND_ARCHITECTURE.md §10.3).
//!
//! Assets are embedded from `ui/build` for release builds via `rust-embed`;
//! debug builds read them from the filesystem at runtime (so `cargo build`
//! needs neither node nor a built UI). The route is mounted OUTSIDE the
//! `/api` + `/v1` middleware stack — no auth, no CORS — because the UI is
//! public static content whose isolation comes from CSP, not CORS.

use axum::{
    extract::Path,
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../ui/build"]
struct UiAssets;

/// Server-side CSP for the UI. This deliberately OMITS `default-src` /
/// `script-src` / `style-src`: SvelteKit injects a hashed `<meta>` CSP that
/// governs scripts and styles (svelte.config.js `kit.csp`). Browsers enforce
/// multiple CSPs as an INTERSECTION, so a hash-less server `script-src` here
/// would re-block Kit's inline bootstrap and the SPA would never hydrate. We
/// keep only header-appropriate directives — `frame-ancestors` (header-only),
/// plus `object-src`/`base-uri`/`form-action` defense — none of which
/// constrain script execution.
///
/// `upgrade-insecure-requests` is intentionally omitted: LlamaForge serves over
/// http on localhost/LAN by default (§10.2), and that directive would upgrade
/// subresource requests to https and break direct http access. TLS, when used,
/// is terminated at a reverse proxy that sets its own headers.
const CONTENT_SECURITY_POLICY: &str =
    "object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=(), payment=(), usb=()";

/// Router for the embedded UI. Mount it on the app root.
pub(crate) fn ui_router() -> Router {
    Router::new()
        .route("/ui", get(serve_index))
        .route("/ui/", get(serve_index))
        .route("/ui/{*path}", get(serve_asset))
}

async fn serve_index() -> Response {
    respond_with("index.html")
}

async fn serve_asset(Path(path): Path<String>) -> Response {
    // Defense-in-depth for debug filesystem mode: reject path traversal so a
    // request like `/ui/%2e%2e%2fsecret` (decoded to `../secret` by the Path
    // extractor) can't escape ui/build. In release the assets are embedded, so
    // a miss simply falls back to index.html.
    if path.split('/').any(|segment| segment == "..") {
        return respond_with("index.html");
    }
    // Serve the requested asset, or fall back to index.html for client-side
    // routes (adapter-static SPA fallback: fallback: 'index.html').
    if UiAssets::get(&path).is_some() {
        respond_with(&path)
    } else {
        respond_with("index.html")
    }
}

fn respond_with(path: &str) -> Response {
    let Some(file) = UiAssets::get(path) else {
        // The UI was not built/embedded (e.g. a debug build with no ui/build).
        return (StatusCode::NOT_FOUND, "UI assets are not available").into_response();
    };
    let content_type = HeaderValue::from_str(file.metadata.mimetype())
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    let mut response =
        (StatusCode::OK, [(header::CONTENT_TYPE, content_type)], file.data.into_owned())
            .into_response();
    apply_security_headers(response.headers_mut());
    response
}

fn apply_security_headers(headers: &mut HeaderMap) {
    headers
        .insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CONTENT_SECURITY_POLICY));
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_all_security_headers() {
        let mut headers = HeaderMap::new();
        apply_security_headers(&mut headers);

        assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
        assert_eq!(headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
        assert_eq!(
            headers.get(header::REFERRER_POLICY).unwrap(),
            "strict-origin-when-cross-origin"
        );
        assert!(headers.contains_key(HeaderName::from_static("permissions-policy")));

        let csp =
            headers.get(header::CONTENT_SECURITY_POLICY).expect("CSP present").to_str().unwrap();
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("object-src 'none'"));
        // Regression guard for the PR13 CSP bug: the server CSP must NOT govern
        // scripts/styles — SvelteKit's hashed <meta> CSP does. A server
        // script-src/default-src here would re-block Kit's inline bootstrap
        // (CSPs intersect), breaking hydration in release.
        assert!(!csp.contains("script-src"));
        assert!(!csp.contains("default-src"));
    }
}
