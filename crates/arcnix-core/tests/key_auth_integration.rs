//! Generated, test-owned OpenSSH keys only; requires tools/sftp_fixture.py.
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};
use arcnix_core::remote::{PendingConnection, RemoteSession};
use arcnix_core::{AuthMethod, ServerProfile, new_id};

fn fixture_profile(key: &str) -> Result<(ServerProfile, serde_json::Value)> {
    let fixture: serde_json::Value = serde_json::from_slice(&fs::read(
        std::env::var_os("ARCNIX_TEST_FIXTURE").context("Set ARCNIX_TEST_FIXTURE")?,
    )?)?;
    let address: SocketAddr = fixture["address"]
        .as_str()
        .context("fixture address")?
        .parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "fixture must use loopback");
    let profile = ServerProfile {
        host: address.ip().to_string(),
        port: address.port(),
        username: "arcnix".into(),
        auth: AuthMethod::PrivateKey,
        private_key: Some(PathBuf::from(fixture[key].as_str().context("fixture key")?)),
        ..Default::default()
    };
    Ok((profile, fixture))
}

fn authenticate(key: &str, passphrase: &str) -> Result<RemoteSession> {
    let (profile, _) = fixture_profile(key)?;
    let pending = PendingConnection::open(&profile)?;
    let fingerprint = pending.fingerprint().to_owned();
    pending.authenticate(passphrase, &fingerprint)
}

#[test]
#[ignore = "requires loopback SSH fixture and ARCNIX_TEST_FIXTURE"]
fn ed25519_openssh_key_supports_commands_and_file_roundtrip() -> Result<()> {
    let session = authenticate("client_key_ed25519", "")?;
    assert_eq!(session.exec("fixture-output")?.stdout, "fixture stdout\n");
    let directory = std::env::temp_dir().join(format!("arcnix-key-auth-{}", new_id()));
    fs::create_dir(&directory)?;
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let source = directory.join("source.txt");
    let destination = directory.join("download.txt");
    let contents = b"Ed25519 SSH and SFTP roundtrip\n";
    fs::write(&source, contents)?;
    let remote = format!("/ed25519-{}.txt", new_id());
    let cancel = AtomicBool::new(false);
    session.upload_path(&source, &remote, &cancel, |_, _| {})?;
    session.download_path(&remote, &destination, &cancel, |_, _| {})?;
    assert_eq!(fs::read(destination)?, contents);
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture and ARCNIX_TEST_FIXTURE"]
fn encrypted_ed25519_openssh_key_authenticates() -> Result<()> {
    let session = authenticate("client_key_ed25519_encrypted", "arcnix-test-key-passphrase")?;
    assert!(session.sftp_available());
    assert!(!session.list_dir("/")?.1.is_empty());
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture and ARCNIX_TEST_FIXTURE"]
fn encrypted_key_rejects_missing_or_wrong_passphrase_without_disclosing_it() -> Result<()> {
    for passphrase in ["", "wrong-test-passphrase"] {
        let error = authenticate("client_key_ed25519_encrypted", passphrase)
            .err()
            .context("incorrect passphrase was accepted")?;
        let message = format!("{error:#}");
        assert!(
            message.contains("key") && message.contains("passphrase"),
            "{message}"
        );
        assert!(!message.contains("wrong-test-passphrase"));
        assert!(!message.contains("arcnix-test-key-passphrase"));
    }
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture and ARCNIX_TEST_FIXTURE"]
fn untrusted_key_reports_server_rejection() -> Result<()> {
    let error = authenticate("client_key_untrusted", "")
        .err()
        .context("server accepted an unauthorized key")?;
    let message = format!("{error:#}");
    assert!(message.contains("Server rejected the SSH key"), "{message}");
    Ok(())
}

#[test]
#[ignore = "requires dual-host-key loopback fixture and ARCNIX_TEST_FIXTURE"]
fn saved_rsa_host_key_survives_backend_preference_change() -> Result<()> {
    let (profile, fixture) = fixture_profile("client_key_ed25519")?;
    let expected = fixture["host_key_rsa_fingerprint"]
        .as_str()
        .context("RSA pin")?;
    let default = PendingConnection::open(&profile)?;
    assert_ne!(
        default.fingerprint(),
        expected,
        "fixture must prefer a different host key"
    );
    drop(default);
    let pending = PendingConnection::open_for_known_host(&profile, expected)?;
    assert_eq!(pending.fingerprint(), expected);
    let session = pending.authenticate("", expected)?;
    assert!(session.sftp_available());
    assert_eq!(session.exec("fixture-output")?.stdout, "fixture stdout\n");
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture and ARCNIX_TEST_FIXTURE"]
fn matching_default_host_key_keeps_its_pin() -> Result<()> {
    let (profile, _) = fixture_profile("client_key_ed25519")?;
    let initial = PendingConnection::open(&profile)?;
    let expected = initial.fingerprint().to_owned();
    drop(initial);
    let pending = PendingConnection::open_for_known_host(&profile, &expected)?;
    assert_eq!(pending.fingerprint(), expected);
    assert!(pending.authenticate("", &expected)?.sftp_available());
    Ok(())
}

#[test]
#[ignore = "requires audited loopback SSH fixture and ARCNIX_TEST_FIXTURE"]
fn changed_host_keys_are_rejected_without_sending_credentials() -> Result<()> {
    let (mut profile, fixture) = fixture_profile("client_key_ed25519")?;
    profile.username = format!("must-not-authenticate-{}", new_id());
    let expected = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let error = PendingConnection::open_for_known_host(&profile, expected)
        .err()
        .context("a different host key was accepted")?;
    let message = format!("{error:#}");
    assert!(message.contains("SSH host key changed"), "{message}");
    assert!(message.contains(expected));
    let audit = fs::read_to_string(fixture["auth_audit"].as_str().context("auth audit")?)?;
    assert!(
        !audit.contains(&profile.username),
        "credentials reached an untrusted host"
    );
    Ok(())
}
