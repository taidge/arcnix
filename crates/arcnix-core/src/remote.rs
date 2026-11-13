//! Blocking SSH/SFTP primitives, intended to run on the application worker.
//! A pending handshake never sends authentication credentials until trust is checked.
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use sha2::{Digest, Sha256};
use ssh2::{ErrorCode, MethodType, OpenFlags, OpenType, RenameFlags, Session, Sftp};

use crate::transfer::{ConflictAction, TransferConflict, TransferSummary};
use crate::{
    AuthMethod, CommandOutput, FileEntry, ServerInfo, ServerProfile, TransferDirection, new_id,
};

const TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_LIMIT: usize = 1024 * 1024;
const TREE_DEPTH_LIMIT: usize = 64;
const TREE_ENTRY_LIMIT: usize = 100_000;
const STREAM_TAIL_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandStream {
    Stdout,
    Stderr,
}

struct BlockingRestore<'a> {
    session: &'a Session,
    blocking: bool,
}
impl Drop for BlockingRestore<'_> {
    fn drop(&mut self) {
        self.session.set_blocking(self.blocking);
    }
}

#[derive(Default)]
struct OutputTail {
    bytes: std::collections::VecDeque<u8>,
    truncated: bool,
}
impl OutputTail {
    fn push(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes);
        if self.bytes.len() > STREAM_TAIL_LIMIT {
            self.bytes.drain(..self.bytes.len() - STREAM_TAIL_LIMIT);
            self.truncated = true;
        }
    }
    fn text(self) -> String {
        let bytes: Vec<_> = self.bytes.into_iter().collect();
        let text = String::from_utf8_lossy(&bytes);
        if self.truncated {
            format!("[Earlier output omitted; showing last 256 KiB]\n{text}")
        } else {
            text.into_owned()
        }
    }
}

fn stream_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(
            io::Error::new(io::ErrorKind::TimedOut, "Remote command deadline exceeded").into(),
        );
    }
    Ok(())
}
fn stream_retry<T>(
    deadline: Instant,
    mut operation: impl FnMut() -> std::result::Result<T, ssh2::Error>,
) -> Result<T> {
    loop {
        stream_deadline(deadline)?;
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if error.code() == ErrorCode::Session(-37) => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(error) => return Err(error.into()),
        }
    }
}

struct TreeEntry {
    relative: String,
    directory: bool,
    size: u64,
}

#[derive(Default)]
struct TreePlan {
    entries: Vec<TreeEntry>,
    total: u64,
}

impl TreePlan {
    fn push(&mut self, relative: String, directory: bool, size: u64) -> Result<()> {
        ensure!(
            self.entries.len() < TREE_ENTRY_LIMIT,
            "目录超过 {TREE_ENTRY_LIMIT} 个项目"
        );
        self.total = self.total.checked_add(size).context("目录文件总大小溢出")?;
        self.entries.push(TreeEntry {
            relative,
            directory,
            size,
        });
        Ok(())
    }
}

#[derive(Debug)]
pub struct TransferCancelled;
impl std::fmt::Display for TransferCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Transfer cancelled")
    }
}
impl std::error::Error for TransferCancelled {}

pub struct PendingConnection {
    session: Session,
    profile: ServerProfile,
    fingerprint: String,
}
pub struct RemoteSession {
    session: Session,
    sftp: Option<Sftp>,
}

impl PendingConnection {
    pub fn open(profile: &ServerProfile) -> Result<Self> {
        Self::open_with_host_key_preference(profile, None, Instant::now() + TIMEOUT)
    }

    /// Re-negotiate the previously trusted host key when crypto backends have
    /// different defaults. Never trusts a different key or sends credentials.
    pub fn open_for_known_host(profile: &ServerProfile, expected: &str) -> Result<Self> {
        let deadline = Instant::now() + TIMEOUT;
        let initial = Self::open_with_host_key_preference(profile, None, deadline)?;
        if initial.fingerprint == expected {
            return Ok(initial);
        }
        let observed = initial.fingerprint.clone();
        let method = initial
            .session
            .methods(MethodType::HostKey)
            .map(str::to_owned);
        let supported = initial.session.supported_algs(MethodType::HostKey)?;
        drop(initial);
        // RSA SHA-2 signatures use the same RSA public key/fingerprint as the
        // old backend. Do not re-enable SHA-1/DSA to recover a historical pin.
        for preference in [
            "rsa-sha2-512,rsa-sha2-256",
            "ecdsa-sha2-nistp256",
            "ecdsa-sha2-nistp384",
            "ecdsa-sha2-nistp521",
            "ssh-ed25519",
        ] {
            if Instant::now() >= deadline {
                break;
            }
            if preference
                .split(',')
                .any(|item| Some(item) == method.as_deref())
                || !preference.split(',').any(|item| supported.contains(&item))
            {
                continue;
            }
            if let Ok(candidate) =
                Self::open_with_host_key_preference(profile, Some(preference), deadline)
                && candidate.fingerprint == expected
            {
                return Ok(candidate);
            }
        }
        bail!(
            "SSH host key changed. Saved: {expected}; current: {observed}. Authentication was not attempted. Verify the server identity through a trusted channel."
        )
    }

