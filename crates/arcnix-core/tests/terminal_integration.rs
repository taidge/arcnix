//! Genuine SSH PTY requests against the fixed loopback peer; no local shell.
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use arcnix_core::ServerProfile;
use arcnix_core::remote::PendingConnection;
use arcnix_core::terminal::Pty;
fn terminal(username: &str) -> Result<Pty> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "fixture must use loopback");
    let profile = ServerProfile {
        host: address.ip().to_string(),
        port: address.port(),
        username: username.into(),
        ..Default::default()
    };
    let pending = PendingConnection::open(&profile)?;
    let fingerprint = pending.fingerprint().to_owned();
    pending
        .authenticate("arcnix-test", &fingerprint)?
        .into_terminal(80, 24)
}
fn until(terminal: &mut Pty, expected: &str) -> Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut bytes = Vec::new();
    loop {
        let output = terminal.poll(65_536)?;
        bytes.extend(output.bytes);
        if String::from_utf8_lossy(&bytes).contains(expected) {
            return Ok(bytes);
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "terminal output timed out: {expected}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore = "requires loopback SSH fixture"]
fn pty_streams_input_unicode_colors_resize_interrupt_and_exit() -> Result<()> {
    let mut pty = terminal("arcnix")?;
    until(&mut pty, "Arcnix terminal fixture")?;
    pty.input(b"pwd\r".to_vec())?;
    until(&mut pty, "/fixture")?;
    pty.resize(111, 37)?;
    pty.input(b"size\r".to_vec())?;
    until(&mut pty, "111x37")?;
    pty.input(b"ansi\r".to_vec())?;
    assert!(until(&mut pty, "RED")?.windows(5).any(|s| s == b"\x1b[31m"));
    pty.input(b"unicode\r".to_vec())?;
    until(&mut pty, "中文终端")?;
    pty.input(vec![3])?;
    until(&mut pty, "^C")?;
    pty.input(b"exit\r".to_vec())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(code) = pty.poll(65_536)?.exit_code {
            assert_eq!(code, 0);
            break;
        }
        anyhow::ensure!(Instant::now() < deadline, "shell did not exit");
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
#[test]
#[ignore = "requires loopback SSH fixture"]
fn pty_drains_output_larger_than_the_flow_control_window() -> Result<()> {
    let mut pty = terminal("arcnix")?;
    until(&mut pty, "Arcnix terminal fixture")?;
    pty.input(b"burst\r".to_vec())?;
    let output = until(&mut pty, "BURST COMPLETE")?;
    assert!(output.len() > 256 * 1024);
    Ok(())
}
#[test]
#[ignore = "requires loopback SSH fixture"]
fn pty_refusal_is_reported_as_a_terminal_error() -> Result<()> {
    let error = terminal("arcnix-no-terminal")
        .err()
        .context("PTY refusal was ignored")?;
    assert!(format!("{error:#}").contains("Server refused a terminal PTY"));
    Ok(())
}
