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

/// Live CPU speed as a percentage of base clock, from the Windows
/// `% Processor Performance` counter (over 100 while boosting). sysinfo
/// only reports the base clock on Windows, so this is what makes it move.
#[cfg(windows)]
mod perf {
    use windows::core::w;
    use windows::Win32::System::Performance::*;

    /// From pdh.h; missing from the windows crate. Without it PDH caps
    /// percentages at 100, which would hide every boost.
    const PDH_FMT_NOCAP100: u32 = 0x8000;

    pub struct CpuPerf {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
    }

    impl CpuPerf {
        pub fn new() -> Option<Self> {
            let mut query = PDH_HQUERY::default();
            unsafe {
                if PdhOpenQueryW(None, 0, &mut query) != 0 {
                    return None;
                }
                // Owned from here, so an early return closes the query.
                let mut me = Self { query, counter: PDH_HCOUNTER::default() };
                let path = w!("\\Processor Information(_Total)\\% Processor Performance");
                if PdhAddEnglishCounterW(query, path, 0, &mut me.counter) != 0 {
                    return None;
                }
                // A rate counter needs two collections; this is the first.
                PdhCollectQueryData(query);
                Some(me)
            }
        }

        pub fn percent(&self) -> Option<f64> {
            let mut value = PDH_FMT_COUNTERVALUE::default();
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
                let fmt = PDH_FMT(PDH_FMT_DOUBLE.0 | PDH_FMT_NOCAP100);
                if PdhGetFormattedCounterValue(self.counter, fmt, None, &mut value) != 0
                    || value.CStatus != PDH_CSTATUS_VALID_DATA
                {
                    return None;
                }
                Some(value.Anonymous.doubleValue).filter(|p| *p > 0.0)
            }
        }
    }

    impl Drop for CpuPerf {
        fn drop(&mut self) {
            unsafe { PdhCloseQuery(self.query) };
        }
    }
}

#[cfg(not(windows))]
mod perf {
    pub struct CpuPerf;
    impl CpuPerf {
        pub fn new() -> Option<Self> { None }
        pub fn percent(&self) -> Option<f64> { None }
    }
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

/// LibreHardwareMonitor's web server, when it is running. It reads the CPU's
/// own sensor through a driver, which Windows otherwise keeps behind admin.
const LHM_URL: &str = "http://localhost:8085/data.json";

fn lhm_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        // localhost must never be routed through a system proxy
        .try_proxy_from_env(false)
        .timeout(Duration::from_millis(500))
        .build()
}

fn lhm_cpu_temp(agent: &ureq::Agent) -> Option<f32> {
    let tree: serde_json::Value = agent.get(LHM_URL).call().ok()?.into_json().ok()?;
    pick_lhm_cpu_temp(&tree)
}

/// Finds the CPU temperature in LibreHardwareMonitor's sensor tree,
/// preferring what `cpu_temp` does: Tctl on AMD, Package on Intel.
fn pick_lhm_cpu_temp(tree: &serde_json::Value) -> Option<f32> {
    let mut temps = Vec::new();
    collect_cpu_temps(tree, &mut temps);
    let pick = |needle: &str| {
        temps.iter().find(|(label, _)| label.to_lowercase().contains(needle)).map(|(_, c)| *c)
    };
    pick("tctl").or_else(|| pick("package")).or_else(|| temps.first().map(|(_, c)| *c))
}

fn collect_cpu_temps(node: &serde_json::Value, out: &mut Vec<(String, f32)>) {
    let id = node["SensorId"].as_str().unwrap_or("");
    if node["Type"] == "Temperature" && (id.starts_with("/amdcpu/") || id.starts_with("/intelcpu/")) {
        // Values arrive formatted, e.g. "48.9 °C", with a comma in some locales.
        let celsius = node["Value"]
            .as_str()
            .and_then(|v| v.split_whitespace().next())
            .and_then(|n| n.replace(',', ".").parse().ok())
            .filter(|c| valid_temp(*c));
        if let Some(c) = celsius {
            out.push((node["Text"].as_str().unwrap_or("").to_string(), c));
        }
    }
    for child in node["Children"].as_array().into_iter().flatten() {
        collect_cpu_temps(child, out);
    }
}

