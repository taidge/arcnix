//! Read-only Linux resource samples over the existing SSH connection.
//!
//! Rates use remote uptime, so client latency and wall-clock adjustments do not
//! distort them. No remote agent, elevated permissions or persistent process is used.
use std::collections::HashMap;

use anyhow::{Context, Result, ensure};

use crate::remote::RemoteSession;

pub const SAMPLE_COMMAND: &str = r#"# arcnix:metrics:sample
LC_ALL=C; export LC_ALL
if [ "$(uname -s)" != Linux ]; then printf 'Resource monitoring requires Linux /proc.\n' >&2; exit 1; fi
printf 'ARCNIX_METRICS_V1\nARCNIX_CPU_BEGIN\n'
cat /proc/stat 2>/dev/null
printf '\nARCNIX_CPU_END\nARCNIX_MEMORY_BEGIN\n'
cat /proc/meminfo 2>/dev/null
printf '\nARCNIX_MEMORY_END\nARCNIX_LOAD_BEGIN\n'
cat /proc/loadavg 2>/dev/null
printf '\nARCNIX_LOAD_END\nARCNIX_UPTIME_BEGIN\n'
cat /proc/uptime 2>/dev/null
printf '\nARCNIX_UPTIME_END\nARCNIX_NETWORK_BEGIN\n'
cat /proc/net/dev 2>/dev/null
printf '\nARCNIX_NETWORK_END\nARCNIX_DISK_BEGIN\n'
df -Pk 2>/dev/null
printf '\nARCNIX_DISK_END\nARCNIX_METRICS_END\n'
"#;

#[derive(Clone, Debug)]
pub struct CpuCounters {
    pub total_ticks: u64,
    pub idle_ticks: u64,
    pub logical_cpus: usize,
}

#[derive(Clone, Debug)]
pub struct MemoryUsage {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub used_percent: f64,
}

