//! Independent SSH transport keeps interactive input responsive during SFTP.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use arcnix_core::ServerProfile;
use arcnix_core::remote::PendingConnection;
use arcnix_core::terminal::Pty;
use zeroize::Zeroizing;

use crate::worker::{Response, Update};

pub const OUTPUT_WINDOW: usize = 256 * 1024;
pub enum Action {
    Open { id: u64, columns: u32, rows: u32 },
    Input { id: u64, bytes: Vec<u8> },
    Resize { id: u64, columns: u32, rows: u32 },
    Consumed { id: u64, bytes: usize },
    Close { id: u64 },
    Detach,
}
pub enum Command {
    Attach {
        epoch: u64,
        profile: ServerProfile,
        fingerprint: String,
        secret: Zeroizing<String>,
    },
    Request {
        epoch: u64,
        serial: u64,
        action: Action,
    },
    Reset,
}
pub enum Event {
    Started { id: u64 },
    Output { id: u64, bytes: Vec<u8> },
    Exited { id: u64, code: Option<i32> },
    Failed { id: u64, message: String },
}
struct Credentials {
    epoch: u64,
    profile: ServerProfile,
    fingerprint: String,
    secret: Zeroizing<String>,
}
pub fn start(
    events: async_channel::Sender<Response>,
    generation: Arc<AtomicU64>,
) -> mpsc::Sender<Command> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut credentials: Option<Credentials> = None;
        let mut terminal: Option<Pty> = None;
        let mut terminal_id = 0;
        let mut epoch = 0;
        let mut unconsumed = 0usize;
        let send = |epoch, serial, event| {
            events
                .send_blocking(Response {
                    epoch,
                    serial,
                    update: Update::Terminal(event),
                })
                .is_ok()
        };
        loop {
            if epoch != generation.load(Ordering::Acquire) {
                terminal = None;
                credentials = None;
            }
            match rx.recv_timeout(Duration::from_millis(12)) {
                Ok(Command::Reset) => {
                    terminal = None;
                    credentials = None;
                    unconsumed = 0;
                }
                Ok(Command::Attach {
                    epoch: next,
                    profile,
                    fingerprint,
                    secret,
                }) => {
                    if next != generation.load(Ordering::Acquire) {
                        continue;
                    }
                    terminal = None;
                    epoch = next;
                    credentials = Some(Credentials {
                        epoch,
                        profile,
                        fingerprint,
                        secret,
                    });
                    unconsumed = 0;
                }
                Ok(Command::Request {
                    epoch: request_epoch,
                    serial,
                    action,
                }) => {
                    if request_epoch != generation.load(Ordering::Acquire) || request_epoch != epoch
                    {
                        continue;
                    }
                    let result: anyhow::Result<()> = (|| {
                        match action {
                            Action::Open { id, columns, rows } => {
                                terminal = None;
                                terminal_id = id;
                                unconsumed = 0;
                                let credentials = credentials.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("Reconnect SSH before opening a terminal")
                                })?;
                                anyhow::ensure!(
                                    credentials.epoch == epoch,
                                    "Terminal connection has changed"
                                );
                                let pending = PendingConnection::open_for_known_host(
                                    &credentials.profile,
                                    &credentials.fingerprint,
                                )?;
                                anyhow::ensure!(
                                    epoch == generation.load(Ordering::Acquire),
                                    "Terminal connection was cancelled"
                                );
                                let remote = pending
                                    .authenticate(&credentials.secret, &credentials.fingerprint)?;
                                anyhow::ensure!(
                                    epoch == generation.load(Ordering::Acquire),
                                    "Terminal connection was cancelled"
                                );
                                terminal = Some(remote.into_terminal(columns, rows)?);
                                if !send(epoch, serial, Event::Started { id }) {
                                    return Err(anyhow::anyhow!("Terminal receiver stopped"));
                                }
                            }
                            Action::Input { id, bytes } if id == terminal_id => {
                                if let Some(terminal) = terminal.as_mut() {
                                    terminal.input(bytes)?;
                                }
                            }
                            Action::Resize { id, columns, rows } if id == terminal_id => {
                                if let Some(terminal) = terminal.as_mut() {
                                    terminal.resize(columns, rows)?;
                                }
                            }
                            Action::Consumed { id, bytes } if id == terminal_id => {
                                unconsumed = unconsumed.saturating_sub(bytes)
                            }
                            Action::Close { id } if id == terminal_id => {
                                terminal = None;
                                send(epoch, serial, Event::Exited { id, code: None });
                            }
                            Action::Detach => {
                                terminal = None;
                                credentials = None;
                                unconsumed = 0;
                            }
                            _ => {}
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        terminal = None;
                        if !send(
                            epoch,
                            serial,
                            Event::Failed {
                                id: terminal_id,
                                message: format!("{error:#}"),
                            },
                        ) {
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(pty) = terminal.as_mut() {
                match pty.poll(OUTPUT_WINDOW.saturating_sub(unconsumed).min(16_384)) {
                    Ok(output) => {
                        if !output.bytes.is_empty() {
                            unconsumed += output.bytes.len();
                            if !send(
                                epoch,
                                0,
                                Event::Output {
                                    id: terminal_id,
                                    bytes: output.bytes,
                                },
                            ) {
                                break;
                            }
                        }
                        if let Some(code) = output.exit_code {
                            terminal = None;
                            if !send(
                                epoch,
                                0,
                                Event::Exited {
                                    id: terminal_id,
                                    code: Some(code),
                                },
                            ) {
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        terminal = None;
                        if !send(
                            epoch,
                            0,
                            Event::Failed {
                                id: terminal_id,
                                message: format!("{error:#}"),
                            },
                        ) {
                            break;
                        }
                    }
                }
            }
        }
    });
    tx
}
