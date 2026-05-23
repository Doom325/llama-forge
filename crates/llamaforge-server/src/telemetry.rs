//! GPU telemetry port and FFI.
//! See design doc Section 2.2.

use llamaforge_core::config::GpuId;

/// A GPU device managed by the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuDevice {
    pub id: GpuId,
    pub name: String,
    pub total_vram_bytes: u64,
}

/// Errors that can occur during telemetry operations.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("Telemetry service is unavailable")]
    Unavailable,

    #[error("GPU device not found: {device:?}")]
    DeviceNotFound { device: GpuId },

    #[error("FFI call failed: {message}")]
    FfiError { message: String },
}

/// Abstract port for querying GPU telemetry data.
/// See design doc Section 2.2.
pub trait GpuTelemetry: Send + Sync + 'static {
    /// Get the available free VRAM in bytes for a specific GPU.
    fn free_vram_bytes(&self, id: &GpuId) -> Result<u64, TelemetryError>;

    /// Get the current utilization percentage (0-100) for a specific GPU.
    fn utilization_percent(&self, id: &GpuId) -> Result<u8, TelemetryError>;

    /// List all discovered GPU devices.
    fn list_devices(&self) -> Result<Vec<GpuDevice>, TelemetryError>;
}

/// A stub telemetry provider used as a fallback when no platform telemetry is available.
pub struct StubTelemetry;

impl GpuTelemetry for StubTelemetry {
    fn free_vram_bytes(&self, _id: &GpuId) -> Result<u64, TelemetryError> {
        Err(TelemetryError::Unavailable)
    }

    fn utilization_percent(&self, _id: &GpuId) -> Result<u8, TelemetryError> {
        Err(TelemetryError::Unavailable)
    }

    fn list_devices(&self) -> Result<Vec<GpuDevice>, TelemetryError> {
        Err(TelemetryError::Unavailable)
    }
}

/// Creates the appropriate telemetry backend based on configuration and available features.
pub fn create_telemetry(backend_override: Option<&str>) -> std::sync::Arc<dyn GpuTelemetry> {
    if let Some(backend) = backend_override {
        match backend {
            #[cfg(feature = "telemetry-nvml")]
            "nvml" => match crate::telemetry_nvml::NvmlTelemetry::new() {
                Ok(t) => {
                    tracing::info!("Using NVML telemetry backend");
                    return std::sync::Arc::new(t);
                }
                Err(e) => {
                    tracing::warn!("Failed to initialize NVML telemetry: {}", e);
                }
            },
            #[cfg(feature = "telemetry-amdgpu")]
            "amdgpu" => match crate::telemetry_amdgpu::AmdGpuTelemetry::new() {
                Ok(t) => {
                    tracing::info!("Using AMDGPU sysfs telemetry backend");
                    return std::sync::Arc::new(t);
                }
                Err(e) => {
                    tracing::warn!("Failed to initialize AMDGPU telemetry: {}", e);
                }
            },
            other => {
                tracing::warn!(
                    "Requested telemetry backend '{}' is unknown or not compiled in",
                    other
                );
            }
        }
    } else {
        // Auto-detect
        #[cfg(feature = "telemetry-nvml")]
        {
            if let Ok(t) = crate::telemetry_nvml::NvmlTelemetry::new() {
                tracing::info!("Auto-detected NVML telemetry backend");
                return std::sync::Arc::new(t);
            }
        }

        #[cfg(feature = "telemetry-amdgpu")]
        {
            if let Ok(t) = crate::telemetry_amdgpu::AmdGpuTelemetry::new() {
                tracing::info!("Auto-detected AMDGPU sysfs telemetry backend");
                return std::sync::Arc::new(t);
            }
        }
    }

    tracing::warn!("No GPU telemetry backend available. Falling back to StubTelemetry.");
    std::sync::Arc::new(StubTelemetry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stub_telemetry_unavailable() {
        let stub = StubTelemetry;
        let gpu = GpuId(0);

        assert!(matches!(stub.free_vram_bytes(&gpu), Err(TelemetryError::Unavailable)));

        assert!(matches!(stub.utilization_percent(&gpu), Err(TelemetryError::Unavailable)));

        assert!(matches!(stub.list_devices(), Err(TelemetryError::Unavailable)));
    }
}