    fn open_with_host_key_preference(
        profile: &ServerProfile,
        preference: Option<&str>,
        deadline: Instant,
    ) -> Result<Self> {
        ensure!(
            !profile.host.trim().is_empty() && profile.port != 0,
            "Host and nonzero port are required"
        );
        let host = profile
            .host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']');
        let addresses = (host, profile.port)
            .to_socket_addrs()
            .context("Cannot resolve SSH host")?;
        let mut last_error = None;
        let mut socket = None;
        for addr in addresses {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .context("SSH connection timed out")?;
            match TcpStream::connect_timeout(&addr, remaining) {
                Ok(stream) => {
                    socket = Some(stream);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let socket = socket.ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot connect to SSH server: {}",
                last_error
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "no resolved addresses".into())
            )
        })?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("SSH connection timed out")?;
        socket.set_read_timeout(Some(remaining))?;
        socket.set_write_timeout(Some(remaining))?;
        let mut session = Session::new()?;
        session.set_timeout(remaining.as_millis().clamp(1, u32::MAX as u128) as u32);
        if let Some(preference) = preference {
            session.method_pref(MethodType::HostKey, preference)?;
        }
        session.set_tcp_stream(socket.try_clone()?);
        session.handshake().context("SSH handshake failed")?;
        // The shared deadline bounds key discovery, not later authentication or
        // file operations on the successfully matched connection.
        socket.set_read_timeout(Some(TIMEOUT))?;
        socket.set_write_timeout(Some(TIMEOUT))?;
        session.set_timeout(TIMEOUT.as_millis() as u32);
        let (key, _) = session.host_key().context("Server supplied no host key")?;
        let fingerprint = format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(key)));
        Ok(Self {
            session,
            profile: profile.clone(),
            fingerprint,
        })
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn authenticate(self, secret: &str, expected_fingerprint: &str) -> Result<RemoteSession> {
        ensure!(
            self.fingerprint == expected_fingerprint,
            "SSH host key mismatch; authentication was not attempted"
        );
        ensure!(
            !self.profile.username.trim().is_empty(),
            "SSH username is required"
        );
        match self.profile.auth {
            AuthMethod::Password => self
                .session
                .userauth_password(&self.profile.username, secret)
                .context("SSH password authentication failed")?,
            AuthMethod::PrivateKey => {
                let key = self
                    .profile
                    .private_key
                    .as_ref()
                    .context("Private key path is required")?;
                ensure!(
                    fs::metadata(key)
                        .context("Cannot access SSH private key file")?
                        .is_file(),
                    "SSH private key path must be a file"
                );
                self.session
                    .userauth_pubkey_file(
                        &self.profile.username,
                        None,
                        key,
                        if secret.is_empty() {
                            None
                        } else {
                            Some(secret)
                        },
                    )
                    .map_err(|error| {
                        let reason = match error.code() {
                            ErrorCode::Session(-18 | -19) => {
                                "Server rejected the SSH key; check the username and authorized_keys"
                            }
                            ErrorCode::Session(-16 | -48) => {
                                "Cannot load SSH private key; check the file format and passphrase"
                            }
                            _ => {
                                "SSH private key authentication failed; check the key file, passphrase and server authentication policy"
                            }
                        };
                        anyhow::Error::new(error).context(reason)
                    })?;
            }
        }
        ensure!(self.session.authenticated(), "SSH authentication failed");
        let sftp = match self.session.sftp() {
            Ok(sftp) => Some(sftp),
            Err(error) => {
                let error = anyhow::Error::new(error).context("Cannot start SFTP subsystem");
                if crate::transport_error::is_connection_error(&error) {
                    return Err(error);
                }
                None
            }
        };
        Ok(RemoteSession {
            session: self.session,
            sftp,
        })
    }
}

impl RemoteSession {
    /// Use only a dedicated SSH transport: PTY polling changes its blocking mode.
    pub fn into_terminal(self, columns: u32, rows: u32) -> Result<crate::terminal::Pty> {
        let Self { session, sftp } = self;
        drop(sftp);
        crate::terminal::Pty::open(session, columns, rows)
    }
    pub fn sftp_available(&self) -> bool {
        self.sftp.is_some()
    }
    fn sftp(&self) -> Result<&Sftp> {
        self.sftp
            .as_ref()
            .context("服务器未启用 SFTP，文件管理不可用；SSH 系统操作仍可使用")
    }
    /// Transfers a regular file or an entire directory into an absent destination.
    /// Directory cancellation preserves completed files; retry with a new target.
    pub fn upload_path(
        &self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        check_cancel(cancel)?;
        validate_path(remote)?;
        let metadata = fs::symlink_metadata(local).context("Cannot inspect upload source")?;
        ensure!(!metadata.is_symlink(), "不传输符号链接");
        if !metadata.is_dir() {
            return self.upload(local, remote, cancel, progress);
        }
        let mut plan = TreePlan::default();
        scan_local(local, "", 0, cancel, &mut plan)?;
        self.ensure_absent(remote)?;
        for item in &plan.entries {
            validate_path(&tree_remote(remote, &item.relative))?;
        }
        progress(0, plan.total);
        check_cancel(cancel)?;
        self.mkdir(remote)?;
        let result: Result<()> = (|| {
            let mut completed = 0;
            for item in &plan.entries {
                check_cancel(cancel)?;
                let source = tree_local(local, &item.relative);
                let destination = tree_remote(remote, &item.relative);
                if item.directory {
                    let metadata = fs::symlink_metadata(&source)?;
                    ensure!(
                        metadata.is_dir() && !metadata.is_symlink(),
                        "Source directory changed during transfer"
                    );
                    self.mkdir(&destination)?;
                } else {
                    let metadata = fs::symlink_metadata(&source)?;
                    ensure!(
                        metadata.is_file() && !metadata.is_symlink() && metadata.len() == item.size,
                        "Source file changed after directory scan"
                    );
                    self.upload(&source, &destination, cancel, |done, _| {
                        progress(completed + done.min(item.size), plan.total)
                    })?;
                    completed += item.size;
                }
            }
            check_cancel(cancel)?;
            progress(plan.total, plan.total);
            Ok(())
        })();
        result.with_context(|| {
            format!("目录传输未完成；已完成文件保留在 {remote}，重试前请更换目标目录或移走已有目录")
        })
    }

