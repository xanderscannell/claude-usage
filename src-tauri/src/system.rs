//! Samples CPU, memory and the first NVIDIA GPU for the System page,
//! pushed to the frontend as `stats` events.

use std::thread;
use std::time::Duration;

use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;
use serde::Serialize;
use sysinfo::{Components, System};
use tauri::{AppHandle, Emitter};

/// CPU usage is a delta between samples, so this also sets the averaging window.
const REFRESH_MS: u64 = 1500;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    name: String,
    temp: u32,
    util: u32,
    clock_mhz: u32,
    fan: u32,
    mem_used: u64,
    mem_total: u64,
    power_w: f32,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    cpu_usage: f32,
    cpu_freq_mhz: u64,
    cpu_brand: String,
    cpu_temp: Option<f32>,
    mem_total: u64,
    mem_used: u64,
    swap_total: u64,
    swap_used: u64,
    /// `None` when there is no NVIDIA GPU or NVML is unavailable.
    gpu: Option<Gpu>,
}

/// Discards bogus readings from unconnected board sensors
/// (-128°C thermistors, sensors pinned at 0°C).
fn valid_temp(c: f32) -> bool {
    c > 1.0 && c < 150.0
}

/// The most CPU-representative temperature: `k10temp Tctl` on AMD,
/// then common fallbacks.
fn cpu_temp(components: &Components) -> Option<f32> {
    let pick = |needle: &str| {
        components
            .iter()
            .find(|c| c.label().to_lowercase().contains(needle))
            .and_then(|c| c.temperature())
            .filter(|t| valid_temp(*t))
    };
    pick("tctl")
        .or_else(|| pick("tdie"))
        .or_else(|| pick("package"))
        .or_else(|| pick("composite"))
        .or_else(|| pick("cpu"))
}

fn gpu(nvml: &Nvml) -> Option<Gpu> {
    let dev = nvml.device_by_index(0).ok()?;
    let mem = dev.memory_info().ok()?;
    Some(Gpu {
        name: dev.name().unwrap_or_else(|_| "NVIDIA GPU".into()),
        temp: dev.temperature(TemperatureSensor::Gpu).unwrap_or(0),
        util: dev.utilization_rates().map(|u| u.gpu).unwrap_or(0),
        clock_mhz: dev.clock_info(Clock::Graphics).unwrap_or(0),
        fan: dev.fan_speed(0).unwrap_or(0),
        mem_used: mem.used,
        mem_total: mem.total,
        // power_usage() is in milliwatts
        power_w: dev.power_usage().map(|mw| mw as f32 / 1000.0).unwrap_or(0.0),
    })
}

fn sample(sys: &mut System, components: &mut Components, nvml: Option<&Nvml>) -> Stats {
    sys.refresh_cpu_all();
    sys.refresh_memory();
    components.refresh(true);

    let cpus = sys.cpus();
    Stats {
        cpu_usage: sys.global_cpu_usage(),
        // the fastest core is the most representative "current clock"
        cpu_freq_mhz: cpus.iter().map(|c| c.frequency()).max().unwrap_or(0),
        cpu_brand: cpus.first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
        cpu_temp: cpu_temp(components),
        mem_total: sys.total_memory(),
        mem_used: sys.used_memory(),
        swap_total: sys.total_swap(),
        swap_used: sys.used_swap(),
        gpu: nvml.and_then(gpu),
    }
}

pub fn spawn(handle: AppHandle) {
    thread::spawn(move || {
        let mut sys = System::new_all();
        let mut components = Components::new_with_refreshed_list();
        // NVML ships with the NVIDIA driver; absent elsewhere, so the GPU row hides.
        let nvml = Nvml::init().ok();
        loop {
            let _ = handle.emit("stats", sample(&mut sys, &mut components, nvml.as_ref()));
            thread::sleep(Duration::from_millis(REFRESH_MS));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_this_machine() {
        let mut sys = System::new_all();
        let mut components = Components::new_with_refreshed_list();
        let s = sample(&mut sys, &mut components, Nvml::init().ok().as_ref());
        assert!(s.mem_total > 0 && s.mem_used <= s.mem_total);
        assert!((0.0..=100.0).contains(&s.cpu_usage));
        assert!(s.cpu_temp.map_or(true, valid_temp));
    }

    #[test]
    fn rejects_unconnected_sensors() {
        assert!(!valid_temp(-128.0) && !valid_temp(0.0) && valid_temp(45.0));
    }
}