#[derive(Clone, Debug)]
pub struct SwapUsage {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
    /// None when swap is disabled (total is zero).
    pub used_percent: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct DiskUsage {
    pub filesystem: String,
    pub mount: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct NetworkCounters {
    pub interface: String,
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ResourceSnapshot {
    pub cpu: Option<CpuCounters>,
    pub uptime_seconds: Option<f64>,
    pub load_average: Option<[f64; 3]>,
    pub memory: Option<MemoryUsage>,
    pub swap: Option<SwapUsage>,
    pub disks: Vec<DiskUsage>,
    pub networks: Vec<NetworkCounters>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct NetworkUsage {
    pub interface: String,
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
    pub receive_bytes_per_second: Option<f64>,
    pub transmit_bytes_per_second: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct ResourceUsage {
    pub snapshot: ResourceSnapshot,
    pub cpu_percent: Option<f64>,
    pub networks: Vec<NetworkUsage>,
    /// Aggregate rates exclude loopback. None if any included interface lacks
    /// a previous valid counter, or the interface set changed.
    pub receive_bytes_per_second: Option<f64>,
    pub transmit_bytes_per_second: Option<f64>,
}

pub fn sample(session: &RemoteSession) -> Result<ResourceSnapshot> {
    let output = session
        .exec(SAMPLE_COMMAND)
        .context("读取远程资源信息失败")?;
    ensure!(
        output.exit_code == 0,
        "读取资源失败（退出码 {}）：{}",
        output.exit_code,
        output.stderr.trim()
    );
    parse_snapshot(&output.stdout)
}

/// Parse the complete framed command response. An absent or invalid section
/// yields a warning; missing framing rejects truncated SSH output altogether.
pub fn parse_snapshot(output: &str) -> Result<ResourceSnapshot> {
    let lines: Vec<_> = output.lines().map(str::trim_end).collect();
    ensure!(
        lines.first() == Some(&"ARCNIX_METRICS_V1"),
        "无法识别资源采样响应（需要 Linux /proc）"
    );
    ensure!(
        lines.last() == Some(&"ARCNIX_METRICS_END"),
        "资源采样响应不完整，请重试"
    );
    let mut cursor = 1;
    let mut sections = Vec::new();
    for name in ["CPU", "MEMORY", "LOAD", "UPTIME", "NETWORK", "DISK"] {
        let begin = format!("ARCNIX_{name}_BEGIN");
        let end = format!("ARCNIX_{name}_END");
        ensure!(
            lines.get(cursor) == Some(&begin.as_str()),
            "资源采样缺少 {name} 起始标记"
        );
        cursor += 1;
        let start = cursor;
        while cursor < lines.len() && lines[cursor] != end {
            ensure!(
                !lines[cursor].starts_with("ARCNIX_"),
                "资源采样 {name} 内容不完整"
            );
            cursor += 1;
        }
        ensure!(cursor < lines.len(), "资源采样缺少 {name} 结束标记");
        sections.push(lines[start..cursor].join("\n"));
        cursor += 1;
    }
    ensure!(cursor == lines.len() - 1, "资源采样响应包含意外内容");
    let mut result = ResourceSnapshot::default();
    result.cpu = section_result(parse_cpu(&sections[0]), "CPU", &mut result.warnings);
    let (memory, swap) = parse_memory(&sections[1], &mut result.warnings);
    result.memory = memory;
    result.swap = swap;
    result.load_average =
        section_result(parse_load(&sections[2]), "系统负载", &mut result.warnings);
    result.uptime_seconds =
        section_result(parse_uptime(&sections[3]), "运行时间", &mut result.warnings);
    result.networks = parse_network(&sections[4], &mut result.warnings);
    result.disks = parse_disks(&sections[5], &mut result.warnings);
    Ok(result)
}

fn section_result<T>(result: Result<T>, name: &str, warnings: &mut Vec<String>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            warnings.push(format!("{name} 暂不可用：{error}"));
            None
        }
    }
}

fn parse_cpu(text: &str) -> Result<CpuCounters> {
    let row = text
        .lines()
        .find(|line| line.starts_with("cpu "))
        .context("缺少 /proc/stat CPU 计数")?;
    let fields: Vec<_> = row.split_whitespace().skip(1).collect();
    ensure!(fields.len() >= 4, "CPU 计数不完整");
    // guest and guest_nice (columns 9 and 10) are already included in user and
    // nice respectively. Sum only the first eight non-overlapping counters.
    let ticks: Vec<u64> = fields
        .iter()
        .take(8)
        .map(|field| field.parse().context("无效 CPU 计数"))
        .collect::<Result<_>>()?;
    let total_ticks = ticks
        .iter()
        .try_fold(0u64, |sum, value| sum.checked_add(*value))
        .context("CPU 计数溢出")?;
    let idle_ticks = ticks[3]
        .checked_add(ticks.get(4).copied().unwrap_or(0))
        .context("CPU 空闲计数溢出")?;
    let logical_cpus = text
        .lines()
        .filter(|line| {
            line.split_whitespace()
                .next()
                .and_then(|word| word.strip_prefix("cpu"))
                .is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
        .count();
    ensure!(logical_cpus > 0, "缺少逻辑 CPU 信息");
    Ok(CpuCounters {
        total_ticks,
        idle_ticks,
        logical_cpus,
    })
}

fn kib(value: u64) -> Result<u64> {
    value.checked_mul(1024).context("KiB 字节数溢出")
}
fn ratio(used: u64, total: u64) -> Option<f64> {
    (total > 0 && used <= total).then(|| used as f64 / total as f64 * 100.0)
}

fn parse_memory(
    text: &str,
    warnings: &mut Vec<String>,
) -> (Option<MemoryUsage>, Option<SwapUsage>) {
    let values: HashMap<_, _> = text
        .lines()
        .filter_map(|line| line.split_once(':'))
        .collect();
    let get = |key: &str| -> Result<u64> {
        let mut parts = values
            .get(key)
            .with_context(|| format!("缺少 {key}"))?
            .split_whitespace();
        let value = parts.next().context("缺少内存值")?.parse::<u64>()?;
        ensure!(
            parts.next() == Some("kB") && parts.next().is_none(),
            "无效内存单位"
        );
        kib(value)
    };
    let memory = (|| -> Result<MemoryUsage> {
        let total_bytes = get("MemTotal")?;
        ensure!(total_bytes > 0, "内存总量为零");
        let available_bytes = if values.contains_key("MemAvailable") {
            get("MemAvailable")?
        } else {
            // Older kernels: conservatively count MemFree plus documented
            // buffers/cache minus shared memory. Reclaimable slab is omitted.
            let free = get("MemFree")?;
            let mut cached = 0u64;
            for key in ["Buffers", "Cached"] {
                if values.contains_key(key) {
                    cached = cached.checked_add(get(key)?).context("可用内存溢出")?;
                }
            }
            if values.contains_key("Shmem") {
                cached = cached.saturating_sub(get("Shmem")?);
            }
            warnings.push("内核未提供 MemAvailable；可用内存使用保守估算".into());
            free.checked_add(cached).context("可用内存溢出")?
        }
        .min(total_bytes);
        let used_bytes = total_bytes - available_bytes;
        Ok(MemoryUsage {
            total_bytes,
            available_bytes,
            used_bytes,
            used_percent: ratio(used_bytes, total_bytes).unwrap(),
        })
    })();
    let swap = (|| -> Result<SwapUsage> {
        let total_bytes = get("SwapTotal")?;
        let free_bytes = get("SwapFree")?.min(total_bytes);
        let used_bytes = total_bytes - free_bytes;
        Ok(SwapUsage {
            total_bytes,
            free_bytes,
            used_bytes,
            used_percent: ratio(used_bytes, total_bytes),
        })
    })();
    (
        section_result(memory, "内存", warnings),
        section_result(swap, "交换空间", warnings),
    )
}

fn finite_nonnegative(text: &str) -> Result<f64> {
    let value: f64 = text.parse()?;
    ensure!(value.is_finite() && value >= 0.0, "数值不是有效非负数");
    Ok(value)
}
fn parse_load(text: &str) -> Result<[f64; 3]> {
    let mut fields = text.split_whitespace();
    let mut result = [0.0; 3];
    for value in &mut result {
        *value = finite_nonnegative(fields.next().context("缺少负载值")?)?;
    }
    Ok(result)
}
fn parse_uptime(text: &str) -> Result<f64> {
    finite_nonnegative(text.split_whitespace().next().context("缺少运行时间")?)
}

fn parse_network(text: &str, warnings: &mut Vec<String>) -> Vec<NetworkCounters> {
    let mut result = Vec::new();
    for line in text.lines().filter(|line| line.contains(':')) {
        let (interface, data) = line.rsplit_once(':').unwrap();
        let parsed = (|| -> Result<NetworkCounters> {
            let fields: Vec<_> = data.split_whitespace().collect();
            ensure!(
                fields.len() >= 16 && !interface.trim().is_empty(),
                "网卡计数不完整"
            );
            Ok(NetworkCounters {
                interface: interface.trim().into(),
                received_bytes: fields[0].parse()?,
                transmitted_bytes: fields[8].parse()?,
            })
        })();
        if let Some(network) = section_result(parsed, "网卡", warnings) {
            result.push(network);
        }
    }
    if result.is_empty() {
        warnings.push("网络计数暂不可用：缺少 /proc/net/dev 数据".into());
    }
    result
}

fn parse_disks(text: &str, warnings: &mut Vec<String>) -> Vec<DiskUsage> {
    let mut result = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("Filesystem"))
    {
        let parsed = (|| -> Result<DiskUsage> {
            // Consume exactly five whitespace-separated columns; the remainder
            // is the mount path and may contain spaces (df -P never wraps rows).
            let mut remainder = line;
            let mut fields = Vec::new();
            for _ in 0..5 {
                let index = remainder
                    .find(char::is_whitespace)
                    .context("磁盘行不完整")?;
                fields.push(&remainder[..index]);
                remainder = remainder[index..].trim_start();
            }
            ensure!(!remainder.is_empty(), "缺少挂载目录");
            let total_bytes = kib(fields[1].parse()?)?;
            let used_bytes = kib(fields[2].parse()?)?;
            let available_bytes = kib(fields[3].parse()?)?;
            ensure!(
                used_bytes <= total_bytes && available_bytes <= total_bytes,
                "磁盘计数超出总量"
            );
            Ok(DiskUsage {
                filesystem: fields[0].into(),
                mount: remainder.into(),
                total_bytes,
                used_bytes,
                available_bytes,
                used_percent: ratio(used_bytes, total_bytes),
            })
        })();
        if let Some(disk) = section_result(parsed, "磁盘", warnings) {
            result.push(disk);
        }
    }
    if result.is_empty() {
        warnings.push("磁盘空间暂不可用：df 未返回有效数据".into());
    }
    result
}

/// Derive rates from two raw samples; first samples, reboots, hotplug, counter
/// resets and nonpositive intervals leave affected rates unavailable.
pub fn usage(current: &ResourceSnapshot, previous: Option<&ResourceSnapshot>) -> ResourceUsage {
    let interval = previous.and_then(|old| {
        let seconds = current.uptime_seconds? - old.uptime_seconds?;
        (seconds.is_finite() && seconds > 0.0).then_some(seconds)
    });
    let invalid_interval = previous.is_some_and(|old| matches!((current.uptime_seconds, old.uptime_seconds), (Some(now), Some(before)) if now <= before));
    let cpu_percent = previous.and_then(|old| {
        if invalid_interval {
            return None;
        }
        let now = current.cpu.as_ref()?;
        let before = old.cpu.as_ref()?;
        if now.logical_cpus != before.logical_cpus {
            return None;
        }
        let total = now.total_ticks.checked_sub(before.total_ticks)?;
        let idle = now.idle_ticks.checked_sub(before.idle_ticks)?;
        if total == 0 || idle > total {
            return None;
        }
        ratio(total - idle, total)
    });
    let networks: Vec<_> = current
        .networks
        .iter()
        .map(|now| {
            let before = previous.and_then(|old| {
                old.networks
                    .iter()
                    .find(|item| item.interface == now.interface)
            });
            let rate = |counter: u64, previous: Option<u64>| -> Option<f64> {
                let value = counter.checked_sub(previous?)? as f64 / interval?;
                value.is_finite().then_some(value)
            };
            NetworkUsage {
                interface: now.interface.clone(),
                received_bytes: now.received_bytes,
                transmitted_bytes: now.transmitted_bytes,
                receive_bytes_per_second: rate(
                    now.received_bytes,
                    before.map(|item| item.received_bytes),
                ),
                transmit_bytes_per_second: rate(
                    now.transmitted_bytes,
                    before.map(|item| item.transmitted_bytes),
                ),
            }
        })
        .collect();
    let aggregate = |receive: bool| -> Option<f64> {
        let before = previous?;
        let included: Vec<_> = networks
            .iter()
            .filter(|item| item.interface != "lo")
            .collect();
        if included.is_empty()
            || before
                .networks
                .iter()
                .filter(|item| item.interface != "lo")
                .count()
                != included.len()
        {
            return None;
        }
        included.iter().try_fold(0.0, |sum, item| {
            let value = if receive {
                item.receive_bytes_per_second?
            } else {
                item.transmit_bytes_per_second?
            };
            let sum = sum + value;
            sum.is_finite().then_some(sum)
        })
    };
    let receive_bytes_per_second = aggregate(true);
    let transmit_bytes_per_second = aggregate(false);
    ResourceUsage {
        snapshot: current.clone(),
        cpu_percent,
        networks,
        receive_bytes_per_second,
        transmit_bytes_per_second,
    }
}
