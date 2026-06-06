//! Axum handlers for the `/api/setup/*` endpoints.

use std::collections::HashSet;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use axum::extract::{Json, Query, State};
use axum::response::{IntoResponse, Response};
use llamaforge_core::config::{AppConfig, ConfigLoadError, TomlFileConfigSource};
use llamaforge_server::{
    detect_hardware, recommend, scan_binary, scan_models, BinaryScanRequest, ModelScanRequest,
    RecommendationRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::config_assembly::{assemble_config, render_toml, SaveConfigRequest, ServerConfigDraft};
use super::{SetupError, SetupState, ValidationDetail, Versioned, SETUP_API_VERSION};

/// Query params for the hardware endpoint.
#[derive(Debug, Default, Deserialize)]
pub(super) struct HardwareQuery {
    /// When true, include diagnostic fields (driver versions, etc.).
    #[serde(default)]
    diagnostics: bool,
}

/// `GET /api/setup/hardware` — detect hardware, cache it, return the summary.
pub(super) async fn hardware_handler(
    State(state): State<SetupState>,
    Query(query): Query<HardwareQuery>,
) -> Result<Response, SetupError> {
    let hardware = detect_hardware().await;
    *state.hardware_cache.write().await = Some(hardware.clone());

    let mut value = serde_json::to_value(&hardware)
        .map_err(|e| SetupError::Internal(format!("serialize hardware: {e}")))?;

    // Diagnostic fields are only surfaced under ?diagnostics=true.
    if !query.diagnostics {
        if let Some(gpus) = value.get_mut("gpus").and_then(|g| g.as_array_mut()) {
            for gpu in gpus.iter_mut() {
                if let Some(obj) = gpu.as_object_mut() {
                    obj.remove("driver_version");
                }
            }
        }
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert("version".to_string(), json!(SETUP_API_VERSION));
    }
    Ok(Json(value).into_response())
}

/// `POST /api/setup/scan-binary` — locate and probe `llama-server`.
pub(super) async fn scan_binary_handler(
    Json(request): Json<BinaryScanRequest>,
) -> Result<Versioned<llamaforge_server::BinaryScanResult>, SetupError> {
    let result = scan_binary(request).await.map_err(|e| SetupError::BadRequest(e.to_string()))?;
    Ok(Versioned(result))
}

/// `POST /api/setup/scan-models` — discover `.gguf` files.
pub(super) async fn scan_models_handler(
    Json(request): Json<ModelScanRequest>,
) -> Result<Versioned<llamaforge_server::ModelScanResult>, SetupError> {
    let result = scan_models(request).await.map_err(|e| SetupError::BadRequest(e.to_string()))?;
    Ok(Versioned(result))
}

/// `POST /api/setup/recommend` — recommend a config per model using cached
/// hardware (detecting on demand if the cache is empty).
pub(super) async fn recommend_handler(
    State(state): State<SetupState>,
    Json(request): Json<RecommendationRequest>,
) -> Result<Versioned<llamaforge_server::RecommendationResult>, SetupError> {
    if request.models.is_empty() {
        return Err(SetupError::BadRequest("at least one model is required".to_string()));
    }

    let cached = state.hardware_cache.read().await.clone();
    let hardware = match cached {
        Some(hardware) => hardware,
        None => {
            let detected = detect_hardware().await;
            *state.hardware_cache.write().await = Some(detected.clone());
            detected
        }
    };

    Ok(Versioned(recommend(&hardware, &request.models)))
}

/// `POST /api/setup/preview-config` — render the assembled TOML without writing.
#[derive(Debug, Serialize)]
pub(super) struct PreviewResponse {
    toml: String,
}

pub(super) async fn preview_config_handler(
    Json(request): Json<SaveConfigRequest>,
) -> Result<Versioned<PreviewResponse>, SetupError> {
    let existing = load_existing(Path::new(&request.path)).await;
    let active_ports = active_config_ports(&existing);
    let config = assemble_config(existing, &request).map_err(SetupError::BadRequest)?;

    let details = pre_validate(&config, &request, &active_ports).await;
    if !details.is_empty() {
        return Err(SetupError::PreValidation(details));
    }

    let toml = render_toml(&config).map_err(SetupError::Internal)?;
    Ok(Versioned(PreviewResponse { toml }))
}

/// `POST /api/setup/save-config` — assemble, pre-validate, back up, and write.
#[derive(Debug, Serialize)]
pub(super) struct SaveResponse {
    path: String,
    replaced_existing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup_path: Option<String>,
}

pub(super) async fn save_config_handler(
    State(state): State<SetupState>,
    Json(request): Json<SaveConfigRequest>,
) -> Result<Versioned<SaveResponse>, SetupError> {
    // Serialize concurrent saves so backups/writes can't interleave.
    let _guard = state.save_lock.lock().await;

    let path = PathBuf::from(&request.path);
    let existing = load_existing(&path).await;
    let active_ports = active_config_ports(&existing);
    let config = assemble_config(existing, &request).map_err(SetupError::BadRequest)?;

    // Pre-validation (Section 13.5 Step 1): collect all failures and return 422.
    let details = pre_validate(&config, &request, &active_ports).await;
    if !details.is_empty() {
        return Err(SetupError::PreValidation(details));
    }

    let rendered = render_toml(&config).map_err(SetupError::Internal)?;
    // The rendered TOML must round-trip back to a valid config (internal invariant).
    toml::from_str::<AppConfig>(&rendered)
        .map_err(|e| SetupError::Internal(format!("config failed round-trip validation: {e}")))?;

    let replaced_existing = tokio::fs::try_exists(&path).await.unwrap_or(false);
    if replaced_existing && !request.force {
        return Err(SetupError::Conflict(format!(
            "{} already exists; pass force=true to overwrite",
            path.display()
        )));
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await.map_err(map_write_error)?;
        }
    }

    // Atomic write via a temp sibling (also probes writability before backup).
    // Any failure after the temp is written cleans it up so no orphan remains.
    let tmp = sibling(&path, "tmp");
    tokio::fs::write(&tmp, rendered.as_bytes()).await.map_err(map_write_error)?;

    let backup_path = match maybe_backup(replaced_existing, &path).await {
        Ok(backup) => backup,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
    };

    if let Err(e) = tokio::fs::rename(&tmp, &path).await.map_err(map_write_error) {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }

    Ok(Versioned(SaveResponse {
        path: path.display().to_string(),
        replaced_existing,
        backup_path: backup_path.map(|p| p.display().to_string()),
    }))
}

