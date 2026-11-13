//! Real loopback SSH transport, fixed fixture log responses; no real system reads.
use std::net::SocketAddr;

use anyhow::{Context, Result};
use arcnix_core::ServerProfile;
use arcnix_core::remote::PendingConnection;
use arcnix_core::system_logs::{self, JournalPeriod, LogQuery, LogSource};

#[test]
#[ignore = "requires tools/sftp_fixture.py and ARCNIX_TEST_SSH_ADDR"]
fn discover_and_read_logs_without_sftp() -> Result<()> {
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
    let fingerprint = pending.fingerprint().to_owned();
    let session = pending.authenticate("arcnix-test", &fingerprint)?;
    assert!(!session.sftp_available());
    let capabilities = system_logs::discover(&session)?;
    assert!(capabilities.sources.contains(&LogSource::Journal));
    let query = LogQuery {
        unit: "ssh.service".into(),
        period: JournalPeriod::Today,
        ..Default::default()
    };
    let logs = system_logs::read(&session, &query)?;
    assert_eq!(logs.source, LogSource::Journal);
    assert!(!logs.entries.is_empty());
    assert!(logs.entries.iter().any(|entry| entry.timestamp.is_some()));
    let file_query = LogQuery {
        source: LogSource::Syslog,
        ..Default::default()
    };
    let logs = system_logs::read(&session, &file_query)?;
    assert!(!logs.entries.is_empty());
    assert!(logs.entries.iter().all(|entry| entry.timestamp.is_none()));
    assert!(
        system_logs::read(
            &session,
            &LogQuery {
                unit: "$(id)".into(),
                ..Default::default()
            }
        )
        .is_err()
    );
    Ok(())
}
