//! GPU telemetry port and FFI.
//! See design doc Section 2.2.

use llamaforge_config::GpuId;

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

    #[error("GPU device not found: {id:?}")]
    DeviceNotFound { id: GpuId },

    #[error("FFI call failed: {message}")]
    FfiError { message: String },
}

/// Abstract port for querying GPU telemetry data.
/// See design doc Section 2.2.
#[async_trait::async_trait]
pub trait GpuTelemetry: Send + Sync + 'static {
    /// Get the available free VRAM in bytes for a specific GPU.
    async fn free_vram_bytes(&self, id: &GpuId) -> Result<u64, TelemetryError>;

    /// Get the current utilization percentage (0-100) for a specific GPU.
    async fn utilization_percent(&self, id: &GpuId) -> Result<u8, TelemetryError>;

    /// List all discovered GPU devices.
    async fn list_devices(&self) -> Result<Vec<GpuDevice>, TelemetryError>;
}

/// A stub telemetry provider used as a fallback when no platform telemetry is available.
pub struct StubTelemetry;

#[async_trait::async_trait]
impl GpuTelemetry for StubTelemetry {
    async fn free_vram_bytes(&self, _id: &GpuId) -> Result<u64, TelemetryError> {
        Err(TelemetryError::Unavailable)
    }

    async fn utilization_percent(&self, _id: &GpuId) -> Result<u8, TelemetryError> {
        Err(TelemetryError::Unavailable)
    }

    async fn list_devices(&self) -> Result<Vec<GpuDevice>, TelemetryError> {
        Err(TelemetryError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stub_telemetry_unavailable() {
        let stub = StubTelemetry;
        let gpu = GpuId(0);

        assert!(matches!(stub.free_vram_bytes(&gpu).await, Err(TelemetryError::Unavailable)));

        assert!(matches!(stub.utilization_percent(&gpu).await, Err(TelemetryError::Unavailable)));

        assert!(matches!(stub.list_devices().await, Err(TelemetryError::Unavailable)));
    }
}
