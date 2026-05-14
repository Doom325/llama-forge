#![allow(dead_code)]
use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
};
use llamaforge_config::ConfigReader;
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct LargeBodyConcurrencyLimit {
    semaphore: Arc<Semaphore>,
    threshold_bytes: u64,
}

impl LargeBodyConcurrencyLimit {
    pub fn new(max_concurrent: usize, threshold_mb: u64) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            threshold_bytes: threshold_mb * 1024 * 1024,
        }
    }
}

pub(crate) async fn large_body_limit_middleware(
    state: axum::extract::State<LargeBodyConcurrencyLimit>,
    req: Request,
    next: Next,
) -> Result<Response, crate::error::ApiError> {
    let content_length = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|val| val.to_str().ok())
        .and_then(|val| val.parse::<u64>().ok())
        .unwrap_or(u64::MAX);

    if content_length > state.threshold_bytes {
        let _permit = state
            .semaphore
            .acquire()
            .await
            .map_err(|_| crate::error::ApiError::Internal("Semaphore closed".to_string()))?;
        Ok(next.run(req).await)
    } else {
        Ok(next.run(req).await)
    }
}

pub(crate) async fn auth_middleware(
    state: axum::extract::State<Arc<dyn ConfigReader>>,
    req: Request,
    next: Next,
) -> Result<Response, crate::error::ApiError> {
    let config = state.snapshot();
    
    if let Some(expected_key) = &config.global.api_key {
        if expected_key.is_empty() {
            return Ok(next.run(req).await);
        }

        let auth_header = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|val| val.to_str().ok());

        let is_valid = match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let token = &header[7..];
                token == expected_key
            }
            _ => false,
        };

        if !is_valid {
            // TODO: Record llamaforge_auth_failures_total metric
            return Err(crate::error::ApiError::AuthFailure);
        }
    }

    Ok(next.run(req).await)
}

pub(crate) async fn request_id_middleware(
    mut req: Request,
    next: Next,
) -> Response {
    let request_id = Uuid::now_v7();
    let request_id_str = request_id.to_string();

    req.extensions_mut().insert(llamaforge_types::RequestId::new(request_id));

    let mut response = next.run(req).await;
    
    response.headers_mut().insert(
        "x-request-id",
        axum::http::HeaderValue::from_str(&request_id_str).unwrap(),
    );
    response.headers_mut().insert(
        "x-llamaforge-request-id",
        axum::http::HeaderValue::from_str(&request_id_str).unwrap(),
    );

    response
}

pub(crate) async fn metrics_middleware(
    req: Request,
    next: Next,
) -> Response {
    let _method = req.method().clone();
    let _path = if let Some(matched_path) = req.extensions().get::<axum::extract::MatchedPath>() {
        matched_path.as_str().to_string()
    } else {
        "unknown".to_string()
    };

    let start = std::time::Instant::now();
    
    let response = next.run(req).await;
    
    let _duration = start.elapsed().as_secs_f64();
    let _status = response.status().as_u16().to_string();

    response
}