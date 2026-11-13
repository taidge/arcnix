use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use arcnix_core::metrics::{self, ResourceSnapshot};
use arcnix_core::packages::{self, PackageAction, PackagePlan, SystemEnvironment};
use arcnix_core::remote::{PendingConnection, RemoteSession};
use arcnix_core::system_logs::{self, LogCapabilities, LogQuery, LogSnapshot};
use arcnix_core::{local, *};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictScope {
    ThisFile,
    ThisQueue,
    ThisSession,
    FutureConnections,
}
#[derive(Clone, Copy, Debug)]
pub struct ConflictDecision {
    pub action: arcnix_core::transfer::ConflictAction,
    pub scope: ConflictScope,
}

pub enum Action {
    Terminal(crate::terminal_worker::Action),
    Probe(ServerProfile, Option<String>),
    Authenticate {
        secret: String,
        fingerprint: String,
    },
    Disconnect,
    Local(PathBuf),
    Remote(String),
    Children {
        remote: bool,
        path: String,
    },
    LocalMkdir(PathBuf),
    RemoteMkdir(String),
    LocalRename(PathBuf, PathBuf),
    ImportSshConfig(PathBuf),
    RemoteRename(String, String),
    TransferInteractive {
        job: TransferJob,
        cancel: Arc<AtomicBool>,
        policy: Option<arcnix_core::transfer::ConflictAction>,
    },
    PackageDetect,
    PackageSearch(String),
    PackageInstalled,
    PackagePreview(PackageAction),
    PackageApply {
        plan_id: u64,
        secret: String,
    },
    MetricsSample,
    LogsDiscover,
    LogsRead(LogQuery),
}
pub struct Request {
    pub epoch: u64,
    pub serial: u64,
    pub action: Action,
}
#[cfg(test)]
impl Action {
    fn transfer(job: TransferJob, cancel: Arc<AtomicBool>) -> Self {
        Self::TransferInteractive {
            job,
            cancel,
            policy: None,
        }
    }
}
pub enum Update {
    Terminal(crate::terminal_worker::Event),
    Fingerprint(String),
    Connected(bool),
    Disconnected,
    Local(PathBuf, Vec<FileEntry>),
    Remote(String, Vec<FileEntry>),
    Children {
        remote: bool,
        parent: String,
        result: Result<Vec<FileEntry>, String>,
    },
    Progress(String, u64, u64, u64),
    Finished(String, Result<(), String>),
    Log(String),
    Error(String),
    ConnectionError(String),
    Imported(arcnix_core::ssh_config::ImportReport),
    BrowseError {
        remote: bool,
        message: String,
    },
    ConnectionLost(String),
    PackageEnvironment(Result<SystemEnvironment, String>),
    PackageList(Result<Vec<packages::Package>, String>),
    PackagePreview(Result<PackagePlan, String>),
    PackageStarted,
    PackageOutput(String),
    PackageFinished(Result<CommandOutput, String>),
    Metrics(Result<ResourceSnapshot, String>),
    LogCapabilities(Result<LogCapabilities, String>),
    SystemLogs(Result<LogSnapshot, String>),
    TransferConflict(
        String,
        arcnix_core::transfer::TransferConflict,
        mpsc::Sender<ConflictDecision>,
    ),
    TransferReport(String, arcnix_core::transfer::TransferSummary),
}
pub struct Response {
    pub epoch: u64,
    pub serial: u64,
    pub update: Update,
}
pub fn start() -> (mpsc::Sender<Request>, async_channel::Receiver<Response>) {
    let (tx, rx) = mpsc::channel::<Request>();
    let (events, receive) = async_channel::bounded(128);
    let (local_tx, local_rx) = mpsc::channel::<Request>();
    let (remote_tx, remote_rx) = mpsc::channel::<Request>();
    let generation = Arc::new(AtomicU64::new(0));
    let router_generation = generation.clone();
    let terminal = crate::terminal_worker::start(events.clone(), generation.clone());
    let routed_terminal = terminal.clone();
    // Local filesystem work must stay available during a large SFTP transfer.
    thread::spawn(move || {
        for request in rx {
            match request.action {
                Action::Terminal(action) => {
                    let _ = routed_terminal.send(crate::terminal_worker::Command::Request {
                        epoch: request.epoch,
                        serial: request.serial,
                        action,
                    });
                }
                Action::Local(_)
                | Action::Children { remote: false, .. }
                | Action::LocalMkdir(_)
                | Action::LocalRename(..)
                | Action::ImportSshConfig(_) => {
                    if local_tx.send(request).is_err() {
                        break;
                    }
                }
                _ => {
                    if matches!(request.action, Action::Probe(..) | Action::Disconnect) {
                        router_generation.store(request.epoch, Ordering::Release);
                        let _ = routed_terminal.send(crate::terminal_worker::Command::Reset);
                    }
                    if remote_tx.send(request).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let local_events = events.clone();
    thread::spawn(move || {
        for Request {
            epoch,
            serial,
            action,
        } in local_rx
        {
            let browsing = matches!(action, Action::Local(_));
            let result = match action {
                Action::Local(path) => {
                    local::list_dir(&path).map(|(path, entries)| Update::Local(path, entries))
                }
                Action::Children {
                    remote: false,
                    path,
                } => Ok(Update::Children {
                    remote: false,
                    parent: path.clone(),
                    result: local::list_dir(Path::new(&path))
                        .map(|(_, entries)| entries)
                        .map_err(|e| format!("{e:#}")),
                }),
                Action::LocalMkdir(path) => {
                    local::mkdir(&path).map(|_| Update::Log("本地目录已创建".into()))
                }
                Action::LocalRename(from, to) => {
                    local::rename(&from, &to).map(|_| Update::Log("本地文件已重命名".into()))
                }
                Action::ImportSshConfig(path) => {
                    arcnix_core::ssh_config::import_file(&path).map(Update::Imported)
                }
                _ => unreachable!(),
            };
            let update = result.unwrap_or_else(|error| {
                if browsing {
                    Update::BrowseError {
                        remote: false,
                        message: format!("{error:#}"),
                    }
                } else {
                    Update::Error(format!("{error:#}"))
                }
            });
            if local_events
                .send_blocking(Response {
                    epoch,
                    serial,
                    update,
                })
                .is_err()
            {
                break;
            }
        }
    });
    thread::spawn(move || {
        let mut pending: Option<PendingConnection> = None;
        let mut session: Option<RemoteSession> = None;
        let mut active_epoch = 0;
        let mut active_server = String::new();
        let mut active_profile: Option<ServerProfile> = None;
        let mut package_environment: Option<SystemEnvironment> = None;
        let mut package_plan: Option<(u64, PackagePlan)> = None;
        for Request {
            epoch,
            serial,
            action,
        } in remote_rx
        {
            let send = |update| {
                let _ = events.send_blocking(Response {
                    epoch,
                    serial,
                    update,
                });
            };
            if matches!(
                action,
                Action::MetricsSample | Action::LogsDiscover | Action::LogsRead(_)
            ) {
                let kind = match &action {
                    Action::MetricsSample => 0,
                    Action::LogsDiscover => 1,
                    _ => 2,
                };
                let outcome: anyhow::Result<()> = (|| {
                    anyhow::ensure!(
                        epoch == generation.load(Ordering::Acquire) && epoch == active_epoch,
                        "连接已切换，采集已停止"
                    );
                    let remote = session
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("请先连接服务器"))?;
                    send(match action {
                        Action::MetricsSample => Update::Metrics(Ok(metrics::sample(remote)?)),
                        Action::LogsDiscover => {
                            Update::LogCapabilities(Ok(system_logs::discover(remote)?))
                        }
                        Action::LogsRead(query) => {
                            Update::SystemLogs(Ok(system_logs::read(remote, &query)?))
                        }
                        _ => unreachable!(),
                    });
                    Ok(())
                })();
                if let Err(error) = outcome {
                    if arcnix_core::transport_error::is_connection_error(&error) {
                        session = None;
                        pending = None;
                        package_environment = None;
                        package_plan = None;
                        send(Update::ConnectionLost(format!("{error:#}")));
                    }
                    let message = format!("{error:#}");
                    send(match kind {
                        0 => Update::Metrics(Err(message)),
                        1 => Update::LogCapabilities(Err(message)),
                        _ => Update::SystemLogs(Err(message)),
                    });
                }
                continue;
            }
            if matches!(
                action,
                Action::PackageDetect
                    | Action::PackageSearch(_)
                    | Action::PackageInstalled
                    | Action::PackagePreview(_)
                    | Action::PackageApply { .. }
            ) {
                let kind = match &action {
                    Action::PackageDetect => 0,
                    Action::PackageSearch(_) | Action::PackageInstalled => 1,
                    Action::PackagePreview(_) => 2,
                    _ => 3,
                };
                let outcome: anyhow::Result<()> = (|| {
                    anyhow::ensure!(
                        epoch == generation.load(Ordering::Acquire) && epoch == active_epoch,
                        "连接已切换，系统软件操作已停止"
                    );
                    let remote = session
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("请先连接服务器"))?;
                    if package_environment.is_none() || matches!(action, Action::PackageDetect) {
                        package_plan = None;
                        package_environment = Some(packages::detect(remote)?);
                    }
                    let environment = package_environment.as_ref().expect("environment detected");
                    match action {
                        Action::PackageDetect => {
                            send(Update::PackageEnvironment(Ok(environment.clone())))
                        }
                        Action::PackageSearch(query) => send(Update::PackageList(Ok(
                            packages::search(remote, environment, &query)?,
                        ))),
                        Action::PackageInstalled => send(Update::PackageList(Ok(
                            packages::installed(remote, environment)?,
                        ))),
                        Action::PackagePreview(action) => {
                            package_plan = None;
                            let plan = packages::preview(remote, environment, action)?;
                            package_plan = Some((serial, plan.clone()));
                            send(Update::PackagePreview(Ok(plan)));
                        }
                        Action::PackageApply { plan_id, secret } => {
                            let (approved_id, plan) = package_plan
                                .take()
                                .ok_or_else(|| anyhow::anyhow!("安装预览已失效，请重新预览操作"))?;
                            anyhow::ensure!(
                                approved_id == plan_id,
                                "安装预览不属于当前任务，请重新预览"
                            );
                            let execution =
                                packages::build_execution(environment, &plan, !secret.is_empty())?;
                            anyhow::ensure!(
                                secret.len() <= 1024 && !secret.contains(['\n', '\r', '\0']),
                                "管理员密码格式无效"
                            );
                            let mut bytes = secret.into_bytes();
                            let mut stdout = crate::secret_output::SecretOutput::new(&bytes);
                            let mut stderr = crate::secret_output::SecretOutput::new(&bytes);
                            let mut input = if execution.use_stdin_password {
                                bytes.push(b'\n');
                                Some(bytes)
                            } else {
                                bytes.fill(0);
                                None
                            };
                            send(Update::PackageStarted);
                            send(Update::PackageOutput(format!(
                                "$ {}\n",
                                crate::command_display(&execution.command)
                            )));
                            let result = remote.exec_stream(
                                &execution.command,
                                input.as_deref(),
                                std::time::Duration::from_secs(1800),
                                |stream, bytes| {
                                    let text = match stream {
                                        arcnix_core::remote::CommandStream::Stdout => {
                                            stdout.push(bytes)
                                        }
                                        arcnix_core::remote::CommandStream::Stderr => {
                                            stderr.push(bytes)
                                        }
                                    };
                                    if !text.is_empty() {
                                        send(Update::PackageOutput(text));
                                    }
                                },
                            );
                            for text in [stdout.finish(), stderr.finish()] {
                                if !text.is_empty() {
                                    send(Update::PackageOutput(text));
                                }
                            }
                            if let Some(bytes) = input.as_mut() {
                                bytes.fill(0);
                            }
                            // The page receives output through the filtered stream only;
                            // do not expose an unfiltered retained tail in the final event.
                            let mut output = result?;
                            output.stdout.clear();
                            output.stderr.clear();
                            send(Update::PackageFinished(Ok(output)));
                        }
                        _ => unreachable!(),
                    }
                    Ok(())
                })();
                if let Err(error) = outcome {
                    if arcnix_core::transport_error::is_connection_error(&error) {
                        session = None;
                        pending = None;
                        package_environment = None;
                        package_plan = None;
                        send(Update::ConnectionLost(format!("{error:#}")));
                    }
                    let message = format!("{error:#}");
                    send(match kind {
                        0 => Update::PackageEnvironment(Err(message)),
                        1 => Update::PackageList(Err(message)),
                        2 => Update::PackagePreview(Err(message)),
                        _ => Update::PackageFinished(Err(message)),
                    });
                }
                continue;
            }
            // Every accepted transfer gets its terminal event, including a
            // disconnected/stale session. Otherwise the queue stays Running.
            if let Action::TransferInteractive {
                job,
                cancel,
                mut policy,
            } = action
            {
                let result: anyhow::Result<()> = (|| {
                    anyhow::ensure!(
                        epoch == generation.load(Ordering::Acquire) && epoch == active_epoch,
                        "连接已切换，传输已停止"
                    );
                    anyhow::ensure!(job.server_id == active_server, "服务器会话已变化，拒绝传输");
                    let remote = session
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("连接已断开，请重新连接后重试"))?;
                    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
                    let latest = std::cell::Cell::new(None);
                    let progress = |done, total, actual| {
                        latest.set(Some((done, total, actual)));
                        if done == total || last.elapsed().as_millis() >= 100 {
                            send(Update::Progress(job.id.clone(), done, total, actual));
                            last = std::time::Instant::now();
                        }
                    };
                    let mut resolve = |conflict:&arcnix_core::transfer::TransferConflict| -> anyhow::Result<arcnix_core::transfer::ConflictAction> {
                            use arcnix_core::transfer::ConflictAction;
                            if let Some(action) = policy { return Ok(action); }
                            let (reply,answer) = mpsc::channel();
                            send(Update::TransferConflict(job.id.clone(), conflict.clone(),reply));
                            loop {
                                if cancel.load(Ordering::Relaxed) || epoch != generation.load(Ordering::Acquire) {
                                    return Err(arcnix_core::remote::TransferCancelled.into());
                                }
                                match answer.recv_timeout(std::time::Duration::from_millis(100)) {
                                    Ok(decision) => {
                                        if decision.action == ConflictAction::Cancel {
                                            cancel.store(true,Ordering::Relaxed);
                                        } else if decision.scope != ConflictScope::ThisFile {
                                            policy = Some(decision.action);
                                        }
                                        return Ok(decision.action);
                                    }
                                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(ConflictAction::Cancel),
                                }
                            }
                        };
                    let summary = match job.direction {
                        TransferDirection::Upload => remote.upload_path_with_conflicts_stats(
                            &job.local_path,
                            &job.remote_path,
                            &cancel,
                            progress,
                            &mut resolve,
                        ),
                        TransferDirection::Download => remote.download_path_with_conflicts_stats(
                            &job.remote_path,
                            &job.local_path,
                            &cancel,
                            progress,
                            &mut resolve,
                        ),
                    };
                    if let Some((done, total, actual)) = latest.get() {
                        send(Update::Progress(job.id.clone(), done, total, actual));
                    }
                    let summary = summary?;
                    send(Update::TransferReport(job.id.clone(), summary));
                    Ok(())
                })();
                let lost = result
                    .as_ref()
                    .err()
                    .filter(|e| arcnix_core::transport_error::is_connection_error(e))
                    .map(|e| format!("{e:#}"));
                // Publish connection loss before Finished so the UI cannot
                // schedule another transfer on a known dead session.
                if let Some(message) = lost {
                    session = None;
                    pending = None;
                    send(Update::ConnectionLost(message));
                }
                send(Update::Finished(
                    job.id,
                    result.map_err(|e| format!("{e:#}")),
                ));
                continue;
            }
            let browsing = matches!(action, Action::Remote(_));
            let child_path = match &action {
                Action::Children { remote: true, path } => Some(path.clone()),
                _ => None,
            };
            let connecting = matches!(action, Action::Probe(..) | Action::Authenticate { .. });
            let had_session = session.is_some();
            let result: anyhow::Result<()> = (|| {
                anyhow::ensure!(
                    epoch == generation.load(Ordering::Acquire),
                    "连接已切换，操作已忽略"
                );
                match action {
                    Action::Probe(profile, expected) => {
                        package_environment = None;
                        package_plan = None;
                        active_epoch = epoch;
                        active_server = profile.id.clone();
                        active_profile = Some(profile.clone());
                        session = None;
                        pending = None;
                        let connection = match expected {
                            Some(expected) => {
                                PendingConnection::open_for_known_host(&profile, &expected)?
                            }
                            None => PendingConnection::open(&profile)?,
                        };
                        let fingerprint = connection.fingerprint().to_string();
                        pending = Some(connection);
                        send(Update::Fingerprint(fingerprint));
                    }
                    Action::Authenticate {
                        secret,
                        fingerprint,
                    } => {
                        let secret = zeroize::Zeroizing::new(secret);
                        if epoch != active_epoch {
                            return Ok(());
                        }
                        let connection = pending
                            .take()
                            .ok_or_else(|| anyhow::anyhow!("连接已失效，请重新连接"))?;
                        session = Some(connection.authenticate(&secret, &fingerprint)?);
                        if let Some(profile) = active_profile.clone() {
                            let _ = terminal.send(crate::terminal_worker::Command::Attach {
                                epoch,
                                profile,
                                fingerprint,
                                secret,
                            });
                        }
                        send(Update::Connected(
                            session.as_ref().is_some_and(|s| s.sftp_available()),
                        ));
                    }
                    Action::Disconnect => {
                        package_environment = None;
                        package_plan = None;
                        active_epoch = epoch;
                        session = None;
                        pending = None;
                        send(Update::Disconnected);
                    }
                    action => {
                        if epoch != active_epoch {
                            anyhow::bail!("连接已切换，操作已忽略");
                        }
                        let remote = session
                            .as_mut()
                            .ok_or_else(|| anyhow::anyhow!("请先连接服务器"))?;
                        match action {
                            Action::Remote(path) => {
                                let (path, entries) = remote.list_dir(&path)?;
                                send(Update::Remote(path, entries));
                            }
                            Action::Children { remote: true, path } => {
                                let result = remote.list_dir(&path).map(|(_, entries)| entries);
                                match result {
                                    Ok(entries) => send(Update::Children {
                                        remote: true,
                                        parent: path,
                                        result: Ok(entries),
                                    }),
                                    Err(error)
                                        if arcnix_core::transport_error::is_connection_error(
                                            &error,
                                        ) =>
                                    {
                                        return Err(error);
                                    }
                                    Err(error) => send(Update::Children {
                                        remote: true,
                                        parent: path,
                                        result: Err(format!("{error:#}")),
                                    }),
                                }
                            }
                            Action::RemoteMkdir(path) => {
                                remote.mkdir(&path)?;
                                send(Update::Log("远程目录已创建".into()));
                            }
                            Action::RemoteRename(from, to) => {
                                remote.rename(&from, &to)?;
                                send(Update::Log("远程文件已重命名".into()));
                            }
                            _ => unreachable!(),
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                if had_session && arcnix_core::transport_error::is_connection_error(&error) {
                    session = None;
                    pending = None;
                    send(Update::ConnectionLost(format!("{error:#}")));
                } else if let Some(parent) = child_path {
                    send(Update::Children {
                        remote: true,
                        parent,
                        result: Err(format!("{error:#}")),
                    });
                } else if browsing {
                    send(Update::BrowseError {
                        remote: true,
                        message: format!("{error:#}"),
                    });
                } else if connecting {
                    send(Update::ConnectionError(format!("{error:#}")));
                } else {
                    send(Update::Error(format!("{error:#}")));
                }
            }
        }
    });
    (tx, receive)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result};

    use super::*;

    fn receive(rx: &async_channel::Receiver<Response>) -> Result<Response> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match rx.try_recv() {
                Ok(response) => return Ok(response),
                Err(async_channel::TryRecvError::Closed) => {
                    anyhow::bail!("worker event channel closed")
                }
                Err(async_channel::TryRecvError::Empty) if Instant::now() >= deadline => {
                    anyhow::bail!("worker event timed out")
                }
                Err(async_channel::TryRecvError::Empty) => thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    #[test]
    fn disconnected_observation_requests_have_terminal_results() -> Result<()> {
        let (tx, rx) = start();
        for (serial, action) in [
            Action::MetricsSample,
            Action::LogsDiscover,
            Action::LogsRead(LogQuery::default()),
        ]
        .into_iter()
        .enumerate()
        {
            tx.send(Request {
                epoch: 0,
                serial: serial as u64 + 1,
                action,
            })
            .map_err(|_| anyhow::anyhow!("worker stopped"))?;
            let response = receive(&rx)?;
            assert_eq!(response.serial, serial as u64 + 1);
            assert!(matches!(
                response.update,
                Update::Metrics(Err(_))
                    | Update::LogCapabilities(Err(_))
                    | Update::SystemLogs(Err(_))
            ));
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires loopback SSH fixture"]
    fn observation_bridge_uses_ssh_without_sftp_and_rejects_stale_session() -> Result<()> {
        let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
        anyhow::ensure!(address.ip().is_loopback(), "fixture must be loopback");
        let (tx, rx) = start();
        let send = |epoch, serial, action| {
            tx.send(Request {
                epoch,
                serial,
                action,
            })
            .map_err(|_| anyhow::anyhow!("worker stopped"))
        };
        send(
            88,
            1,
            Action::Probe(
                ServerProfile {
                    host: address.ip().to_string(),
                    port: address.port(),
                    username: "arcnix-nosftp".into(),
                    ..Default::default()
                },
                None,
            ),
        )?;
        let fingerprint = match receive(&rx)?.update {
            Update::Fingerprint(value) => value,
            _ => anyhow::bail!("expected fingerprint"),
        };
        send(
            88,
            2,
            Action::Authenticate {
                secret: "arcnix-test".into(),
                fingerprint,
            },
        )?;
        assert!(matches!(receive(&rx)?.update, Update::Connected(false)));
        send(88, 3, Action::MetricsSample)?;
        let first = match receive(&rx)?.update {
            Update::Metrics(Ok(snapshot)) => snapshot,
            _ => anyhow::bail!("expected resource snapshot"),
        };
        send(88, 4, Action::MetricsSample)?;
        let second = match receive(&rx)?.update {
            Update::Metrics(Ok(snapshot)) => snapshot,
            _ => anyhow::bail!("expected second resource snapshot"),
        };
        let usage = metrics::usage(&second, Some(&first));
        assert_eq!(usage.cpu_percent, Some(30.0));
        assert_eq!(usage.receive_bytes_per_second, Some(1024.0));
        send(88, 5, Action::LogsDiscover)?;
        assert!(matches!(
            receive(&rx)?.update,
            Update::LogCapabilities(Ok(_))
        ));
        send(88, 6, Action::LogsRead(LogQuery::default()))?;
        match receive(&rx)?.update {
            Update::SystemLogs(Ok(snapshot)) => assert_eq!(snapshot.entries.len(), 2),
            _ => anyhow::bail!("expected journal snapshot"),
        }
        send(89, 7, Action::Disconnect)?;
        assert!(matches!(receive(&rx)?.update, Update::Disconnected));
        send(88, 8, Action::MetricsSample)?;
        let stale = receive(&rx)?;
        assert_eq!(stale.epoch, 88);
        assert!(matches!(stale.update, Update::Metrics(Err(_))));
        Ok(())
    }

    #[test]
    #[ignore = "requires tools/sftp_fixture.py and ARCNIX_TEST_SSH_ADDR"]
    fn worker_bridge_authenticates_browses_and_rejects_wrong_server() -> Result<()> {
        let addr: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")
            .context("start loopback fixture and set ARCNIX_TEST_SSH_ADDR")?
            .parse()?;
        anyhow::ensure!(addr.ip().is_loopback(), "fixture must be loopback-only");
        let profile = ServerProfile {
            id: "worker-fixture".into(),
            name: "Worker integration".into(),
            host: addr.ip().to_string(),
            port: addr.port(),
            username: "arcnix".into(),
            auth: AuthMethod::Password,
            ..Default::default()
        };
        let (tx, rx) = start();
        let send = |epoch, action| {
            tx.send(Request {
                epoch,
                serial: epoch,
                action,
            })
            .map_err(|_| anyhow::anyhow!("worker request channel closed"))
        };
        send(77, Action::Probe(profile, None))?;
        let response = receive(&rx)?;
        assert_eq!(response.epoch, 77);
        let fingerprint = match response.update {
            Update::Fingerprint(value) => value,
            Update::Error(error) => anyhow::bail!("probe: {error}"),
            _ => anyhow::bail!("expected pending fingerprint before authentication"),
        };
        assert!(fingerprint.starts_with("SHA256:"));
        send(
            77,
            Action::Authenticate {
                secret: "arcnix-test".into(),
                fingerprint,
            },
        )?;
        assert!(matches!(receive(&rx)?.update, Update::Connected(true)));
        send(77, Action::Remote("/".into()))?;
        match receive(&rx)?.update {
            Update::Remote(path, entries) => {
                assert_eq!(path, "/");
                assert!(entries.iter().any(|entry| entry.name == "中文 空格.txt"));
            }
            Update::Error(error) => anyhow::bail!("browse: {error}"),
            _ => anyhow::bail!("expected directory listing"),
        }
        let id = new_id();
        send(
            77,
            Action::transfer(
                TransferJob {
                    id: id.clone(),
                    server_id: "different-server".into(),
                    direction: TransferDirection::Upload,
                    local_path: "nonexistent-worker-source".into(),
                    remote_path: "/must-not-be-written".into(),
                    total_bytes: 0,
                    transferred_bytes: 0,
                    status: TransferStatus::Queued,
                },
                Arc::new(AtomicBool::new(false)),
            ),
        )?;
        match receive(&rx)?.update {
            Update::Finished(received, Err(error)) => {
                assert_eq!(received, id);
                assert!(error.contains("服务器会话已变化"));
            }
            _ => anyhow::bail!("wrong-server transfer was not rejected"),
        }
        send(76, Action::RemoteMkdir("/must-not-be-created".into()))?;
        let stale = receive(&rx)?;
        assert_eq!(stale.epoch, 76);
        assert!(matches!(stale.update, Update::Error(_)));
        send(77, Action::Remote("/__fixture_disconnect__".into()))?;
        let lost = receive(&rx)?;
        assert!(
            matches!(lost.update, Update::ConnectionLost(_)),
            "socket loss must clear connected state"
        );
        send(
            77,
            Action::transfer(
                TransferJob {
                    id: "after-loss".into(),
                    server_id: "worker-fixture".into(),
                    direction: TransferDirection::Download,
                    local_path: "must-not-be-written".into(),
                    remote_path: "/welcome.txt".into(),
                    total_bytes: 0,
                    transferred_bytes: 0,
                    status: TransferStatus::Queued,
                },
                Arc::new(AtomicBool::new(false)),
            ),
        )?;
        assert!(matches!(receive(&rx)?.update, Update::Finished(id, Err(_)) if id == "after-loss"));
        send(78, Action::Disconnect)?;
        let disconnected = receive(&rx)?;
        assert_eq!(disconnected.epoch, 78);
        assert!(matches!(disconnected.update, Update::Disconnected));
        Ok(())
    }

    #[test]
    fn disconnected_and_stale_transfers_always_finish() -> Result<()> {
        let (tx, rx) = start();
        tx.send(Request {
            epoch: 3,
            serial: 1,
            action: Action::Disconnect,
        })
        .map_err(|_| anyhow::anyhow!("worker stopped"))?;
        let job = TransferJob {
            id: "stale-job".into(),
            server_id: "missing".into(),
            direction: TransferDirection::Upload,
            local_path: "absent".into(),
            remote_path: "/absent".into(),
            total_bytes: 0,
            transferred_bytes: 0,
            status: TransferStatus::Queued,
        };
        tx.send(Request {
            epoch: 2,
            serial: 9,
            action: Action::transfer(job.clone(), Arc::new(AtomicBool::new(false))),
        })
        .map_err(|_| anyhow::anyhow!("worker stopped"))?;
        assert!(matches!(receive(&rx)?.update, Update::Disconnected));
        let response = receive(&rx)?;
        assert_eq!(response.serial, 9);
        assert!(matches!(response.update, Update::Finished(id, Err(_)) if id == "stale-job"));
        tx.send(Request {
            epoch: 3,
            serial: 10,
            action: Action::transfer(job, Arc::new(AtomicBool::new(false))),
        })
        .map_err(|_| anyhow::anyhow!("worker stopped"))?;
        assert!(matches!(receive(&rx)?.update, Update::Finished(_, Err(_))));
        Ok(())
    }

    #[test]
    fn local_browsing_stays_available_during_ssh_handshake() -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            if let Ok((socket, _)) = listener.accept() {
                let _ = accepted_tx.send(());
                let _ = release_rx.recv_timeout(Duration::from_secs(5));
                drop(socket);
            }
        });
        let (tx, rx) = start();
        let profile = ServerProfile {
            host: address.ip().to_string(),
            port: address.port(),
            ..Default::default()
        };
        tx.send(Request {
            epoch: 1,
            serial: 1,
            action: Action::Probe(profile, None),
        })
        .map_err(|_| anyhow::anyhow!("worker stopped"))?;
        accepted_rx.recv_timeout(Duration::from_secs(2))?;
        let began = Instant::now();
        tx.send(Request {
            epoch: 1,
            serial: 2,
            action: Action::Local(std::env::current_dir()?),
        })
        .map_err(|_| anyhow::anyhow!("worker stopped"))?;
        let response = receive(&rx)?;
        let _ = release_tx.send(());
        server
            .join()
            .map_err(|_| anyhow::anyhow!("test server panicked"))?;
        assert!(matches!(response.update, Update::Local(_, _)));
        assert_eq!(response.serial, 2);
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "local browsing was blocked by network work"
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires loopback SSH fixture; never runs actual apt"]
    fn package_plans_are_session_bound_single_use_and_stream_results() -> Result<()> {
        let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
        anyhow::ensure!(address.ip().is_loopback(), "fixture must be loopback");
        let profile = ServerProfile {
            id: "package-worker".into(),
            host: address.ip().to_string(),
            port: address.port(),
            username: "arcnix".into(),
            ..Default::default()
        };
        let (tx, rx) = start();
        let send = |serial, action| {
            tx.send(Request {
                epoch: 100,
                serial,
                action,
            })
            .map_err(|_| anyhow::anyhow!("worker stopped"))
        };
        send(1, Action::Probe(profile, None))?;
        let fingerprint = match receive(&rx)?.update {
            Update::Fingerprint(f) => f,
            _ => anyhow::bail!("expected fingerprint"),
        };
        send(
            2,
            Action::Authenticate {
                secret: "arcnix-test".into(),
                fingerprint,
            },
        )?;
        assert!(matches!(receive(&rx)?.update, Update::Connected(true)));
        send(3, Action::PackageDetect)?;
        assert!(matches!(
            receive(&rx)?.update,
            Update::PackageEnvironment(Ok(_))
        ));
        send(
            4,
            Action::PackagePreview(PackageAction::Install(vec!["nginx".into()])),
        )?;
        assert!(matches!(
            receive(&rx)?.update,
            Update::PackagePreview(Ok(_))
        ));
        send(
            5,
            Action::PackageApply {
                plan_id: 99,
                secret: String::new(),
            },
        )?;
        assert!(matches!(
            receive(&rx)?.update,
            Update::PackageFinished(Err(_))
        ));
        send(
            6,
            Action::PackagePreview(PackageAction::Install(vec!["nginx".into()])),
        )?;
        assert!(matches!(
            receive(&rx)?.update,
            Update::PackagePreview(Ok(_))
        ));
        send(
            7,
            Action::PackageApply {
                plan_id: 6,
                secret: "fixture-transient-secret".into(),
            },
        )?;
        assert!(matches!(receive(&rx)?.update, Update::PackageStarted));
        let mut text = String::new();
        let mut chunks = 0;
        loop {
            match receive(&rx)?.update {
                Update::PackageOutput(chunk) => {
                    chunks += 1;
                    text.push_str(&chunk);
                }
                Update::PackageFinished(Ok(output)) => {
                    assert_eq!(output.exit_code, 0);
                    assert!(output.stdout.is_empty() && output.stderr.is_empty());
                    break;
                }
                Update::PackageFinished(Err(e)) => anyhow::bail!("package fixture failed: {e}"),
                _ => anyhow::bail!("unexpected package update"),
            }
        }
        assert!(chunks >= 3);
        assert!(text.contains("Package operation completed"));
        assert!(!text.contains("fixture-transient-secret"));
        send(
            8,
            Action::PackageApply {
                plan_id: 6,
                secret: String::new(),
            },
        )?;
        assert!(
            matches!(receive(&rx)?.update, Update::PackageFinished(Err(_))),
            "plan must not replay"
        );
        send(9, Action::PackagePreview(PackageAction::RefreshIndex))?;
        assert!(matches!(
            receive(&rx)?.update,
            Update::PackagePreview(Ok(_))
        ));
        tx.send(Request {
            epoch: 101,
            serial: 10,
            action: Action::Disconnect,
        })
        .map_err(|_| anyhow::anyhow!("worker stopped"))?;
        assert!(matches!(receive(&rx)?.update, Update::Disconnected));
        tx.send(Request {
            epoch: 101,
            serial: 11,
            action: Action::PackageApply {
                plan_id: 9,
                secret: String::new(),
            },
        })
        .map_err(|_| anyhow::anyhow!("worker stopped"))?;
        assert!(matches!(
            receive(&rx)?.update,
            Update::PackageFinished(Err(_))
        ));
        Ok(())
    }
}
