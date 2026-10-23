//! Bounded, read-only host and native-process resource sampling.
//!
//! This module deliberately reports only aggregate counters. It never exposes
//! command lines, environment variables, file paths, credentials, or arbitrary
//! PIDs supplied by a caller.

use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs, io,
    time::{SystemTime, UNIX_EPOCH},
};

const SAMPLE_WINDOW_MS: u64 = 250;
const CORE_PROCESS: &str = "lmm-core";
const EXTENSIONS_PROCESS: &str = "lmm-extensions";

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub observed_at_unix_ms: i64,
    pub sample_window_ms: u64,
    pub system: SystemResources,
    pub processes: BTreeMap<String, ProcessResources>,
}

#[derive(Debug, Serialize)]
pub struct SystemResources {
    pub logical_cpus: u32,
    pub load_average: LoadAverage,
    pub cpu_usage_percent: f64,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub memory_used_bytes: u64,
    pub memory_usage_percent: f64,
}

#[derive(Debug, Serialize)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(Debug, Serialize)]
pub struct ProcessResources {
    pub status: &'static str,
    pub version: Option<String>,
    pub pid: Option<u32>,
    pub cpu_percent: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub memory_percent: Option<f64>,
    pub threads: Option<u64>,
}

#[derive(Clone, Copy)]
struct CpuTicks {
    total: u64,
    idle: u64,
}

#[derive(Clone, Copy)]
struct ProcessSample {
    pid: u32,
    ticks: u64,
    rss_bytes: u64,
    threads: u64,
}

#[derive(Debug)]
pub enum SnapshotError {
    Io,
    Invalid,
}

impl From<io::Error> for SnapshotError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

pub async fn snapshot() -> Result<Snapshot, SnapshotError> {
    let before_host = cpu_ticks()?;
    let before = process_samples();
    tokio::time::sleep(std::time::Duration::from_millis(SAMPLE_WINDOW_MS)).await;
    let after_host = cpu_ticks()?;
    let after = process_samples();

    let total_delta = after_host.total.saturating_sub(before_host.total);
    let idle_delta = after_host.idle.saturating_sub(before_host.idle);
    let cpu_usage_percent = percent(total_delta.saturating_sub(idle_delta), total_delta);
    let (total_bytes, available_bytes) = memory_bytes()?;
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let mut processes = BTreeMap::new();
    for name in [CORE_PROCESS, EXTENSIONS_PROCESS] {
        let version = process_version(name);
        let sample = after.get(name).copied();
        let previous = before.get(name).copied();
        let cpu_percent = match (sample, previous, total_delta) {
            (Some(now), Some(old), denominator) if denominator > 0 => {
                Some(percent(now.ticks.saturating_sub(old.ticks), denominator))
            }
            _ => None,
        };
        let process = sample.map(|sample| ProcessResources {
            status: "running",
            version: version.clone(),
            pid: Some(sample.pid),
            cpu_percent,
            memory_bytes: Some(sample.rss_bytes),
            memory_percent: Some(percent(sample.rss_bytes, total_bytes)),
            threads: Some(sample.threads),
        });
        processes.insert(
            name.to_owned(),
            process.unwrap_or(ProcessResources {
                status: "unavailable",
                version,
                pid: None,
                cpu_percent: None,
                memory_bytes: None,
                memory_percent: None,
                threads: None,
            }),
        );
    }
    let logical_cpus = std::thread::available_parallelism()
        .map(|value| value.get().min(u32::MAX as usize) as u32)
        .unwrap_or(1);
    Ok(Snapshot {
        schema_version: 1,
        observed_at_unix_ms: now_unix_ms(),
        sample_window_ms: SAMPLE_WINDOW_MS,
        system: SystemResources {
            logical_cpus,
            load_average: load_average()?,
            cpu_usage_percent,
            memory_total_bytes: total_bytes,
            memory_available_bytes: available_bytes,
            memory_used_bytes: used_bytes,
            memory_usage_percent: percent(used_bytes, total_bytes),
        },
        processes,
    })
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or(0)
}

fn percent(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    ((numerator as f64 / denominator as f64) * 100.0).clamp(0.0, 100.0)
}

fn cpu_ticks() -> Result<CpuTicks, SnapshotError> {
    let stat = fs::read_to_string("/proc/stat")?;
    let line = stat
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or(SnapshotError::Invalid)?;
    let values = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(|value| value.parse::<u64>().map_err(|_| SnapshotError::Invalid))
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() < 4 {
        return Err(SnapshotError::Invalid);
    }
    let total = values.iter().copied().sum();
    let idle = values[3].saturating_add(values.get(4).copied().unwrap_or(0));
    Ok(CpuTicks { total, idle })
}

fn process_samples() -> BTreeMap<&'static str, ProcessSample> {
    let mut result = BTreeMap::new();
    for (name, pid) in [(CORE_PROCESS, std::process::id()), (EXTENSIONS_PROCESS, 0)] {
        let pid = if name == CORE_PROCESS {
            pid
        } else {
            find_process(name).unwrap_or(0)
        };
        if pid == 0 {
            continue;
        }
        if let Some(sample) = read_process(pid) {
            result.insert(name, sample);
        }
    }
    result
}

fn find_process(name: &str) -> Option<u32> {
    fs::read_dir("/proc")
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
        .find(|pid| {
            fs::read_link(format!("/proc/{pid}/exe"))
                .ok()
                .and_then(|path| path.file_name().map(|value| value == name))
                .unwrap_or(false)
        })
}

fn read_process(pid: u32) -> Option<ProcessSample> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.rsplit_once(") ")?.1;
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    let ticks = fields
        .get(11)?
        .parse::<u64>()
        .ok()?
        .saturating_add(fields.get(12)?.parse::<u64>().ok()?);
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let rss_bytes = status_value(&status, "VmRSS:")?.saturating_mul(1024);
    let threads = status_value(&status, "Threads:")?;
    Some(ProcessSample {
        pid,
        ticks,
        rss_bytes,
        threads,
    })
}

fn status_value(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.split_whitespace().next()?;
        value.parse::<u64>().ok()
    })
}

fn memory_bytes() -> Result<(u64, u64), SnapshotError> {
    let text = fs::read_to_string("/proc/meminfo")?;
    let value = |key: &str| {
        text.lines()
            .find_map(|line| {
                line.strip_prefix(key)?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .map(|kb| kb.saturating_mul(1024))
            .ok_or(SnapshotError::Invalid)
    };
    Ok((value("MemTotal:")?, value("MemAvailable:")?))
}

fn load_average() -> Result<LoadAverage, SnapshotError> {
    let text = fs::read_to_string("/proc/loadavg")?;
    let values = text
        .split_whitespace()
        .take(3)
        .map(|value| value.parse::<f64>().map_err(|_| SnapshotError::Invalid))
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 3
        || values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(SnapshotError::Invalid);
    }
    Ok(LoadAverage {
        one: values[0],
        five: values[1],
        fifteen: values[2],
    })
}

fn process_version(name: &str) -> Option<String> {
    if name == CORE_PROCESS {
        return Some(env!("CARGO_PKG_VERSION").to_owned());
    }
    let bytes = fs::read("/usr/share/doc/lmm-extensions/BUILDINFO.json").ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.get("version")?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::percent;

    #[test]
    fn percent_is_bounded_and_zero_safe() {
        assert_eq!(percent(0, 0), 0.0);
        assert_eq!(percent(2, 1), 100.0);
        assert_eq!(percent(1, 4), 25.0);
    }
}
