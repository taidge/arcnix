//! Deterministic Linux sample parsing and rate tests; the optional SSH test uses
//! only the loopback fixture and never installs or executes a remote agent.
use std::net::SocketAddr;

use anyhow::{Context, Result};
use arcnix_core::ServerProfile;
use arcnix_core::metrics::{self, ResourceSnapshot};
use arcnix_core::remote::PendingConnection;

fn response(cpu: &str, memory: &str, uptime: &str, network: &str, disk: &str) -> String {
    format!(
        "ARCNIX_METRICS_V1\nARCNIX_CPU_BEGIN\n{cpu}\nARCNIX_CPU_END\nARCNIX_MEMORY_BEGIN\n{memory}\nARCNIX_MEMORY_END\nARCNIX_LOAD_BEGIN\n0.25 0.50 0.75 1/100 321\nARCNIX_LOAD_END\nARCNIX_UPTIME_BEGIN\n{uptime}\nARCNIX_UPTIME_END\nARCNIX_NETWORK_BEGIN\n{network}\nARCNIX_NETWORK_END\nARCNIX_DISK_BEGIN\n{disk}\nARCNIX_DISK_END\nARCNIX_METRICS_END\n"
    )
}

const CPU: &str = "cpu  100 20 30 400 10 5 5 0 20 0\ncpu0 50 10 15 200 5 2 3 0 10 0\ncpu1 50 10 15 200 5 3 2 0 10 0";
const MEMORY: &str =
    "MemTotal: 1048576 kB\nMemAvailable: 524288 kB\nSwapTotal: 262144 kB\nSwapFree: 196608 kB";
const DISK: &str = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/root 1048576 262144 786432 25% /\n/dev/data 2097152 1048576 1048576 50% /srv/my data";
fn network(received: u64, transmitted: u64) -> String {
    format!(
        "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\nlo: 100000 1 0 0 0 0 0 0 100000 1 0 0 0 0 0 0\neth0: {received} 10 0 0 0 0 0 0 {transmitted} 20 0 0 0 0 0 0"
    )
}
fn initial() -> ResourceSnapshot {
    metrics::parse_snapshot(&response(
        CPU,
        MEMORY,
        "1000.00 1800.00",
        &network(10000, 20000),
        DISK,
    ))
    .unwrap()
}

#[test]
fn parses_linux_counts_units_paths_and_first_sample() {
    let snapshot = initial();
    assert!(snapshot.warnings.is_empty(), "{:?}", snapshot.warnings);
    let cpu = snapshot.cpu.as_ref().unwrap();
    assert_eq!(cpu.total_ticks, 570, "guest must not be counted twice");
    assert_eq!(cpu.idle_ticks, 410, "iowait belongs to idle");
    assert_eq!(cpu.logical_cpus, 2);
    let memory = snapshot.memory.as_ref().unwrap();
    assert_eq!(memory.total_bytes, 1024 * 1024 * 1024);
    assert_eq!(memory.used_bytes, 512 * 1024 * 1024);
    assert_eq!(memory.used_percent, 50.0);
    assert_eq!(snapshot.swap.as_ref().unwrap().used_percent, Some(25.0));
    assert_eq!(snapshot.load_average, Some([0.25, 0.5, 0.75]));
    assert_eq!(snapshot.disks[1].mount, "/srv/my data");
    assert_eq!(snapshot.disks[1].used_bytes, 1024 * 1024 * 1024);
    let usage = metrics::usage(&snapshot, None);
    assert!(usage.cpu_percent.is_none());
    assert!(usage.receive_bytes_per_second.is_none());
    assert!(
        usage
            .networks
            .iter()
            .all(|item| item.transmit_bytes_per_second.is_none())
    );
}

#[test]
fn derives_cpu_and_network_with_remote_elapsed_time() {
    let previous = initial();
    let cpu = CPU.replacen("100 20 30 400", "200 20 30 500", 1);
    let current = metrics::parse_snapshot(&response(
        &cpu,
        MEMORY,
        "1002.0 1800.0",
        &network(14000, 26000),
        DISK,
    ))
    .unwrap();
    let usage = metrics::usage(&current, Some(&previous));
    assert_eq!(usage.cpu_percent, Some(50.0));
    assert_eq!(usage.receive_bytes_per_second, Some(2000.0));
    assert_eq!(usage.transmit_bytes_per_second, Some(3000.0));
    assert_eq!(
        usage.networks.len(),
        2,
        "loopback remains visible per interface"
    );
    assert_eq!(usage.networks[0].receive_bytes_per_second, Some(0.0));
}