    /// Downloads a directory using a prevalidated manifest; never merges trees.
    pub fn download_path(
        &self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        check_cancel(cancel)?;
        validate_path(remote)?;
        let metadata = self
            .sftp()?
            .lstat(Path::new(remote))
            .context("Cannot inspect download source")?;
        ensure!(!metadata.file_type().is_symlink(), "不传输符号链接");
        if !metadata.is_dir() {
            return self.download(remote, local, cancel, progress);
        }
        let mut plan = TreePlan::default();
        self.scan_remote(remote, "", 0, cancel, &mut plan)?;
        ensure!(
            fs::symlink_metadata(local).is_err_and(|e| e.kind() == io::ErrorKind::NotFound),
            "目标目录已存在或无法读取；目录传输不会合并或覆盖"
        );
        progress(0, plan.total);
        check_cancel(cancel)?;
        fs::create_dir(local).context("Cannot create download directory")?;
        let result: Result<()> = (|| {
            let mut completed = 0;
            for item in &plan.entries {
                check_cancel(cancel)?;
                let source = tree_remote(remote, &item.relative);
                let destination = tree_local(local, &item.relative);
                if item.directory {
                    let metadata = self.sftp()?.lstat(Path::new(&source))?;
                    ensure!(
                        metadata.is_dir() && !metadata.file_type().is_symlink(),
                        "Source directory changed during transfer"
                    );
                    fs::create_dir(&destination).context("Cannot create download subdirectory")?;
                } else {
                    let metadata = self.sftp()?.lstat(Path::new(&source))?;
                    ensure!(
                        metadata.is_file()
                            && !metadata.file_type().is_symlink()
                            && metadata.size == Some(item.size),
                        "Source file changed after directory scan"
                    );
                    self.download(&source, &destination, cancel, |done, _| {
                        progress(completed + done.min(item.size), plan.total)
                    })?;
                    completed += item.size;
                }
            }
            check_cancel(cancel)?;
            progress(plan.total, plan.total);
            Ok(())
        })();
        result.with_context(|| {
            format!(
                "目录传输未完成；已完成文件保留在 {}，重试前请更换目标目录或移走已有目录",
                local.display()
            )
        })
    }

