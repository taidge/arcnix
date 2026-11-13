//! Real loopback SSH transfers with decisions delivered through the UI bridge.
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use std::{fs, thread};

use anyhow::{Context, Result, bail, ensure};
use arcnix_core::transfer::{ConflictAction, TransferConflict, TransferSummary};
use arcnix_core::{ServerProfile, TransferDirection, TransferJob, TransferStatus, new_id};

use crate::worker::{self, Action, ConflictDecision, ConflictScope, Request, Response, Update};

struct File(PathBuf);
impl File {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("arcnix-worker-conflict-{}.bin", new_id())))
    }
}
impl Drop for File {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct Bridge {
    tx: mpsc::Sender<Request>,
    rx: async_channel::Receiver<Response>,
    epoch: u64,
    serial: u64,
    server: String,
}
impl Bridge {
    fn connect() -> Result<Self> {
        let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")
            .context("start tools/sftp_fixture.py")?
            .parse()?;
        ensure!(address.ip().is_loopback(), "fixture must be loopback");
        let (tx, rx) = worker::start();
        let mut bridge = Self {
            tx,
            rx,
            epoch: 71,
            serial: 0,
            server: new_id(),
        };
        bridge.send(Action::Probe(
            ServerProfile {
                id: bridge.server.clone(),
                host: address.ip().to_string(),
                port: address.port(),
                username: "arcnix".into(),
                ..Default::default()
            },
            None,
        ))?;
        let fingerprint = match bridge.receive()?.update {
            Update::Fingerprint(value) => value,
            _ => bail!("expected fingerprint"),
        };
        bridge.send(Action::Authenticate {
            secret: "arcnix-test".into(),
            fingerprint,
        })?;
        ensure!(
            matches!(bridge.receive()?.update, Update::Connected(true)),
            "expected SFTP connection"
        );
        Ok(bridge)
    }
    fn send(&mut self, action: Action) -> Result<()> {
        self.serial += 1;
        self.tx
            .send(Request {
                epoch: self.epoch,
                serial: self.serial,
                action,
            })
            .map_err(|_| anyhow::anyhow!("worker request channel closed"))
    }
    fn receive(&self) -> Result<Response> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match self.rx.try_recv() {
                Ok(response) => return Ok(response),
                Err(async_channel::TryRecvError::Closed) => bail!("worker response channel closed"),
                Err(async_channel::TryRecvError::Empty) if Instant::now() >= deadline => {
                    bail!("worker response timed out")
                }
                Err(async_channel::TryRecvError::Empty) => thread::sleep(Duration::from_millis(10)),
            }
        }
    }
    fn transfer(
        &mut self,
        direction: TransferDirection,
        local: &File,
        remote: &str,
        cancel: Arc<AtomicBool>,
    ) -> Result<String> {
        let id = new_id();
        self.send(Action::TransferInteractive {
            job: TransferJob {
                id: id.clone(),
                server_id: self.server.clone(),
                direction,
                local_path: local.0.clone(),
                remote_path: remote.into(),
                total_bytes: 0,
                transferred_bytes: 0,
                status: TransferStatus::Queued,
            },
            cancel,
            policy: None,
        })?;
        Ok(id)
    }
    fn conflict(&self, id: &str) -> Result<(TransferConflict, mpsc::Sender<ConflictDecision>)> {
        loop {
            match self.receive()?.update {
                Update::TransferConflict(actual, info, reply) => {
                    ensure!(actual == id, "conflict belongs to wrong job");
                    return Ok((info, reply));
                }
                Update::Progress(..) => {}
                Update::Finished(_, result) => bail!("transfer finished before prompt: {result:?}"),
                _ => bail!("unexpected event before collision"),
            }
        }
    }
    fn finished(&self, id: &str, success: bool) -> Result<Option<TransferSummary>> {
        let mut report = None;
        let mut last_progress = 0;
        loop {
            match self.receive()?.update {
                Update::Progress(actual, done, total, _) => {
                    ensure!(
                        actual == id && done >= last_progress && done <= total,
                        "invalid progress"
                    );
                    last_progress = done;
                }
                Update::TransferReport(actual, summary) => {
                    ensure!(
                        actual == id && report.is_none(),
                        "duplicate or wrong transfer report"
                    );
                    report = Some(summary);
                }
                Update::Finished(actual, result) => {
                    ensure!(actual == id, "finished belongs to wrong job");
                    if success {
                        result.map_err(anyhow::Error::msg)?;
                        ensure!(report.is_some(), "successful transfer needs summary");
                    } else {
                        ensure!(result.is_err(), "cancelled/stale transfer must fail");
                    }
                    return Ok(report);
                }
                Update::ConnectionLost(error) => bail!("unexpected connection loss: {error}"),
                _ => bail!("unexpected transfer event"),
            }
        }
    }
}
fn uncancelled() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
#[ignore = "requires tools/sftp_fixture.py and ARCNIX_TEST_SSH_ADDR"]
fn interactive_transfer_pauses_for_skip_and_overwrite_then_reports_real_bytes() -> Result<()> {
    let mut bridge = Bridge::connect()?;
    let local = File::new();
    let remote = format!("/worker-conflict-{}.bin", new_id());
    fs::write(&local.0, vec![17u8; 1024])?;
    let initial = bridge.transfer(TransferDirection::Upload, &local, &remote, uncancelled())?;
    assert_eq!(
        bridge.finished(&initial, true)?.unwrap(),
        TransferSummary {
            transferred_files: 1,
            skipped_files: 0
        }
    );

    let changed = vec![91u8; 4096];
    fs::write(&local.0, &changed)?;
    let skip = bridge.transfer(TransferDirection::Upload, &local, &remote, uncancelled())?;
    let (info, reply) = bridge.conflict(&skip)?;
    assert_eq!(info.direction, TransferDirection::Upload);
    assert_eq!(info.source_size, 4096);
    assert_eq!(info.destination_size, 1024);
    assert_eq!(info.destination_path, remote);
    reply.send(ConflictDecision {
        action: ConflictAction::Skip,
        scope: ConflictScope::ThisFile,
    })?;
    assert_eq!(
        bridge.finished(&skip, true)?.unwrap(),
        TransferSummary {
            transferred_files: 0,
            skipped_files: 1
        }
    );

    let overwrite = bridge.transfer(TransferDirection::Upload, &local, &remote, uncancelled())?;
    let (info, reply) = bridge.conflict(&overwrite)?;
    assert_eq!(
        info.destination_size, 1024,
        "skipping must preserve old bytes"
    );
    reply.send(ConflictDecision {
        action: ConflictAction::Overwrite,
        scope: ConflictScope::ThisFile,
    })?;
    assert_eq!(
        bridge
            .finished(&overwrite, true)?
            .unwrap()
            .transferred_files,
        1
    );
    let downloaded = File::new();
    let download = bridge.transfer(
        TransferDirection::Download,
        &downloaded,
        &remote,
        uncancelled(),
    )?;
    bridge.finished(&download, true)?;
    assert_eq!(fs::read(&downloaded.0)?, changed);
    Ok(())
}