fn sample(
    sys: &mut System,
    components: &mut Components,
    nvml: Option<&Nvml>,
    perf: Option<&perf::CpuPerf>,
    lhm: &ureq::Agent,
) -> Stats {
    sys.refresh_cpu_all();
    sys.refresh_memory();
    components.refresh(true);

    let cpus = sys.cpus();
    // the fastest core is the most representative "current clock"; on
    // Windows that is the base clock, so the counter scales it live
    let base_mhz = cpus.iter().map(|c| c.frequency()).max().unwrap_or(0);
    let cpu_freq_mhz = perf
        .and_then(|p| p.percent())
        .map_or(base_mhz, |pct| (base_mhz as f64 * pct / 100.0).round() as u64);
    Stats {
        cpu_usage: sys.global_cpu_usage(),
        cpu_freq_mhz,
        cpu_brand: cpus.first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
        cpu_temp: cpu_temp(components).or_else(|| lhm_cpu_temp(lhm)),
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
        let perf = perf::CpuPerf::new();
        let lhm = lhm_agent();
        loop {
            let stats = sample(&mut sys, &mut components, nvml.as_ref(), perf.as_ref(), &lhm);
            let _ = handle.emit("stats", stats);
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
        let s = sample(&mut sys, &mut components, Nvml::init().ok().as_ref(), None, &lhm_agent());
        assert!(s.mem_total > 0 && s.mem_used <= s.mem_total);
        assert!((0.0..=100.0).contains(&s.cpu_usage));
        assert!(s.cpu_temp.map_or(true, valid_temp));
    }

    #[cfg(windows)]
    #[test]
    fn reads_live_cpu_performance() {
        let perf = perf::CpuPerf::new().expect("open the % Processor Performance counter");
        thread::sleep(Duration::from_millis(500));
        let pct = perf.percent().expect("a second collection yields a value");
        // idle cores park well below base, boost runs past it
        assert!((5.0..300.0).contains(&pct), "implausible {pct}%");
    }

    /// Trimmed from a real data.json (Ryzen 5800X on an MSI X570 board).
    #[test]
    fn picks_tctl_from_librehardwaremonitor() {
        let tree = serde_json::json!({ "Text": "Sensor", "Children": [{ "Text": "PC", "Children": [
            { "Text": "Nuvoton NCT6797D", "Children": [{ "Text": "Temperatures", "Children": [
                { "Text": "Temperature #3", "Value": "41.5 \u{b0}C", "SensorId": "/lpc/nct6797d/0/temperature/3", "Type": "Temperature" }
            ]}]},
            { "Text": "AMD Ryzen 7 5800X", "Children": [{ "Text": "Temperatures", "Children": [
                { "Text": "CCD1 (Tdie)", "Value": "31.8 \u{b0}C", "SensorId": "/amdcpu/0/temperature/3", "Type": "Temperature" },
                { "Text": "Core (Tctl/Tdie)", "Value": "48.9 \u{b0}C", "SensorId": "/amdcpu/0/temperature/2", "Type": "Temperature" }
            ]}]}
        ]}]});
        assert_eq!(pick_lhm_cpu_temp(&tree), Some(48.9));

        let comma = serde_json::json!({ "Text": "CPU Package", "Value": "61,5 \u{b0}C",
            "SensorId": "/intelcpu/0/temperature/0", "Type": "Temperature" });
        assert_eq!(pick_lhm_cpu_temp(&comma), Some(61.5));
        assert_eq!(pick_lhm_cpu_temp(&serde_json::json!({ "Text": "Sensor" })), None);
    }

    #[test]
    fn rejects_unconnected_sensors() {
        assert!(!valid_temp(-128.0) && !valid_temp(0.0) && valid_temp(45.0));
    }
}