    pub fn upload_path_with_conflicts(
        &self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
        on_conflict: impl FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary> {
        self.upload_path_with_conflicts_stats(
            local,
            remote,
            cancel,
            |done, total, _| progress(done, total),
            on_conflict,
        )
    }

    pub fn upload_path_with_conflicts_stats(
        &self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64, u64),
        mut on_conflict: impl FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary> {
        check_cancel(cancel)?;
        validate_path(remote)?;
        let metadata = fs::symlink_metadata(local).context("Cannot inspect upload source")?;
        ensure!(!metadata.is_symlink(), "不传输符号链接");
        if !metadata.is_dir() {
            return self.upload_with_conflicts_stats(local, remote, cancel, progress, on_conflict);
        }
        let mut plan = TreePlan::default();
        scan_local(local, "", 0, cancel, &mut plan)?;
        for item in &plan.entries {
            validate_path(&tree_remote(remote, &item.relative))?;
        }
        check_cancel(cancel)?;
        self.ensure_remote_directory(remote)?;
        progress(0, plan.total, 0);
        check_cancel(cancel)?;

        let result: Result<TransferSummary> = (|| {
            let mut summary = TransferSummary::default();
            let mut completed = 0;
            let mut actual = 0;
            for item in &plan.entries {
                check_cancel(cancel)?;
                let source = tree_local(local, &item.relative);
                let destination = tree_remote(remote, &item.relative);
                if item.directory {
                    let metadata = fs::symlink_metadata(&source)?;
                    ensure!(
                        metadata.is_dir() && !metadata.is_symlink(),
                        "Source directory changed during transfer"
                    );
                    self.ensure_remote_directory(&destination)?;
                } else {
                    let metadata = fs::symlink_metadata(&source)?;
                    ensure!(
                        metadata.is_file() && !metadata.is_symlink() && metadata.len() == item.size,
                        "Source file changed after directory scan"
                    );
                    let mut file_actual = 0;
                    summary.add(self.upload_with_conflicts_stats(
                        &source,
                        &destination,
                        cancel,
                        |done, _, copied| {
                            file_actual = copied;
                            progress(completed + done.min(item.size), plan.total, actual + copied);
                        },
                        &mut on_conflict,
                    )?);
                    completed += item.size;
                    actual += file_actual;
                }
            }
            check_cancel(cancel)?;
            progress(plan.total, plan.total, actual);
            Ok(summary)
        })();
        result.with_context(|| {
            format!("目录传输未完成；已完成文件保留在 {remote}；重试时可选择覆盖或跳过已有文件")
        })
    }

    /// Merges regular directories and asks before replacing individual files.
    pub fn download_path_with_conflicts(
        &self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
        on_conflict: impl FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary> {
        self.download_path_with_conflicts_stats(
            remote,
            local,
            cancel,
            |done, total, _| progress(done, total),
            on_conflict,
        )
    }

    pub fn download_path_with_conflicts_stats(
        &self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64, u64),
        mut on_conflict: impl FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary> {
        check_cancel(cancel)?;
        validate_path(remote)?;
        let metadata = self
            .sftp()?
            .lstat(Path::new(remote))
            .context("Cannot inspect download source")?;
        ensure!(!metadata.file_type().is_symlink(), "不传输符号链接");
        if !metadata.is_dir() {
            return self.download_with_conflicts_stats(
                remote,
                local,
                cancel,
                progress,
                on_conflict,
            );
        }
        let mut plan = TreePlan::default();
        self.scan_remote(remote, "", 0, cancel, &mut plan)?;
        ensure_local_directory(local)?;
        progress(0, plan.total, 0);
        check_cancel(cancel)?;

        let result: Result<TransferSummary> = (|| {
            let mut summary = TransferSummary::default();
            let mut completed = 0;
            let mut actual = 0;
            for item in &plan.entries {
                check_cancel(cancel)?;
                let source = tree_remote(remote, &item.relative);
                let destination = tree_local(local, &item.relative);
                if item.directory {
                    let metadata = self.sftp()?.lstat(Path::new(&source))?;
                    ensure!(
                        metadata.is_dir() && !metadata.file_type().is_symlink(),
                        "Source directory changed during transfer"
                    );
                    ensure_local_directory(&destination)?;
                } else {
                    let metadata = self.sftp()?.lstat(Path::new(&source))?;
                    ensure!(
                        metadata.is_file()
                            && !metadata.file_type().is_symlink()
                            && metadata.size == Some(item.size),
                        "Source file changed after directory scan"
                    );
                    let mut file_actual = 0;
                    summary.add(self.download_with_conflicts_stats(
                        &source,
                        &destination,
                        cancel,
                        |done, _, copied| {
                            file_actual = copied;
                            progress(completed + done.min(item.size), plan.total, actual + copied);
                        },
                        &mut on_conflict,
                    )?);
                    completed += item.size;
                    actual += file_actual;
                }
            }
            check_cancel(cancel)?;
            progress(plan.total, plan.total, actual);
            Ok(summary)
        })();
        result.with_context(|| {
            format!(
                "目录传输未完成；已完成文件保留在 {}；重试时可选择覆盖或跳过已有文件",
                local.display()
            )
        })
    }

    fn scan_remote(
        &self,
        root: &str,
        relative: &str,
        depth: usize,
        cancel: &AtomicBool,
        plan: &mut TreePlan,
    ) -> Result<()> {
        check_cancel(cancel)?;
        ensure!(
            depth <= TREE_DEPTH_LIMIT,
            "目录深度超过 {TREE_DEPTH_LIMIT} 层"
        );
        let (_, entries) = self.list_dir(&tree_remote(root, relative))?;
        let mut names = std::collections::HashSet::new();
        for entry in entries {
            check_cancel(cancel)?;
            validate_download_name(&entry.name)?;
            #[cfg(windows)]
            let name_key = entry.name.to_lowercase();
            #[cfg(not(windows))]
            let name_key = entry.name.clone();
            ensure!(names.insert(name_key), "目标文件名发生大小写冲突");
            ensure!(!entry.is_symlink, "目录包含符号链接：{}", entry.path);
            let relative = if relative.is_empty() {
                entry.name
            } else {
                format!("{relative}/{}", entry.name)
            };
            let metadata = self.sftp()?.lstat(Path::new(&entry.path))?;
            ensure!(
                metadata.is_dir() || metadata.is_file(),
                "目录包含非普通文件：{}",
                entry.path
            );
            plan.push(
                relative.clone(),
                metadata.is_dir(),
                if metadata.is_dir() {
                    0
                } else {
                    metadata.size.context("Remote file size unavailable")?
                },
            )?;
            if metadata.is_dir() {
                self.scan_remote(root, &relative, depth + 1, cancel, plan)?;
            }
        }
        Ok(())
    }

    pub fn list_dir(&self, path: &str) -> Result<(String, Vec<FileEntry>)> {
        validate_path(path)?;
        let canonical = self
            .sftp()?
            .realpath(Path::new(path))
            .context("Cannot resolve remote directory")?;
        let canonical = path_string(&canonical)?;
        validate_path(&canonical)?;
        let mut entries = Vec::new();
        let mut directory = self
            .sftp()?
            .opendir(Path::new(&canonical))
            .context("Cannot open remote directory")?;
        loop {
            let (filename, stat) = match directory.readdir() {
                Ok(entry) => entry,
                Err(error) if error.code() == ErrorCode::Session(-16) => break,
                Err(error) => return Err(error).context("Cannot list remote directory"),
            };
            let name = path_string(&filename)?;
            if name == "." || name == ".." {
                continue;
            }
            ensure!(
                !name.is_empty() && !name.contains('/'),
                "Invalid remote filename"
            );
            let path = format!("{}/{name}", canonical.trim_end_matches('/'));
            validate_path(&path)?;
            let is_symlink = stat.file_type().is_symlink();
            let is_dir = stat.is_dir()
                || (is_symlink
                    && self
                        .sftp()?
                        .stat(Path::new(&path))
                        .is_ok_and(|s| s.is_dir()));
            ensure!(
                entries.len() < TREE_ENTRY_LIMIT,
                "目录超过 {TREE_ENTRY_LIMIT} 个项目"
            );
            entries.push(FileEntry {
                name,
                path,
                is_dir,
                is_symlink,
                size: stat.size.unwrap_or(0),
                modified: stat.mtime,
                permissions: stat.perm,
            });
        }
        crate::local::sort_entries(&mut entries);
        Ok((canonical, entries))
    }
    pub fn mkdir(&self, path: &str) -> Result<()> {
        validate_path(path)?;
        self.sftp()?
            .mkdir(Path::new(path), 0o755)
            .context("Cannot create remote directory")
    }
    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        validate_path(from)?;
        validate_path(to)?;
        self.ensure_absent(to)?;
        self.sftp()?
            .rename(Path::new(from), Path::new(to), Some(RenameFlags::empty()))
            .context("Cannot rename remote entry without overwrite")
    }
    fn ensure_absent(&self, path: &str) -> Result<()> {
        match self.sftp()?.lstat(Path::new(path)) {
            Ok(_) => bail!("Destination already exists: {path}"),
            Err(e) if e.code() == ErrorCode::SFTP(2) => Ok(()),
            Err(e) => Err(e).context("Cannot inspect remote destination"),
        }
    }
    pub fn upload(
        &self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        validate_path(remote)?;
        check_cancel(cancel)?;
        let metadata = fs::symlink_metadata(local).context("Cannot inspect upload source")?;
        ensure!(
            metadata.is_file() && !metadata.is_symlink(),
            "Upload source must be a regular file, not a directory or symlink"
        );
        self.ensure_absent(remote)?;
        let mut input = fs::File::open(local).context("Cannot open upload source")?;
        let partial = remote_partial(remote)?;
        let mut output = self
            .sftp()?
            .open_mode(
                Path::new(&partial),
                OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUSIVE,
                0o600,
                OpenType::File,
            )
            .context("Cannot create remote partial file")?;
        let result: Result<()> = (|| {
            copy_with_progress(
                &mut input,
                &mut output,
                metadata.len(),
                cancel,
                &mut progress,
            )?;
            output.flush()?;
            output.close()?;
            check_cancel(cancel)?;
            self.rename(&partial, remote)?;
            Ok(())
        })();
        if result.is_err() {
            drop(output);
            let _ = self.sftp()?.unlink(Path::new(&partial));
        }
        result
    }
    pub fn download(
        &self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        validate_path(remote)?;
        check_cancel(cancel)?;
        let metadata = self
            .sftp()?
            .lstat(Path::new(remote))
            .context("Cannot inspect download source")?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Download source must be a regular file, not a directory or symlink"
        );
        ensure!(
            fs::symlink_metadata(local).is_err_and(|e| e.kind() == io::ErrorKind::NotFound),
            "Local destination exists or cannot be inspected"
        );
        let mut input = self
            .sftp()?
            .open(Path::new(remote))
            .context("Cannot open remote file")?;
        let parent = local
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let partial = parent.join(format!(".arcnix-{}.part", new_id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options
            .open(&partial)
            .context("Cannot create local partial file")?;
        let result: Result<()> = (|| {
            copy_with_progress(
                &mut input,
                &mut output,
                metadata.size.unwrap_or(0),
                cancel,
                &mut progress,
            )?;
            output.flush()?;
            output.sync_all()?;
            check_cancel(cancel)?;
            // Hard-link publication fails atomically if another process created
            // the destination. Partial and final are on the same filesystem.
            fs::hard_link(&partial, local).context(
                "Cannot publish download without overwrite (filesystem must support hard links)",
            )?;
            Ok(())
        })();
        drop(output);
        let cleanup = fs::remove_file(&partial);
        result?;
        cleanup.context("Download succeeded but partial file cleanup failed")
    }
    fn replace_remote(&self, partial: &str, destination: &str) -> Result<()> {
        // Replacement publishes a new inode. Retain the destination's access
        // permissions rather than publishing the temporary file's private mode.
        // Fail before publishing if chmod is unavailable or denied.
        let target = self.sftp()?.lstat(Path::new(destination))?;
        ensure!(
            target.is_file() && !target.file_type().is_symlink(),
            "目标不是普通文件，不能覆盖：{destination}"
        );
        let permissions = target.perm.context("目标权限不可用，未覆盖原文件")? & 0o7777;
        self.sftp()?
            .setstat(
                Path::new(partial),
                ssh2::FileStat {
                    size: None,
                    uid: None,
                    gid: None,
                    perm: Some(permissions),
                    atime: None,
                    mtime: None,
                },
            )
            .context("无法保留目标权限，未覆盖原文件")?;
        // SFTP v3 ignores rename flags and OpenSSH's ordinary rename refuses
        // replacement. The Rust ssh2 wrapper lacks the posix-rename binding.
        // Try the protocol operation first, then use one guarded same-directory
        // Linux mv; never unlink the existing file or copy directly into it.
        match self.sftp()?.rename(
            Path::new(partial),
            Path::new(destination),
            Some(RenameFlags::OVERWRITE | RenameFlags::ATOMIC),
        ) {
            Ok(()) => return Ok(()),
            Err(error) if matches!(error.code(), ErrorCode::SFTP(4 | 8 | 11)) => {}
            Err(error) => return Err(error).context("无法发布覆盖文件"),
        }
        let from = shell_path(partial);
        let to = shell_path(destination);
        let command = format!(
            "# arcnix:transfer:replace\n[ \"$(uname -s)\" = Linux ] && [ -f {from} ] && [ ! -L {from} ] && [ -f {to} ] && [ ! -L {to} ] && mv -fT -- {from} {to}"
        );
        let output = self
            .exec(&command)
            .context("无法确认远程覆盖结果，请刷新目标目录")?;
        ensure!(
            output.exit_code == 0,
            "服务器未能安全覆盖目标（需支持 mv -fT）：{}",
            output.stderr.trim()
        );
        Ok(())
    }
    fn remote_target(&self, path: &str) -> Result<Option<(u64, Option<u64>)>> {
        match self.sftp()?.lstat(Path::new(path)) {
            Ok(stat) => {
                ensure!(
                    stat.is_file() && !stat.file_type().is_symlink(),
                    "目标不是普通文件，不能覆盖：{path}"
                );
                Ok(Some((stat.size.context("目标文件大小不可用")?, stat.mtime)))
            }
            Err(error) if error.code() == ErrorCode::SFTP(2) => Ok(None),
            Err(error) => Err(error).context("无法读取目标文件信息"),
        }
    }
    fn ensure_remote_directory(&self, path: &str) -> Result<()> {
        match self.sftp()?.lstat(Path::new(path)) {
            Ok(stat) => {
                ensure!(
                    stat.is_dir() && !stat.file_type().is_symlink(),
                    "目标不是普通目录，不能合并：{path}"
                );
                Ok(())
            }
            Err(error) if error.code() == ErrorCode::SFTP(2) => self.mkdir(path),
            Err(error) => Err(error).context("无法读取目标目录信息"),
        }
    }
    pub fn upload_with_conflicts(
        &self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
        on_conflict: impl FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary> {
        self.upload_with_conflicts_stats(
            local,
            remote,
            cancel,
            |done, total, _| progress(done, total),
            on_conflict,
        )
    }

    pub fn upload_with_conflicts_stats(
        &self,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64, u64),
        mut on_conflict: impl FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary> {
        validate_path(remote)?;
        check_cancel(cancel)?;
        let metadata = fs::symlink_metadata(local).context("Cannot inspect upload source")?;
        ensure!(
            metadata.is_file() && !metadata.is_symlink(),
            "Upload source must be a regular file, not a directory or symlink"
        );
        let mut approved = self.remote_target(remote)?;
        if let Some(target) = approved
            && resolve_conflict(
                &mut on_conflict,
                TransferConflict {
                    direction: TransferDirection::Upload,
                    source_path: local.to_string_lossy().into_owned(),
                    destination_path: remote.into(),
                    source_size: metadata.len(),
                    destination_size: target.0,
                    source_modified: local_modified(&metadata),
                    destination_modified: target.1,
                },
                cancel,
            )?
        {
            progress(metadata.len(), metadata.len(), 0);
            return Ok(TransferSummary {
                transferred_files: 0,
                skipped_files: 1,
            });
        }
        let mut input = fs::File::open(local).context("Cannot open upload source")?;
        let partial = remote_partial(remote)?;
        let mut output = self
            .sftp()?
            .open_mode(
                Path::new(&partial),
                OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUSIVE,
                0o600,
                OpenType::File,
            )
            .context("Cannot create remote partial file")?;
        let result: Result<TransferSummary> = (|| {
            copy_with_progress(
                &mut input,
                &mut output,
                metadata.len(),
                cancel,
                &mut |done, total| progress(done, total, done),
            )?;
            output.flush()?;
            output.close()?;
            check_cancel(cancel)?;
            let current = self.remote_target(remote)?;
            if let Some(target) = current
                && current != approved
            {
                if resolve_conflict(
                    &mut on_conflict,
                    TransferConflict {
                        direction: TransferDirection::Upload,
                        source_path: local.to_string_lossy().into_owned(),
                        destination_path: remote.into(),
                        source_size: metadata.len(),
                        destination_size: target.0,
                        source_modified: local_modified(&metadata),
                        destination_modified: target.1,
                    },
                    cancel,
                )? {
                    return Ok(TransferSummary {
                        transferred_files: 0,
                        skipped_files: 1,
                    });
                }
                approved = current;
            }
            check_cancel(cancel)?;
            if approved.is_some() {
                self.replace_remote(&partial, remote)?;
            } else {
                self.rename(&partial, remote)?;
            }
            Ok(TransferSummary {
                transferred_files: 1,
                skipped_files: 0,
            })
        })();
        drop(output);
        // A late Skip also leaves a fully copied partial to clean up.
        let _ = self.sftp()?.unlink(Path::new(&partial));
        result
    }
    pub fn download_with_conflicts(
        &self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
        on_conflict: impl FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary> {
        self.download_with_conflicts_stats(
            remote,
            local,
            cancel,
            |done, total, _| progress(done, total),
            on_conflict,
        )
    }

    pub fn download_with_conflicts_stats(
        &self,
        remote: &str,
        local: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64, u64),
        mut on_conflict: impl FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary> {
        validate_path(remote)?;
        check_cancel(cancel)?;
        let metadata = self
            .sftp()?
            .lstat(Path::new(remote))
            .context("Cannot inspect download source")?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Download source must be a regular file, not a directory or symlink"
        );
        let mut approved = local_target(local)?;
        if let Some(target) = approved
            && resolve_conflict(
                &mut on_conflict,
                TransferConflict {
                    direction: TransferDirection::Download,
                    source_path: remote.into(),
                    destination_path: local.to_string_lossy().into_owned(),
                    source_size: metadata.size.unwrap_or(0),
                    destination_size: target.0,
                    source_modified: metadata.mtime,
                    destination_modified: target.1,
                },
                cancel,
            )?
        {
            progress(metadata.size.unwrap_or(0), metadata.size.unwrap_or(0), 0);
            return Ok(TransferSummary {
                transferred_files: 0,
                skipped_files: 1,
            });
        }
        let mut input = self
            .sftp()?
            .open(Path::new(remote))
            .context("Cannot open remote file")?;
        let parent = local
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let partial = parent.join(format!(".arcnix-{}.part", new_id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options
            .open(&partial)
            .context("Cannot create local partial file")?;
        let result: Result<TransferSummary> = (|| {
            copy_with_progress(
                &mut input,
                &mut output,
                metadata.size.unwrap_or(0),
                cancel,
                &mut |done, total| progress(done, total, done),
            )?;
            output.flush()?;
            output.sync_all()?;
            check_cancel(cancel)?;
            let current = local_target(local)?;
            if let Some(target) = current
                && current != approved
            {
                if resolve_conflict(
                    &mut on_conflict,
                    TransferConflict {
                        direction: TransferDirection::Download,
                        source_path: remote.into(),
                        destination_path: local.to_string_lossy().into_owned(),
                        source_size: metadata.size.unwrap_or(0),
                        destination_size: target.0,
                        source_modified: metadata.mtime,
                        destination_modified: target.1,
                    },
                    cancel,
                )? {
                    return Ok(TransferSummary {
                        transferred_files: 0,
                        skipped_files: 1,
                    });
                }
                approved = current;
            }
            check_cancel(cancel)?;
            if approved.is_some() {
                replace_local(&partial, local)?;
            } else {
                fs::hard_link(&partial, local)
                    .context("Cannot publish download without overwrite")?;
            }
            Ok(TransferSummary {
                transferred_files: 1,
                skipped_files: 0,
            })
        })();
        drop(output);
        let cleanup = fs::remove_file(&partial);
        let summary = result?;
        // A replace consumes the partial, while no-overwrite publication links it.
        if let Err(error) = cleanup {
            ensure!(
                error.kind() == io::ErrorKind::NotFound,
                "下载完成但临时文件清理失败：{error}"
            );
        }
        Ok(summary)
    }
    /// Streams a bounded-duration command while retaining only bounded output
    /// tails. Output beyond the retained tail is still drained and delivered to
    /// the callback; verbosity must never terminate a package installation.
    /// Input is sent on stdin and is never added to command/error text.
    pub fn exec_stream(
        &self,
        command: &str,
        stdin: Option<&[u8]>,
        timeout: Duration,
        mut on_output: impl FnMut(CommandStream, &[u8]),
    ) -> Result<CommandOutput> {
        ensure!(!command.contains('\0'), "Command contains NUL");
        ensure!(
            (Duration::from_secs(1)..=Duration::from_secs(3600)).contains(&timeout),
            "Command timeout must be between 1 second and 1 hour"
        );
        let stdin = stdin.unwrap_or_default();
        ensure!(stdin.len() <= 4096, "Command stdin exceeds 4096-byte limit");
        let deadline = Instant::now() + timeout;
        let _restore = BlockingRestore {
            session: &self.session,
            blocking: self.session.is_blocking(),
        };
        self.session.set_blocking(false);
        let mut channel = stream_retry(deadline, || self.session.channel_session())
            .context("Cannot open streaming command channel")?;
        let result = (|| {
            stream_retry(deadline, || channel.exec(command))
                .context("Cannot start streaming command")?;
            let mut written = 0;
            while written < stdin.len() {
                stream_deadline(deadline)?;
                match channel.write(&stdin[written..]) {
                    Ok(0) => {
                        return Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "Cannot send command stdin",
                        )
                        .into());
                    }
                    Ok(count) => written += count,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) if channel.eof() => break,
                    Err(error) => return Err(error.into()),
                }
            }
            if let Err(error) = stream_retry(deadline, || channel.send_eof()) {
                // A short-lived command may exit without reading stdin. Still
                // drain its diagnostics and report its actual exit status.
                if !channel.eof() {
                    return Err(error).context("Cannot finish command stdin");
                }
            }
            let mut stdout = OutputTail::default();
            let mut stderr = OutputTail::default();
            let mut buffer = [0u8; 8192];
            loop {
                stream_deadline(deadline)?;
                let mut advanced = false;
                for (id, kind, tail) in [
                    (0, CommandStream::Stdout, &mut stdout),
                    (1, CommandStream::Stderr, &mut stderr),
                ] {
                    match channel.stream(id).read(&mut buffer) {
                        Ok(0) => {}
                        Ok(count) => {
                            tail.push(&buffer[..count]);
                            on_output(kind, &buffer[..count]);
                            advanced = true;
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                if channel.eof() && !advanced {
                    break;
                }
                if !advanced {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            stream_retry(deadline, || channel.wait_close())
                .context("Cannot finish streaming command")?;
            Ok(CommandOutput {
                stdout: stdout.text(),
                stderr: stderr.text(),
                exit_code: channel.exit_status()?,
            })
        })();
        if result.is_err() {
            let _ = channel.close();
        }
        result
    }

    pub fn exec(&self, command: &str) -> Result<CommandOutput> {
        ensure!(!command.contains('\0'), "Command contains NUL");
        let mut channel = self
            .session
            .channel_session()
            .context("Cannot open SSH command channel")?;
        channel
            .exec(command)
            .context("Cannot execute remote command")?;
        self.session.set_blocking(false);
        let result = (|| {
            let deadline = Instant::now() + TIMEOUT;
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut buffer = [0; 8192];
            loop {
                let mut advanced = false;
                for (stream, destination) in [(0, &mut stdout), (1, &mut stderr)] {
                    match channel.stream(stream).read(&mut buffer) {
                        Ok(0) => {}
                        Ok(n) => {
                            destination.extend_from_slice(&buffer[..n]);
                            advanced = true;
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                ensure!(
                    stdout.len() + stderr.len() <= OUTPUT_LIMIT,
                    "Command output exceeded 1 MiB limit"
                );
                if channel.eof() && !advanced {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Remote command timed out after 15 seconds",
                    )
                    .into());
                }
                if !advanced {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            // Waiting for channel-close can itself EAGAIN; poll with deadline.
            loop {
                match channel.wait_close() {
                    Ok(()) => break,
                    Err(e) if e.code() == ErrorCode::Session(-37) && Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(CommandOutput {
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                exit_code: channel.exit_status()?,
            })
        })();
        if result.is_err() {
            let _ = channel.close();
        }
        drop(channel);
        self.session.set_blocking(true);
        result
    }
    pub fn server_info(&self) -> Result<ServerInfo> {
        // Fixed commands avoid interpolation and tolerate non-Linux hosts.
        fn field(session: &RemoteSession, command: &str) -> Result<String> {
            let result = session.exec(command)?;
            if result.exit_code == 0 {
                Ok(result.stdout.trim().to_owned())
            } else {
                Ok(format!("Unavailable: {}", result.stderr.trim()))
            }
        }
        Ok(ServerInfo {
            hostname: field(self, "hostname")?,
            os: field(self, "uname -sr")?,
            uptime: field(self, "uptime")?,
            memory: field(self, "free -h")?,
            disk: field(self, "df -h /")?,
        })
    }
}

fn tree_remote(root: &str, relative: &str) -> String {
    if relative.is_empty() {
        root.to_owned()
    } else {
        format!("{}/{relative}", root.trim_end_matches('/'))
    }
}

fn tree_local(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn scan_local(
    root: &Path,
    relative: &str,
    depth: usize,
    cancel: &AtomicBool,
    plan: &mut TreePlan,
) -> Result<()> {
    check_cancel(cancel)?;
    ensure!(
        depth <= TREE_DEPTH_LIMIT,
        "目录深度超过 {TREE_DEPTH_LIMIT} 层"
    );
    let remaining = TREE_ENTRY_LIMIT - plan.entries.len();
    let mut entries = fs::read_dir(tree_local(root, relative))?
        .take(remaining + 1)
        .collect::<io::Result<Vec<_>>>()?;
    ensure!(
        entries.len() <= remaining,
        "目录超过 {TREE_ENTRY_LIMIT} 个项目"
    );
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        check_cancel(cancel)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("文件名不是有效 Unicode"))?;
        ensure!(
            !name.is_empty() && name != "." && name != ".." && !name.contains('/'),
            "无效文件名"
        );
        validate_path(&name)?;
        let relative = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            !metadata.is_symlink(),
            "目录包含符号链接：{}",
            entry.path().display()
        );
        ensure!(
            metadata.is_dir() || metadata.is_file(),
            "目录包含非普通文件：{}",
            entry.path().display()
        );
        plan.push(
            relative.clone(),
            metadata.is_dir(),
            if metadata.is_dir() { 0 } else { metadata.len() },
        )?;
        if metadata.is_dir() {
            scan_local(root, &relative, depth + 1, cancel, plan)?;
        }
    }
    Ok(())
}

fn validate_download_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0']),
        "不安全的下载文件名：{name}"
    );
    #[cfg(windows)]
    {
        ensure!(
            !name.ends_with(['.', ' ']) && !name.chars().any(|c| c < ' ' || "<>\"|?*".contains(c)),
            "Windows 不支持文件名：{name}"
        );
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit());
        ensure!(!reserved, "Windows 设备名不能用作下载文件名：{name}");
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && !path.contains('\0'),
        "Remote path is empty or contains NUL"
    );
    // ssh2 normalizes backslashes into separators on Windows; reject such Unix
    // filenames rather than accidentally operating on a different remote path.
    ensure!(
        !path.contains('\\'),
        "Remote paths containing backslash are unsupported"
    );
    Ok(())
}
fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .context("Remote path is not valid Unicode")
}
fn remote_partial(path: &str) -> Result<String> {
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    ensure!(
        !name.is_empty() && name != "." && name != "..",
        "Remote destination must name a file"
    );
    Ok(format!("{parent}/.arcnix-{}.part", new_id())
        .trim_start_matches(if parent.is_empty() && !path.starts_with('/') {
            '/'
        } else {
            '\0'
        })
        .to_owned())
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(TransferCancelled.into());
    }
    Ok(())
}
fn copy_with_progress(
    input: &mut impl Read,
    output: &mut impl Write,
    total: u64,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u64, u64),
) -> Result<()> {
    let mut transferred = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    progress(0, total);
    loop {
        check_cancel(cancel)?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        check_cancel(cancel)?;
        output.write_all(&buffer[..count])?;
        transferred += count as u64;
        progress(transferred, total.max(transferred));
    }
    check_cancel(cancel)?;
    ensure!(
        transferred == total,
        "Source file size changed during transfer ({transferred} bytes read, expected {total})"
    );
    Ok(())
}

fn shell_path(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

fn local_modified(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|t| t.as_secs())
}
fn local_target(path: &Path) -> Result<Option<(u64, Option<u64>)>> {
    match fs::symlink_metadata(path) {
        Ok(stat) => {
            ensure!(
                stat.is_file() && !stat.is_symlink(),
                "目标不是普通文件，不能覆盖：{}",
                path.display()
            );
            Ok(Some((stat.len(), local_modified(&stat))))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("无法读取目标文件信息"),
    }
}
fn ensure_local_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(stat) => {
            ensure!(
                stat.is_dir() && !stat.is_symlink(),
                "目标不是普通目录，不能合并：{}",
                path.display()
            );
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).context("无法创建目标目录")
        }
        Err(error) => Err(error).context("无法读取目标目录信息"),
    }
}
fn resolve_conflict(
    callback: &mut impl FnMut(&TransferConflict) -> Result<ConflictAction>,
    conflict: TransferConflict,
    cancel: &AtomicBool,
) -> Result<bool> {
    check_cancel(cancel)?;
    let decision = callback(&conflict)?;
    check_cancel(cancel)?;
    match decision {
        ConflictAction::Overwrite => Ok(false),
        ConflictAction::Skip => Ok(true),
        ConflictAction::Cancel => Err(TransferCancelled.into()),
    }
}
fn replace_local(from: &Path, to: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW};
        let mut from: Vec<u16> = from.as_os_str().encode_wide().collect();
        let mut to: Vec<u16> = to.as_os_str().encode_wide().collect();
        ensure!(!from.contains(&0) && !to.contains(&0), "Path contains NUL");
        from.push(0);
        to.push(0);
        // SAFETY: live NUL-terminated UTF-16 buffers. The same-directory move
        // replaces only after the complete temporary file has been flushed.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_REPLACE_EXISTING) } == 0 {
            return Err(io::Error::last_os_error()).context("无法原子替换本地文件，原文件已保留");
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to).context("无法原子替换本地文件，原文件已保留")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_download_names_cannot_escape_or_alias_windows_files() {
        for name in ["..", ".", "a/b", "a\\b", "D:escape", "file:stream", ""] {
            assert!(validate_download_name(name).is_err(), "{name}");
        }
        #[cfg(windows)]
        for name in [
            "CON",
            "nul.txt",
            "com1.log",
            "LPT9",
            "trailing.",
            "trailing ",
            "x?y",
        ] {
            assert!(validate_download_name(name).is_err(), "{name}");
        }
        assert!(validate_download_name("中文 空格.bin").is_ok());
    }

