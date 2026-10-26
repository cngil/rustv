//! System statistics for the debug overlay, read from /proc and /sys.
//!
//! Rates (CPU load, network speed) need two readings, so a [`Sampler`] keeps
//! the previous one. Anything a machine does not expose is simply `None`.

use std::fs;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Sample {
    /// Percent, all cores.
    pub cpu: Option<f32>,
    /// Degrees Celsius.
    pub cpu_temp: Option<f32>,
    pub gpu: Option<f32>,
    pub gpu_temp: Option<f32>,
    /// Used and total RAM in bytes.
    pub memory: Option<(u64, u64)>,
    pub net: Option<Net>,
    /// Wi-Fi signal in dBm.
    pub wifi_dbm: Option<f32>,
    pub uptime_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Net {
    pub interface: String,
    /// Bytes per second.
    pub rx: f64,
    pub tx: f64,
}

#[derive(Default)]
pub struct Sampler {
    cpu: Option<CpuTimes>,
    net: Option<(String, u64, u64, Instant)>,
}

#[derive(Clone, Copy)]
struct CpuTimes {
    busy: u64,
    total: u64,
}

impl Sampler {
    pub fn sample(&mut self) -> Sample {
        let cpu = read("/proc/stat").as_deref().and_then(parse_cpu).and_then(|now| {
            let load = self.cpu.and_then(|before| cpu_percent(before, now));
            self.cpu = Some(now);
            load
        });

        let interface = read("/proc/net/route").as_deref().and_then(default_interface);
        let net = interface.and_then(|interface| {
            let (rx, tx) = parse_net_dev(&read("/proc/net/dev")?, &interface)?;
            let now = Instant::now();
            let rate = match &self.net {
                Some((name, rx0, tx0, at)) if *name == interface => {
                    let secs = now.duration_since(*at).as_secs_f64().max(0.001);
                    Some(Net {
                        interface: interface.clone(),
                        rx: rx.saturating_sub(*rx0) as f64 / secs,
                        tx: tx.saturating_sub(*tx0) as f64 / secs,
                    })
                }
                _ => None,
            };
            self.net = Some((interface, rx, tx, now));
            rate
        });
        let wifi_dbm =
            self.net.as_ref().and_then(|(interface, ..)| parse_wireless(&read("/proc/net/wireless")?, interface));

        let (cpu_temp, gpu_temp) = temperatures();
        Sample {
            cpu,
            cpu_temp,
            gpu: gpu_busy(),
            gpu_temp,
            memory: read("/proc/meminfo").as_deref().and_then(parse_meminfo),
            net,
            wifi_dbm,
            uptime_secs: read("/proc/uptime")
                .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok())
                .map(|secs| secs as u64),
        }
    }
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn parse_cpu(stat: &str) -> Option<CpuTimes> {
    let fields: Vec<u64> =
        stat.lines().next()?.strip_prefix("cpu ")?.split_whitespace().filter_map(|n| n.parse().ok()).collect();
    // user nice system idle iowait irq softirq steal
    let total: u64 = fields.iter().take(8).sum();
    let idle = fields.get(3)? + fields.get(4).copied().unwrap_or(0);
    Some(CpuTimes { busy: total - idle, total })
}

fn cpu_percent(before: CpuTimes, now: CpuTimes) -> Option<f32> {
    let total = now.total.checked_sub(before.total).filter(|&t| t > 0)?;
    let busy = now.busy.saturating_sub(before.busy);
    Some(busy as f32 * 100.0 / total as f32)
}

fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let field = |name: &str| -> Option<u64> {
        let line = text.lines().find(|line| line.starts_with(name))?;
        Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    Some((total.saturating_sub(available), total))
}

/// Interface of the default IPv4 route.
fn default_interface(route: &str) -> Option<String> {
    route.lines().skip(1).find_map(|line| {
        let mut parts = line.split_whitespace();
        let interface = parts.next()?;
        (parts.next()? == "00000000").then(|| interface.to_string())
    })
}

fn parse_net_dev(text: &str, interface: &str) -> Option<(u64, u64)> {
    let line = text.lines().find_map(|line| line.trim_start().strip_prefix(interface)?.strip_prefix(':'))?;
    let fields: Vec<u64> = line.split_whitespace().filter_map(|n| n.parse().ok()).collect();
    Some((*fields.first()?, *fields.get(8)?))
}

fn parse_wireless(text: &str, interface: &str) -> Option<f32> {
    let line = text.lines().find_map(|line| line.trim_start().strip_prefix(interface)?.strip_prefix(':'))?;
    // status, link quality, signal level (dBm), ...
    line.split_whitespace().nth(2)?.trim_end_matches('.').parse().ok()
}

/// CPU and GPU temperatures from hwmon drivers.
fn temperatures() -> (Option<f32>, Option<f32>) {
    let (mut cpu, mut gpu) = (None, None);
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else { return (None, None) };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(name) = read(dir.join("name")) else { continue };
        let temp = || Some(read(dir.join("temp1_input"))?.trim().parse::<f32>().ok()? / 1000.0);
        match name.trim() {
            "k10temp" | "zenpower" | "coretemp" | "cpu_thermal" if cpu.is_none() => cpu = temp(),
            "amdgpu" | "radeon" | "nouveau" | "i915" | "xe" if gpu.is_none() => gpu = temp(),
            _ => {}
        }
    }
    (cpu, gpu)
}

/// GPU load in percent. Only amdgpu exposes this without extra tools.
fn gpu_busy() -> Option<f32> {
    fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .find_map(|entry| read(entry.path().join("device/gpu_busy_percent"))?.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_cpu_load() {
        let a = parse_cpu("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 2 3\n").unwrap();
        let b = parse_cpu("cpu  150 0 150 900 0 0 0 0 0 0\n").unwrap();
        assert_eq!(cpu_percent(a, b), Some(50.0));
        assert_eq!(cpu_percent(b, b), None);
    }

    #[test]
    fn reads_memory() {
        let text = "MemTotal:        5939200 kB\nMemFree:          451584 kB\nMemAvailable:    3375104 kB\n";
        assert_eq!(parse_meminfo(text), Some(((5939200 - 3375104) * 1024, 5939200 * 1024)));
    }

    #[test]
    fn reads_network() {
        let route = "Iface\tDestination\tGateway\nwlp2s0\t0001A8C0\t00000000\nwlp2s0\t00000000\t0101A8C0\n";
        assert_eq!(default_interface(route).as_deref(), Some("wlp2s0"));
        let dev = "Inter-|   Receive\n face |bytes packets\n    lo: 10 1 0 0 0 0 0 0 10 1 0 0 0 0 0 0\n\
                   wlp2s0: 1000 9 0 0 0 0 0 0 500 4 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(dev, "wlp2s0"), Some((1000, 500)));
        let wireless = "Inter-| sta-|   Quality\n face | tus | link level noise\nwlp2s0: 0000   44.  -66.  -256 0 0\n";
        assert_eq!(parse_wireless(wireless, "wlp2s0"), Some(-66.0));
    }
}
