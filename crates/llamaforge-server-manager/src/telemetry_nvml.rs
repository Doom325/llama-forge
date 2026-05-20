use crate::telemetry::{GpuDevice, GpuTelemetry, TelemetryError};
use llamaforge_config::GpuId;
use nvml_wrapper::Nvml;

pub struct NvmlTelemetry {
    nvml: Nvml,
}

impl NvmlTelemetry {
    pub fn new() -> Result<Self, TelemetryError> {
        let nvml = Nvml::init().map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to initialize NVML: {}", e),
        })?;
        Ok(Self { nvml })
    }
}

impl GpuTelemetry for NvmlTelemetry {
    fn free_vram_bytes(&self, id: &GpuId) -> Result<u64, TelemetryError> {
        let device = self.nvml.device_by_index(id.0).map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to get device {}: {}", id.0, e),
        })?;
        let memory = device.memory_info().map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to get memory info for device {}: {}", id.0, e),
        })?;
        Ok(memory.free)
    }

    fn utilization_percent(&self, id: &GpuId) -> Result<u8, TelemetryError> {
        let device = self.nvml.device_by_index(id.0).map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to get device {}: {}", id.0, e),
        })?;
        let util = device.utilization_rates().map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to get utilization for device {}: {}", id.0, e),
        })?;
        Ok(util.gpu as u8)
    }

    fn list_devices(&self) -> Result<Vec<GpuDevice>, TelemetryError> {
        let count = self.nvml.device_count().map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to get device count: {}", e),
        })?;

        let mut devices = Vec::new();
        for i in 0..count {
            let device = self.nvml.device_by_index(i).map_err(|e| TelemetryError::FfiError {
                message: format!("Failed to get device {}: {}", i, e),
            })?;
            let name = device.name().unwrap_or_else(|_| "Unknown NVIDIA GPU".to_string());
            let memory = device.memory_info().map_err(|e| TelemetryError::FfiError {
                message: format!("Failed to get memory info for device {}: {}", i, e),
            })?;
            devices.push(GpuDevice {
                id: GpuId(i),
                name,
                total_vram_bytes: memory.total,
            });
        }
        Ok(devices)
    }
}