    #[test]
    fn local_tree_preflight_enforces_depth_before_transfer() -> Result<()> {
        let root = std::env::temp_dir().join(format!("arcnix-tree-scan-{}", new_id()));
        fs::create_dir(&root)?;
        let result = (|| {
            let mut path = root.clone();
            for _ in 0..=TREE_DEPTH_LIMIT {
                path = path.join("d");
                fs::create_dir(&path)?;
            }
            let result = scan_local(
                &root,
                "",
                0,
                &AtomicBool::new(false),
                &mut TreePlan::default(),
            );
            assert!(result.unwrap_err().to_string().contains("目录深度"));
            Ok(())
        })();
        fs::remove_dir_all(root)?;
        result
    }
    #[test]
    fn transfer_cancellation_is_typed_and_stops_copy() {
        let cancel = AtomicBool::new(false);
        let data = vec![42u8; 200_000];
        let mut output = Vec::new();
        let result = copy_with_progress(
            &mut data.as_slice(),
            &mut output,
            data.len() as u64,
            &cancel,
            &mut |done, _| {
                if done > 0 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(result.unwrap_err().is::<TransferCancelled>());
        assert_eq!(output.len(), 65536);
    }
    #[test]
    fn detects_source_size_change() {
        assert!(
            copy_with_progress(
                &mut b"short".as_slice(),
                &mut Vec::new(),
                20,
                &AtomicBool::new(false),
                &mut |_, _| {}
            )
            .is_err()
        );
    }
    #[test]
    fn partial_paths_remain_in_destination_directory() -> Result<()> {
        assert!(remote_partial("/file")?.starts_with("/.arcnix-"));
        assert!(remote_partial("folder/file")?.starts_with("folder/.arcnix-"));
        assert!(remote_partial("file")?.starts_with(".arcnix-"));
        assert!(remote_partial("folder/").is_err());
        Ok(())
    }
}
