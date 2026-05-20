use crate::telemetry::{GpuDevice, GpuTelemetry, TelemetryError};
use llamaforge_config::GpuId;
use std::fs;
use std::path::{Path, PathBuf};

pub struct AmdGpuTelemetry {
    // Mapping from GpuId (0, 1, 2...) to the sysfs path (e.g., /sys/class/drm/card0/device)
    devices: Vec<PathBuf>,
}

impl AmdGpuTelemetry {
    pub fn new() -> Result<Self, TelemetryError> {
        let drm_path = Path::new("/sys/class/drm");
        if !drm_path.exists() {
            return Err(TelemetryError::Unavailable);
        }

        let mut devices = Vec::new();
        let entries = fs::read_dir(drm_path).map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to read drm directory: {}", e),
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if file_name.starts_with("card") && !file_name.contains('-') {
                let device_path = path.join("device");
                // Check if it's an AMD GPU by looking for vendor 0x1002 (AMD)
                if let Ok(vendor) = fs::read_to_string(device_path.join("vendor")) {
                    if vendor.trim() == "0x1002" {
                        devices.push(device_path);
                    }
                }
            }
        }

        // Sort by path string to ensure consistent ordering
        devices.sort();

        if devices.is_empty() {
            return Err(TelemetryError::Unavailable);
        }

        Ok(Self { devices })
    }
}

impl GpuTelemetry for AmdGpuTelemetry {
    fn free_vram_bytes(&self, id: &GpuId) -> Result<u64, TelemetryError> {
        let device_path = self
            .devices
            .get(id.0 as usize)
            .ok_or(TelemetryError::DeviceNotFound { device: id.clone() })?;

        let total_str =
            fs::read_to_string(device_path.join("mem_info_vram_total")).map_err(|e| {
                TelemetryError::FfiError { message: format!("Failed to read vram_total: {}", e) }
            })?;
        let used_str = fs::read_to_string(device_path.join("mem_info_vram_used")).map_err(|e| {
            TelemetryError::FfiError { message: format!("Failed to read vram_used: {}", e) }
        })?;

        let total: u64 = total_str.trim().parse().map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to parse vram_total: {}", e),
        })?;
        let used: u64 = used_str.trim().parse().map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to parse vram_used: {}", e),
        })?;

        Ok(total.saturating_sub(used))
    }

    fn utilization_percent(&self, id: &GpuId) -> Result<u8, TelemetryError> {
        let device_path = self
            .devices
            .get(id.0 as usize)
            .ok_or(TelemetryError::DeviceNotFound { device: id.clone() })?;

        let busy_str = fs::read_to_string(device_path.join("gpu_busy_percent")).map_err(|e| {
            TelemetryError::FfiError { message: format!("Failed to read gpu_busy_percent: {}", e) }
        })?;

        let busy: u8 = busy_str.trim().parse().map_err(|e| TelemetryError::FfiError {
            message: format!("Failed to parse gpu_busy_percent: {}", e),
        })?;
        Ok(busy)
    }

    fn list_devices(&self) -> Result<Vec<GpuDevice>, TelemetryError> {
        let mut result = Vec::new();
        for (i, device_path) in self.devices.iter().enumerate() {
            let total_str =
                fs::read_to_string(device_path.join("mem_info_vram_total")).map_err(|e| {
                    TelemetryError::FfiError {
                        message: format!("Failed to read vram_total for device {}: {}", i, e),
                    }
                })?;
            let total_vram_bytes: u64 =
                total_str.trim().parse().map_err(|e| TelemetryError::FfiError {
                    message: format!("Failed to parse vram_total for device {}: {}", i, e),
                })?;

            // Attempt to get a friendly name, fallback to "AMD GPU"
            // There isn't a direct standard sysfs file for the friendly product name in amdgpu
            // without pci.ids, but sometimes `product_name` exists on newer kernels,
            // or we can just use a generic name.
            let name = "AMD GPU".to_string();

            result.push(GpuDevice { id: GpuId(i as u32), name, total_vram_bytes });
        }
        Ok(result)
    }
}
