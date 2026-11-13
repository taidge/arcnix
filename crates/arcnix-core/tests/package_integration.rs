//! Canned package operations through a real loopback SSH transport; never runs apt.
use std::net::SocketAddr;

use anyhow::{Context, Result};
use arcnix_core::ServerProfile;
use arcnix_core::packages::{self, PackageAction, PackageManager};
use arcnix_core::remote::PendingConnection;

#[test]
#[ignore = "requires tools/sftp_fixture.py and ARCNIX_TEST_SSH_ADDR"]
fn package_detection_search_installed_and_confirmable_preview() -> Result<()> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")
        .context("start loopback fixture")?
        .parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "fixture must be loopback");
    let profile = ServerProfile {
        host: address.ip().to_string(),
        port: address.port(),
        username: "arcnix".into(),
        ..Default::default()
    };
    let pending = PendingConnection::open(&profile)?;
    let fingerprint = pending.fingerprint().to_owned();
    let session = pending.authenticate("arcnix-test", &fingerprint)?;
    let environment = packages::detect(&session)?;
    assert_eq!(environment.manager, PackageManager::Apt);
    assert!(!environment.is_root);
    assert!(environment.sudo_available);
    let installed = packages::installed(&session, &environment)?;
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].name, "curl");
    assert!(installed[0].installed);
    let search = packages::search(&session, &environment, "nginx")?;
    assert_eq!(search.len(), 2);
    assert!(!search[0].installed);
    assert!(search[1].installed);
    assert_eq!(search[1].version, "8.5");
    let plan = packages::preview(
        &session,
        &environment,
        PackageAction::Install(vec!["nginx".into()]),
    )?;
    assert!(plan.preview.contains("Inst nginx"));
    assert!(plan.command.contains("sudo -n"));
    assert!(
        packages::preview(
            &session,
            &environment,
            PackageAction::Install(vec!["does.not.exist".into()])
        )
        .is_err()
    );
    let command = packages::build_execution(&environment, &plan, true)?;
    assert!(command.use_stdin_password);
    assert!(command.command.contains("sudo -S"));
    let refresh = packages::preview(&session, &environment, PackageAction::RefreshIndex)?;
    assert!(refresh.preview.contains("不会升级"));
    assert!(refresh.command.contains("update"));
    assert!(!refresh.command.contains("upgrade"));
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn ssh_system_management_does_not_require_sftp() -> Result<()> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
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
    assert!(session.list_dir("/").is_err());
    let environment = packages::detect(&session)?;
    assert_eq!(environment.manager, PackageManager::Apt);
    assert!(!packages::installed(&session, &environment)?.is_empty());
    let output = session.exec("fixture-output")?;
    assert_eq!(output.exit_code, 7);
    Ok(())
}