async fn maybe_backup(replaced_existing: bool, path: &Path) -> Result<Option<PathBuf>, SetupError> {
    if replaced_existing {
        Ok(Some(backup_and_prune(path).await?))
    } else {
        Ok(None)
    }
}

/// `POST /api/setup/validate-launch` — spawn the first server, wait for its
/// health check, then kill it.
#[derive(Debug, Deserialize)]
pub(super) struct ValidateLaunchRequest {
    servers: Vec<ServerConfigDraft>,
}

#[derive(Debug, Serialize)]
pub(super) struct ValidateResponse {
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
}

/// Health-check budget for the validation launch.
const VALIDATE_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) async fn validate_launch_handler(
    State(state): State<SetupState>,
    Json(request): Json<ValidateLaunchRequest>,
) -> Result<Versioned<ValidateResponse>, SetupError> {
    let server = request
        .servers
        .first()
        .ok_or_else(|| SetupError::BadRequest("no servers to validate".to_string()))?;

    let mut command = tokio::process::Command::new(&server.binary_path);
    command
        .arg("--model")
        .arg(&server.model_path)
        .arg("--port")
        .arg(server.port.to_string())
        .arg("--n-gpu-layers")
        .arg(server.gpu_layers.to_string())
        .arg("--ctx-size")
        .arg(server.ctx_size.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(|e| {
        SetupError::BadRequest(format!("failed to spawn {}: {e}", server.binary_path))
    })?;

    let url = format!("http://127.0.0.1:{}/health", server.port);
    let client = reqwest::Client::new();
    let deadline = Instant::now() + VALIDATE_TIMEOUT;
    let mut healthy = false;
    let mut exited_early = false;

    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            exited_early = true;
            break;
        }
        if let Ok(resp) = client.get(&url).timeout(Duration::from_secs(2)).send().await {
            if resp.status().is_success() {
                healthy = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let _ = child.start_kill();
    let stderr = child
        .wait_with_output()
        .await
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
        .filter(|s| !s.trim().is_empty());

    let response = if healthy {
        // Setup is complete — let a standalone wizard server shut itself down.
        state.signal_complete();
        ValidateResponse {
            success: true,
            message: "server started and passed its health check".to_string(),
            stderr: None,
        }
    } else {
        let message = if exited_early {
            "server process exited before becoming healthy".to_string()
        } else {
            "server did not become healthy within 30s".to_string()
        };
        ValidateResponse { success: false, message, stderr }
    };
    Ok(Versioned(response))
}

// ---------------------------------------------------------------------------
// Pre-validation (Section 13.5 Step 1).
// ---------------------------------------------------------------------------

/// Ports held by the currently-active config. In reconfigure mode the on-disk
/// config IS the running instance's config, so its server ports are legitimately
/// bound and must be exempt from the system port-availability probe — otherwise
/// re-submitting an active server fails with "port already in use" (issue #273).
fn active_config_ports(existing: &Option<AppConfig>) -> HashSet<u16> {
    existing.iter().flat_map(|c| c.servers.values().map(|s| s.port)).collect()
}

/// Run the three pre-validation checks over the assembled config, collecting
/// **all** failures (not short-circuiting) into `details`:
/// 1. each server's `binary_path`/`model_path` resolves to an existing file;
/// 2. each server's port is bindable (not already in use), unless the port is
///    already held by the active config (`active_ports`);
/// 3. the config passes the core load-time validation pipeline (unique ports,
///    non-empty fields, `parallel_slots > 0` — design doc §11.6).
async fn pre_validate(
    config: &AppConfig,
    request: &SaveConfigRequest,
    active_ports: &HashSet<u16>,
) -> Vec<ValidationDetail> {
    let mut details = Vec::new();

    for server in &request.servers {
        if !is_readable_file(&server.binary_path).await {
            details.push(ValidationDetail {
                field: "binary_path".to_string(),
                error: format!("{} does not exist or is not a readable file", server.binary_path),
            });
        }
        if !is_readable_file(&server.model_path).await {
            details.push(ValidationDetail {
                field: "model_path".to_string(),
                error: format!("{} does not exist or is not a readable file", server.model_path),
            });
        }
        // A port already held by the active config (reconfigure re-submitting an
        // existing server) is not "in use" from the wizard's perspective.
        if !active_ports.contains(&server.port) && !is_port_available(server.port) {
            details.push(ValidationDetail {
                field: "port".to_string(),
                error: format!("port {} is already in use", server.port),
            });
        }
    }

    // Reuse the core load-time validation so duplicate ports and other semantic
    // rules can't slip through to `serve`. It mutates (fair-share normalization)
    // and short-circuits, so validate a clone and surface its first error.
    let mut probe = config.clone();
    if let Err(ConfigLoadError::ValidationError { message }) =
        TomlFileConfigSource::validate(&mut probe)
    {
        details.push(ValidationDetail { field: "config".to_string(), error: message });
    }

    details
}

/// Whether `path` exists and is a regular file we can stat.
async fn is_readable_file(path: &str) -> bool {
    tokio::fs::metadata(path).await.map(|m| m.is_file()).unwrap_or(false)
}

/// Whether `port` can be bound on loopback (released immediately).
fn is_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

// ---------------------------------------------------------------------------
// File helpers.
// ---------------------------------------------------------------------------

/// Maximum config backups retained.
const MAX_BACKUPS: usize = 5;

/// Load an existing config for merge; `None` if absent or unparseable (a
/// corrupt file is safely overwritten after the backup step).
async fn load_existing(path: &Path) -> Option<AppConfig> {
    let text = tokio::fs::read_to_string(path).await.ok()?;
    toml::from_str(&text).ok()
}

/// Build a sibling path `<file>.<suffix>` in the same directory.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "llamaforge.toml".to_string());
    path.with_file_name(format!("{name}.{suffix}"))
}

/// Copy the existing config to a timestamped backup and prune to [`MAX_BACKUPS`].
async fn backup_and_prune(path: &Path) -> Result<PathBuf, SetupError> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3f");
    let backup = sibling(path, &format!("backup.{stamp}"));
    tokio::fs::copy(path, &backup).await.map_err(map_write_error)?;
    prune_backups(path).await;
    Ok(backup)
}

/// Remove all but the newest [`MAX_BACKUPS`] `<file>.backup.*` siblings.
async fn prune_backups(path: &Path) {
    let Some(parent) = path.parent() else { return };
    let Some(file_name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return;
    };
    let prefix = format!("{file_name}.backup.");

    let parent = if parent.as_os_str().is_empty() { Path::new(".") } else { parent };
    let Ok(mut entries) = tokio::fs::read_dir(parent).await else {
        return;
    };

    let mut backups: Vec<PathBuf> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            backups.push(entry.path());
        }
    }
    // Timestamped names sort lexicographically by age; drop the oldest.
    backups.sort();
    if backups.len() > MAX_BACKUPS {
        for stale in &backups[..backups.len() - MAX_BACKUPS] {
            let _ = tokio::fs::remove_file(stale).await;
        }
    }
}

/// Map a filesystem write error to the right HTTP status: permission → 403.
fn map_write_error(err: std::io::Error) -> SetupError {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        SetupError::Forbidden(format!("path is not writable: {err}"))
    } else {
        SetupError::Internal(format!("write failed: {err}"))
    }
}
