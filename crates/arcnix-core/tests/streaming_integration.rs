//! Requires the loopback-only tools/sftp_fixture.py server.
use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use arcnix_core::ServerProfile;
use arcnix_core::remote::{CommandStream, PendingConnection, RemoteSession};

fn session() -> Result<RemoteSession> {
    let addr: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")
        .context("start SSH fixture first")?
        .parse()?;
    anyhow::ensure!(addr.ip().is_loopback(), "test fixture must be loopback");
    let profile = ServerProfile {
        host: addr.ip().to_string(),
        port: addr.port(),
        username: "arcnix".into(),
        ..Default::default()
    };
    let pending = PendingConnection::open(&profile)?;
    let fingerprint = pending.fingerprint().to_owned();
    pending.authenticate("arcnix-test", &fingerprint)
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn streams_before_exit_and_preserves_streams_and_status() -> Result<()> {
    let remote = session()?;
    let started = Instant::now();
    let mut chunks = Vec::new();
    let output = remote.exec_stream(
        "fixture-stream",
        None,
        Duration::from_secs(5),
        |kind, data| chunks.push((kind, data.to_vec(), started.elapsed())),
    )?;
    assert_eq!(output.exit_code, 9);
    assert_eq!(output.stdout, "first stdout\nlast stdout\n");
    assert_eq!(output.stderr, "first stderr\n");
    assert!(chunks[0].2 < Duration::from_millis(250));
    assert!(
        chunks
            .iter()
            .any(|(kind, ..)| *kind == CommandStream::Stderr)
    );
    assert_eq!(remote.exec("fixture-output")?.exit_code, 7);
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn verbose_commands_keep_draining_but_only_retain_tails() -> Result<()> {
    let remote = session()?;
    let mut seen = [0usize; 2];
    let output = remote.exec_stream(
        "fixture-stream-large",
        None,
        Duration::from_secs(10),
        |kind, data| seen[if kind == CommandStream::Stdout { 0 } else { 1 }] += data.len(),
    )?;
    assert_eq!(seen, [819_211, 819_211]);
    assert!(output.stdout.starts_with("[Earlier output omitted"));
    assert!(output.stdout.ends_with("stdout-end\n"));
    assert!(output.stderr.ends_with("stderr-end\n"));
    assert!(output.stdout.len() < 263_000 && output.stderr.len() < 263_000);
    assert_eq!(output.exit_code, 0);
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn private_stdin_uses_channel_and_timeout_restores_session() -> Result<()> {
    let remote = session()?;
    let mut streamed = Vec::new();
    let output = remote.exec_stream(
        "fixture-stream-input",
        Some(b"fixture-private-input\n"),
        Duration::from_secs(5),
        |_, bytes| streamed.extend_from_slice(bytes),
    )?;
    assert_eq!(output.stdout, "stdin accepted\n");
    assert!(!String::from_utf8_lossy(&streamed).contains("fixture-private-input"));
    let started = Instant::now();
    let error = remote
        .exec_stream(
            "fixture-stream-timeout",
            None,
            Duration::from_secs(1),
            |_, _| {},
        )
        .unwrap_err();
    assert!(error.chain().any(|e| {
        e.downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::TimedOut)
    }));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(remote.exec("fixture-output")?.exit_code, 7);
    Ok(())
}