#[test]
#[ignore = "requires tools/sftp_fixture.py and ARCNIX_TEST_SSH_ADDR"]
fn pending_conflict_releases_worker_on_cancel_and_connection_switch() -> Result<()> {
    let mut bridge = Bridge::connect()?;
    let local = File::new();
    fs::write(&local.0, b"preserved fixture content")?;
    let remote = format!("/worker-cancel-{}.bin", new_id());
    let initial = bridge.transfer(TransferDirection::Upload, &local, &remote, uncancelled())?;
    bridge.finished(&initial, true)?;

    let cancel = uncancelled();
    let cancelled = bridge.transfer(TransferDirection::Upload, &local, &remote, cancel.clone())?;
    let (_, still_open_reply) = bridge.conflict(&cancelled)?;
    cancel.store(true, Ordering::Release);
    bridge.finished(&cancelled, false)?;
    assert!(
        still_open_reply
            .send(ConflictDecision {
                action: ConflictAction::Overwrite,
                scope: ConflictScope::ThisFile
            })
            .is_err()
    );
    bridge.send(Action::Remote("/".into()))?;
    ensure!(
        matches!(bridge.receive()?.update, Update::Remote(_, _)),
        "cancelled prompt must release SSH worker"
    );

    let pending = bridge.transfer(TransferDirection::Upload, &local, &remote, uncancelled())?;
    let (_, late_reply) = bridge.conflict(&pending)?;
    let old_epoch = bridge.epoch;
    bridge.epoch += 1;
    bridge.send(Action::Disconnect)?;
    bridge.finished(&pending, false)?;
    let disconnected = bridge.receive()?;
    assert_eq!(disconnected.epoch, bridge.epoch);
    ensure!(
        matches!(disconnected.update, Update::Disconnected),
        "disconnect must not wait for user decision"
    );
    assert!(
        late_reply
            .send(ConflictDecision {
                action: ConflictAction::Overwrite,
                scope: ConflictScope::ThisFile
            })
            .is_err()
    );
    bridge.epoch = old_epoch;
    let stale = bridge.transfer(TransferDirection::Upload, &local, &remote, uncancelled())?;
    bridge.finished(&stale, false)?;
    Ok(())
}
