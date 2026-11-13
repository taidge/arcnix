//! Requires tools/sftp_fixture.py and ARCNIX_TEST_SSH_ADDR=127.0.0.1:<port>.
//! Run with cargo test -p arcnix-core --test sftp_integration -- --ignored.
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use arcnix_core::remote::{PendingConnection, RemoteSession, TransferCancelled};
use arcnix_core::{AuthMethod, ServerProfile, new_id};
use sha2::{Digest, Sha256};

fn profile() -> Result<ServerProfile> {
    let addr: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")
        .context("Start loopback fixture and set ARCNIX_TEST_SSH_ADDR")?
        .parse()?;
    anyhow::ensure!(
        addr.ip().is_loopback(),
        "Integration fixture must use a loopback address"
    );
    Ok(ServerProfile {
        host: addr.ip().to_string(),
        port: addr.port(),
        username: "arcnix".into(),
        auth: AuthMethod::Password,
        ..Default::default()
    })
}
fn connect() -> Result<RemoteSession> {
    let pending = PendingConnection::open(&profile()?)?;
    let fingerprint = pending.fingerprint().to_owned();
    pending.authenticate("arcnix-test", &fingerprint)
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("arcnix-integration-{}", new_id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn roundtrip_unicode_listing_rename_and_no_overwrite() -> Result<()> {
    let session = connect()?;
    let (path, entries) = session.list_dir(".")?;
    assert_eq!(path, "/");
    assert!(entries.iter().any(|e| e.name == "中文 空格.txt"));
    assert!(entries.iter().any(|e| e.name == "folder" && e.is_dir));
    let remote_dir = format!("/test-{}", new_id());
    session.mkdir(&remote_dir)?;
    assert!(session.mkdir(&remote_dir).is_err());
    let temp = Temp::new()?;
    let source = temp.join("本地 空格.bin");
    let content: Vec<u8> = (0..300_000).map(|n| (n % 251) as u8).collect();
    fs::write(&source, &content)?;
    let remote = format!("{remote_dir}/上传 文件.bin");
    let cancel = AtomicBool::new(false);
    let mut final_progress = (0, 0);
    session.upload(&source, &remote, &cancel, |done, total| {
        final_progress = (done, total)
    })?;
    assert_eq!(final_progress, (content.len() as u64, content.len() as u64));
    assert!(
        session
            .upload(&source, &remote, &cancel, |_, _| {})
            .is_err()
    );
    let download = temp.join("下载.bin");
    session.download(&remote, &download, &cancel, |_, _| {})?;
    assert_eq!(
        Sha256::digest(fs::read(&download)?),
        Sha256::digest(&content)
    );
    fs::write(&download, b"must survive")?;
    assert!(
        session
            .download(&remote, &download, &cancel, |_, _| {})
            .is_err()
    );
    assert_eq!(fs::read(&download)?, b"must survive");
    assert!(session.rename(&remote, &remote).is_err());
    let renamed = format!("{remote_dir}/renamed.bin");
    session.rename(&remote, &renamed)?;
    let (_, entries) = session.list_dir(&remote_dir)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "renamed.bin");
    assert!(
        session
            .download(&remote_dir, &temp.join("directory"), &cancel, |_, _| {})
            .is_err()
    );
    let empty = temp.join("empty");
    fs::write(&empty, [])?;
    let empty_remote = format!("{remote_dir}/empty");
    let mut empty_progress = None;
    session.upload(&empty, &empty_remote, &cancel, |done, total| {
        empty_progress = Some((done, total))
    })?;
    assert_eq!(empty_progress, Some((0, 0)));
    let empty_download = temp.join("empty-download");
    session.download(&empty_remote, &empty_download, &cancel, |_, _| {})?;
    assert_eq!(fs::metadata(empty_download)?.len(), 0);
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn host_key_mismatch_and_bad_password_are_rejected() -> Result<()> {
    let pending = PendingConnection::open(&profile()?)?;
    assert!(pending.fingerprint().starts_with("SHA256:"));
    let result = pending.authenticate("arcnix-test", "SHA256:wrong");
    assert!(
        result
            .err()
            .context("Mismatch must fail")?
            .to_string()
            .contains("authentication was not attempted")
    );
    let pending = PendingConnection::open(&profile()?)?;
    let fingerprint = pending.fingerprint().to_owned();
    assert!(
        pending
            .authenticate("incorrect-password", &fingerprint)
            .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn cancelled_transfers_remove_partials_and_never_publish() -> Result<()> {
    let session = connect()?;
    let temp = Temp::new()?;
    let source = temp.join("source");
    fs::write(&source, vec![91u8; 512 * 1024])?;
    let remote_dir = format!("/cancel-{}", new_id());
    session.mkdir(&remote_dir)?;
    let remote = format!("{remote_dir}/target");
    let cancel = AtomicBool::new(false);
    let result = session.upload(&source, &remote, &cancel, |done, _| {
        if done > 0 {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert!(result.unwrap_err().is::<TransferCancelled>());
    assert!(session.list_dir(&remote_dir)?.1.is_empty());
    cancel.store(false, Ordering::Relaxed);
    session.upload(&source, &remote, &cancel, |_, _| {})?;
    let destination = temp.join("target");
    let result = session.download(&remote, &destination, &cancel, |done, _| {
        if done > 0 {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert!(result.unwrap_err().is::<TransferCancelled>());
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(&temp.0)?.count(), 1);
    assert!(
        session
            .upload(
                Path::new(&temp.0),
                &format!("{remote_dir}/dir"),
                &AtomicBool::new(false),
                |_, _| {}
            )
            .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn command_separates_output_and_exit_code() -> Result<()> {
    let session = connect()?;
    let result = session.exec("fixture-output")?;
    assert_eq!(result.stdout, "fixture stdout\n");
    assert_eq!(result.stderr, "fixture stderr\n");
    assert_eq!(result.exit_code, 7);
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture and ARCNIX_TEST_SSH_KEY"]
fn private_key_authentication() -> Result<()> {
    let mut profile = profile()?;
    profile.auth = AuthMethod::PrivateKey;
    profile.private_key = Some(
        std::env::var_os("ARCNIX_TEST_SSH_KEY")
            .context("Set ARCNIX_TEST_SSH_KEY to fixture client_key")?
            .into(),
    );
    let pending = PendingConnection::open(&profile)?;
    let fingerprint = pending.fingerprint().to_owned();
    let session = pending.authenticate("", &fingerprint)?;
    assert!(
        session
            .list_dir("/")?
            .1
            .iter()
            .any(|entry| entry.name == "welcome.txt")
    );
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn recursive_directory_roundtrip_and_no_merge() -> Result<()> {
    let session = connect()?;
    let temp = Temp::new()?;
    let source = temp.join("source");
    fs::create_dir_all(source.join("中文 空格/空目录"))?;
    let content: Vec<u8> = (0..180_000).map(|n| (n % 251) as u8).collect();
    fs::write(source.join("中文 空格/数据.bin"), &content)?;
    fs::write(source.join("empty"), [])?;
    let remote = format!("/recursive-{}", new_id());
    let cancel = AtomicBool::new(false);
    let mut previous = 0;
    session.upload_path(&source, &remote, &cancel, |done, total| {
        assert_eq!(total, content.len() as u64);
        assert!(done >= previous && done <= total);
        previous = done;
    })?;
    assert_eq!(previous, content.len() as u64);
    assert!(
        session
            .upload_path(&source, &remote, &cancel, |_, _| {})
            .is_err()
    );
    let destination = temp.join("destination");
    session.download_path(&remote, &destination, &cancel, |_, _| {})?;
    assert!(destination.join("中文 空格/空目录").is_dir());
    assert_eq!(
        Sha256::digest(fs::read(destination.join("中文 空格/数据.bin"))?),
        Sha256::digest(content)
    );
    assert_eq!(fs::metadata(destination.join("empty"))?.len(), 0);
    assert!(
        session
            .download_path(&remote, &destination, &cancel, |_, _| {})
            .is_err()
    );
    let empty_source = temp.join("empty-directory");
    fs::create_dir(&empty_source)?;
    let empty_remote = format!("/empty-directory-{}", new_id());
    session.upload_path(&empty_source, &empty_remote, &cancel, |done, total| {
        assert_eq!((done, total), (0, 0))
    })?;
    let empty_download = temp.join("empty-download");
    session.download_path(&empty_remote, &empty_download, &cancel, |_, _| {})?;
    assert!(empty_download.is_dir());
    assert_eq!(fs::read_dir(empty_download)?.count(), 0);
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn recursive_cancel_preserves_completed_files_without_partial_files() -> Result<()> {
    let session = connect()?;
    let temp = Temp::new()?;
    let source = temp.join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("a-completed"), b"complete")?;
    fs::write(source.join("b-cancelled"), vec![13u8; 300_000])?;
    let remote = format!("/recursive-cancel-{}", new_id());
    let cancel = AtomicBool::new(false);
    let result = session.upload_path(&source, &remote, &cancel, |done, _| {
        if done > 8 {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    let error = result.unwrap_err();
    assert!(error.is::<TransferCancelled>());
    assert!(error.to_string().contains("已完成文件保留"));
    let entries = session.list_dir(&remote)?.1;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a-completed");
    cancel.store(false, Ordering::Relaxed);
    let full_remote = format!("/recursive-full-{}", new_id());
    session.upload_path(&source, &full_remote, &cancel, |_, _| {})?;
    let destination = temp.join("destination");
    let result = session.download_path(&full_remote, &destination, &cancel, |done, _| {
        if done > 8 {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert!(result.unwrap_err().is::<TransferCancelled>());
    let entries = fs::read_dir(destination)?.collect::<std::io::Result<Vec<_>>>()?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].file_name(), "a-completed");
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn conflict_choices_overwrite_skip_and_cancel_both_directions() -> Result<()> {
    use arcnix_core::transfer::ConflictAction;
    let remote = connect()?;
    let root = Temp::new()?;
    let source = root.join("source.txt");
    let target = root.join("target.txt");
    let remote_path = format!("/conflict-{}.txt", new_id());
    let cancel = AtomicBool::new(false);
    fs::write(&source, b"original")?;
    remote.upload(&source, &remote_path, &cancel, |_, _| {})?;
    fs::write(&source, b"replacement data")?;
    let skipped = remote.upload_with_conflicts(
        &source,
        &remote_path,
        &cancel,
        |_, _| {},
        |conflict| {
            assert_eq!(conflict.source_size, 16);
            assert_eq!(conflict.destination_size, 8);
            Ok(ConflictAction::Skip)
        },
    )?;
    assert_eq!(skipped.skipped_files, 1);
    let error = remote
        .upload_with_conflicts(
            &source,
            &remote_path,
            &cancel,
            |_, _| {},
            |_| Ok(ConflictAction::Cancel),
        )
        .unwrap_err();
    assert!(error.is::<TransferCancelled>());
    remote.download(&remote_path, &target, &cancel, |_, _| {})?;
    assert_eq!(fs::read(&target)?, b"original");
    let done = remote.upload_with_conflicts(
        &source,
        &remote_path,
        &cancel,
        |_, _| {},
        |_| Ok(ConflictAction::Overwrite),
    )?;
    assert_eq!(done.transferred_files, 1);
    assert_eq!(
        remote
            .download_with_conflicts(
                &remote_path,
                &target,
                &cancel,
                |_, _| {},
                |_| Ok(ConflictAction::Skip)
            )?
            .skipped_files,
        1
    );
    assert_eq!(fs::read(&target)?, b"original");
    remote.download_with_conflicts(
        &remote_path,
        &target,
        &cancel,
        |_, _| {},
        |_| Ok(ConflictAction::Overwrite),
    )?;
    assert_eq!(fs::read(&target)?, b"replacement data");
    fs::write(&target, b"preserve me")?;
    assert!(
        remote
            .download_with_conflicts(
                &remote_path,
                &target,
                &cancel,
                |_, _| {},
                |_| Ok(ConflictAction::Cancel)
            )
            .is_err()
    );
    assert_eq!(fs::read(&target)?, b"preserve me");
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn cancelled_overwrite_preserves_original_and_late_collision_prompts() -> Result<()> {
    use arcnix_core::transfer::ConflictAction;
    let remote = connect()?;
    let root = Temp::new()?;
    let source = root.join("source");
    let target = root.join("target");
    let remote_path = format!("/cancel-overwrite-{}", new_id());
    let cancel = AtomicBool::new(false);
    fs::write(&source, b"keep-original")?;
    remote.upload(&source, &remote_path, &cancel, |_, _| {})?;
    fs::write(&source, vec![42; 256 * 1024])?;
    assert!(
        remote
            .upload_with_conflicts(
                &source,
                &remote_path,
                &cancel,
                |done, _| {
                    if done > 0 {
                        cancel.store(true, Ordering::Relaxed)
                    }
                },
                |_| Ok(ConflictAction::Overwrite)
            )
            .is_err()
    );
    cancel.store(false, Ordering::Relaxed);
    remote.download(&remote_path, &target, &cancel, |_, _| {})?;
    assert_eq!(fs::read(&target)?, b"keep-original");
    fs::remove_file(&target)?;
    let mut prompted = false;
    let result = remote.download_with_conflicts(
        &remote_path,
        &target,
        &cancel,
        |_, _| {
            if !target.exists() {
                fs::write(&target, b"racing-writer").unwrap();
            }
        },
        |_| {
            prompted = true;
            Ok(ConflictAction::Skip)
        },
    )?;
    assert!(prompted);
    assert_eq!(result.skipped_files, 1);
    assert_eq!(fs::read(&target)?, b"racing-writer");
    assert!(
        !fs::read_dir(&root.0)?.any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part"))
    );
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn recursive_merges_prompt_per_file_and_reject_type_conflicts() -> Result<()> {
    use arcnix_core::transfer::ConflictAction;
    let remote = connect()?;
    let root = Temp::new()?;
    let source = root.join("tree");
    let target = root.join("download");
    fs::create_dir(&source)?;
    fs::create_dir(source.join("nested"))?;
    fs::write(source.join("keep"), b"old")?;
    fs::write(source.join("nested/change"), b"old")?;
    let remote_path = format!("/merge-{}", new_id());
    let cancel = AtomicBool::new(false);
    remote.upload_path(&source, &remote_path, &cancel, |_, _| {})?;
    fs::write(source.join("keep"), b"new")?;
    fs::write(source.join("nested/change"), b"changed")?;
    fs::write(source.join("added"), b"added")?;
    let summary = remote.upload_path_with_conflicts(
        &source,
        &remote_path,
        &cancel,
        |_, _| {},
        |c| {
            Ok(if c.source_path.ends_with("keep") {
                ConflictAction::Skip
            } else {
                ConflictAction::Overwrite
            })
        },
    )?;
    assert_eq!(summary.skipped_files, 1);
    assert_eq!(summary.transferred_files, 2);
    remote.download_path(&remote_path, &target, &cancel, |_, _| {})?;
    assert_eq!(fs::read(target.join("keep"))?, b"old");
    assert_eq!(fs::read(target.join("nested/change"))?, b"changed");
    fs::write(target.join("keep"), b"local")?;
    let summary = remote.download_path_with_conflicts(
        &remote_path,
        &target,
        &cancel,
        |_, _| {},
        |_| Ok(ConflictAction::Overwrite),
    )?;
    assert_eq!(summary.transferred_files, 3);
    assert_eq!(fs::read(target.join("keep"))?, b"old");
    fs::remove_file(target.join("keep"))?;
    fs::create_dir(target.join("keep"))?;
    assert!(
        remote
            .download_path_with_conflicts(
                &remote_path,
                &target,
                &cancel,
                |_, _| {},
                |_| Ok(ConflictAction::Overwrite)
            )
            .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn overwrite_fallback_quotes_paths_and_failure_preserves_destination() -> Result<()> {
    use arcnix_core::transfer::ConflictAction;
    let remote = connect()?;
    let root = Temp::new()?;
    let cancel = AtomicBool::new(false);
    let source = root.join("source");
    let target = root.join("result");
    let strange = format!("/quote' $(literal); space-{}.txt", new_id());
    fs::write(&source, b"old")?;
    remote.upload(&source, &strange, &cancel, |_, _| {})?;
    fs::write(&source, b"new contents")?;
    remote.upload_with_conflicts(
        &source,
        &strange,
        &cancel,
        |_, _| {},
        |_| Ok(ConflictAction::Overwrite),
    )?;
    remote.download(&strange, &target, &cancel, |_, _| {})?;
    assert_eq!(fs::read(&target)?, b"new contents");
    let denied = format!("/fixture-replace-denied-{}", new_id());
    fs::write(&source, b"retain original")?;
    remote.upload(&source, &denied, &cancel, |_, _| {})?;
    fs::write(&source, b"replacement")?;
    assert!(
        remote
            .upload_with_conflicts(
                &source,
                &denied,
                &cancel,
                |_, _| {},
                |_| Ok(ConflictAction::Overwrite)
            )
            .is_err()
    );
    let verify = root.join("verify");
    remote.download(&denied, &verify, &cancel, |_, _| {})?;
    assert_eq!(fs::read(&verify)?, b"retain original");
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture with permission metadata"]
fn overwrite_retains_remote_permissions_and_chmod_failure_preserves_original() -> Result<()> {
    use arcnix_core::transfer::ConflictAction;
    let remote = connect()?;
    let root = Temp::new()?;
    let cancel = AtomicBool::new(false);
    let source = root.join("source");
    let verify = root.join("verify");
    let profile = profile()?;
    let mut connection = ssh2::Session::new()?;
    connection.set_tcp_stream(std::net::TcpStream::connect((
        profile.host.as_str(),
        profile.port,
    ))?);
    connection.handshake()?;
    connection.userauth_password("arcnix", "arcnix-test")?;
    let sftp = connection.sftp()?;
    for permissions in [0o644, 0o755] {
        let destination = format!("/retain-mode-{}-{permissions:o}", new_id());
        fs::write(&source, b"old")?;
        remote.upload(&source, &destination, &cancel, |_, _| {})?;
        sftp.setstat(
            Path::new(&destination),
            ssh2::FileStat {
                perm: Some(permissions),
                size: None,
                uid: None,
                gid: None,
                mtime: None,
                atime: None,
            },
        )?;
        assert_eq!(
            sftp.stat(Path::new(&destination))?.perm.unwrap() & 0o7777,
            permissions
        );
        fs::write(&source, b"replacement")?;
        remote.upload_with_conflicts(
            &source,
            &destination,
            &cancel,
            |_, _| {},
            |_| Ok(ConflictAction::Overwrite),
        )?;
        assert_eq!(
            sftp.stat(Path::new(&destination))?.perm.unwrap() & 0o7777,
            permissions
        );
    }
    let denied_directory = format!("/fixture-mode-denied-{}", new_id());
    remote.mkdir(&denied_directory)?;
    let denied = format!("{denied_directory}/entry");
    fs::write(&source, b"unchanged")?;
    remote.upload(&source, &denied, &cancel, |_, _| {})?;
    fs::write(&source, b"replacement")?;
    let error = remote
        .upload_with_conflicts(
            &source,
            &denied,
            &cancel,
            |_, _| {},
            |_| Ok(ConflictAction::Overwrite),
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("无法保留目标权限"));
    remote.download(&denied, &verify, &cancel, |_, _| {})?;
    assert_eq!(fs::read(verify)?, b"unchanged");
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn telemetry_separates_skipped_progress_from_payload_bytes_in_both_directions() -> Result<()> {
    use arcnix_core::transfer::ConflictAction;
    let session = connect()?;
    let temp = Temp::new()?;
    let source = temp.join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("skip.bin"), vec![1u8; 100_000])?;
    fs::write(source.join("send.bin"), vec![2u8; 200_000])?;
    fs::write(source.join("empty"), [])?;
    let dest = format!("/telemetry-{}", new_id());
    session.mkdir(&dest)?;
    session.upload(
        &source.join("skip.bin"),
        &format!("{dest}/skip.bin"),
        &AtomicBool::new(false),
        |_, _| {},
    )?;
    let mut last = (0, 0, 0);
    let report = session.upload_path_with_conflicts_stats(
        &source,
        &dest,
        &AtomicBool::new(false),
        |done, total, actual| {
            assert!(done >= last.0 && actual >= last.2 && done <= total && actual <= done);
            last = (done, total, actual);
        },
        |_| Ok(ConflictAction::Skip),
    )?;
    assert_eq!(last, (300_000, 300_000, 200_000));
    assert_eq!(report.skipped_files, 1);
    let target = temp.join("target");
    fs::create_dir(&target)?;
    fs::write(target.join("send.bin"), b"keep me")?;
    last = (0, 0, 0);
    session.download_path_with_conflicts_stats(
        &dest,
        &target,
        &AtomicBool::new(false),
        |done, total, actual| {
            assert!(done >= last.0 && actual >= last.2 && actual <= done);
            last = (done, total, actual);
        },
        |_| Ok(ConflictAction::Skip),
    )?;
    assert_eq!(last, (300_000, 300_000, 100_000));
    assert_eq!(fs::read(target.join("send.bin"))?, b"keep me");
    let mut actual = 1;
    session.upload_with_conflicts_stats(
        &source.join("skip.bin"),
        &format!("{dest}/skip.bin"),
        &AtomicBool::new(false),
        |_, _, bytes| actual = bytes,
        |_| Ok(ConflictAction::Skip),
    )?;
    assert_eq!(actual, 0);
    session.download_with_conflicts_stats(
        &format!("{dest}/skip.bin"),
        &target.join("skip.bin"),
        &AtomicBool::new(false),
        |_, _, bytes| actual = bytes,
        |_| Ok(ConflictAction::Skip),
    )?;
    assert_eq!(actual, 0);
    Ok(())
}