#[test]
fn guards_reboots_cpu_hotplug_zero_intervals_and_counter_resets() {
    let old = initial();
    let mut now = old.clone();
    now.uptime_seconds = Some(1002.0);
    now.cpu.as_mut().unwrap().total_ticks += 10;
    now.cpu.as_mut().unwrap().idle_ticks += 20;
    assert!(
        metrics::usage(&now, Some(&old)).cpu_percent.is_none(),
        "idle delta cannot exceed total"
    );
    now.cpu = old.cpu.clone();
    now.cpu.as_mut().unwrap().total_ticks = 1;
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    now.cpu = old.cpu.clone();
    now.cpu.as_mut().unwrap().total_ticks += 10;
    now.cpu.as_mut().unwrap().idle_ticks = 1;
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    now.cpu = old.cpu.clone();
    now.cpu.as_mut().unwrap().total_ticks += 10;
    now.cpu.as_mut().unwrap().logical_cpus = 3;
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    now.cpu = old.cpu.clone();
    now.cpu.as_mut().unwrap().total_ticks += 10;
    now.uptime_seconds = Some(999.0);
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    assert!(
        metrics::usage(&now, Some(&old))
            .receive_bytes_per_second
            .is_none()
    );
    now.uptime_seconds = old.uptime_seconds;
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    now.uptime_seconds = Some(1002.0);
    now.networks[1].received_bytes = 1;
    now.networks[1].transmitted_bytes += 100;
    let usage = metrics::usage(&now, Some(&old));
    assert!(usage.receive_bytes_per_second.is_none());
    assert_eq!(usage.transmit_bytes_per_second, Some(50.0));
    now.networks.pop();
    assert!(
        metrics::usage(&now, Some(&old))
            .transmit_bytes_per_second
            .is_none()
    );
}

#[test]
fn old_kernel_memory_fallback_is_conservative_and_swap_disabled_is_explicit() {
    let memory = "MemTotal: 1000 kB\nMemFree: 100 kB\nBuffers: 20 kB\nCached: 200 kB\nShmem: 50 kB\nSReclaimable: 300 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB";
    let snapshot =
        metrics::parse_snapshot(&response(CPU, memory, "1 1", &network(1, 2), DISK)).unwrap();
    let memory = snapshot.memory.unwrap();
    assert_eq!(memory.available_bytes, 270 * 1024);
    assert_eq!(memory.used_bytes, 730 * 1024);
    assert!(
        snapshot
            .warnings
            .iter()
            .any(|warning| warning.contains("保守估算"))
    );
    assert_eq!(snapshot.swap.unwrap().used_percent, None);
}

#[test]
fn malformed_sections_do_not_hide_valid_metrics_and_truncation_is_rejected() {
    let output = response(
        "cpu nonsense",
        "MemTotal: 18446744073709551615 kB",
        "NaN",
        "eth0: 12 3",
        "/dev/root broken",
    );
    let snapshot = metrics::parse_snapshot(&output).unwrap();
    assert!(snapshot.cpu.is_none());
    assert!(snapshot.memory.is_none());
    assert!(snapshot.uptime_seconds.is_none());
    assert!(snapshot.networks.is_empty());
    assert!(snapshot.disks.is_empty());
    assert!(
        snapshot.load_average.is_some(),
        "unaffected sections remain usable"
    );
    assert!(snapshot.warnings.len() >= 6);
    assert!(metrics::parse_snapshot(output.trim_end_matches("ARCNIX_METRICS_END\n")).is_err());
    assert!(metrics::parse_snapshot(&output.replace("ARCNIX_CPU_END", "")).is_err());
    assert!(metrics::parse_snapshot("Darwin").is_err());
    let overflow = CPU.replacen("100 20", "18446744073709551615 20", 1);
    assert!(
        metrics::parse_snapshot(&response(&overflow, MEMORY, "1 1", &network(1, 2), DISK))
            .unwrap()
            .cpu
            .is_none()
    );
}

#[test]
#[ignore = "requires tools/sftp_fixture.py and ARCNIX_TEST_SSH_ADDR"]
fn samples_over_real_ssh_without_sftp() -> Result<()> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")
        .context("start loopback fixture")?
        .parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "fixture must be loopback");
    let profile = ServerProfile {
        host: address.ip().to_string(),
        port: address.port(),
        username: "arcnix-nosftp".into(),
        ..Default::default()
    };
    let pending = PendingConnection::open(&profile)?;
    let fingerprint = pending.fingerprint().to_string();
    let session = pending.authenticate("arcnix-test", &fingerprint)?;
    assert!(!session.sftp_available());
    let snapshot = metrics::sample(&session)?;
    assert_eq!(snapshot.cpu.context("fixture CPU")?.logical_cpus, 2);
    assert_eq!(
        snapshot.memory.context("fixture memory")?.total_bytes,
        1024 * 1024 * 1024
    );
    assert_eq!(snapshot.disks[1].mount, "/srv/my data");
    Ok(())
}
