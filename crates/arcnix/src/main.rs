#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app_icon;
mod browser;
mod clipboard;
mod file_browser;
mod file_picker;
mod i18n;
mod icons;
mod navigation;
mod observability;
mod secret_output;
mod servers;
mod settings;
mod software;
mod splitter;
mod terminal;
mod terminal_worker;
#[cfg(test)]
mod terminal_worker_tests;
mod transfer_progress;
mod transfer_ui;
mod worker;
#[cfg(test)]
mod worker_conflict_tests;
mod workspace;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use arcnix_core::store::ConfigStore;
use arcnix_core::*;
use browser::{Sort, visible};
use glory::prelude::*;
use glory::ui::{IntoChildren, fragment};
use glory::wry::shell::{Config, Shell, WindowApp};
use icons::{Icon, close_action, icon_action, text_action};
use worker::{Action, Request, Update};

#[derive(Clone, PartialEq, Eq)]
enum Dialog {
    Server,
    Group { id: Option<String> },
    Import,
    Connect,
    File { remote: bool, rename: bool },
    DeleteServer(String),
    DissolveGroup(String),
    DeleteGroupAndServers(String),
    PackageConfirm,
    TransferConflict,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum WorkspacePage {
    #[default]
    Files,
    Software,
    Resources,
    SystemLogs,
    Terminal,
}

#[derive(Clone)]
struct State {
    config: AppConfig,
    selected: Option<String>,
    group: Option<String>,
    collapsed_groups: HashSet<String>,
    search: String,
    editor: ServerProfile,
    dialog: Option<Dialog>,
    dialog_error: String,
    host_menu: Option<String>,
    group_menu: Option<String>,
    file_menu: Option<bool>,
    file_pending: Option<u64>,
    import_pending: Option<u64>,
    activity_tab: u8,
    activity_open: bool,
    local_filter_open: bool,
    remote_filter_open: bool,
    notice: String,
    group_name: String,
    port: String,
    secret: String,
    connected: bool,
    sftp_available: bool,
    connecting: bool,
    fingerprint: Option<String>,
    epoch: u64,
    request_serial: u64,
    local_request: u64,
    remote_request: u64,
    local_filter: String,
    remote_filter: String,
    local_sort: Sort,
    remote_sort: Sort,
    local_desc: bool,
    remote_desc: bool,
    queue_paused: bool,
    queue_filter: u8,
    import_path: String,
    local_path: String,
    remote_path: String,
    local_ready: bool,
    remote_ready: bool,
    local: Vec<FileEntry>,
    remote: Vec<FileEntry>,
    local_selection: HashSet<String>,
    remote_selection: HashSet<String>,
    file_browser: file_browser::BrowserState,
    terminal: terminal::TerminalState,
    name: String,
    jobs: Vec<TransferJob>,
    cancels: HashMap<String, Arc<AtomicBool>>,
    logs: String,
    workspace_page: WorkspacePage,
    navigation: navigation::Page,
    settings_section: navigation::SettingsSection,
    unread_notifications: usize,
    software: software::SoftwareState,
    metrics: observability::MetricState,
    system_logs: observability::LogsState,
    system_task_running: bool,
    settings: settings::SettingsState,
    system_dark: bool,
    file_split: f64,
    transfer_conflict: Option<transfer_ui::PendingConflict>,
    batch_ids: HashMap<String, String>,
    queue_rules: HashMap<(String, TransferDirection), arcnix_core::transfer::ConflictAction>,
    session_rules: HashMap<TransferDirection, arcnix_core::transfer::ConflictAction>,
    transfer_now: std::time::Instant,
    transfer_meters: HashMap<String, transfer_progress::Meter>,
    transfer_summaries: HashMap<String, arcnix_core::transfer::TransferSummary>,
}
#[derive(Clone)]
struct Ui {
    state: Cage<State>,
    tx: Sender<Request>,
    store: Rc<ConfigStore>,
    writable: bool,
    scope: Scope,
    shell_focus: Cage<Option<glory::render::NodeId>>,
}
impl Ui {
    fn apply_conflict_rule(
        &self,
        job_id: &str,
        direction: TransferDirection,
        decision: worker::ConflictDecision,
    ) {
        use arcnix_core::transfer::ConflictAction;
        if decision.action == ConflictAction::Cancel {
            self.cancel_all();
            return;
        }
        self.state.revise(|s| match decision.scope {
            worker::ConflictScope::ThisFile => {}
            worker::ConflictScope::ThisQueue => {
                if let Some(batch) = s.batch_ids.get(job_id) {
                    s.queue_rules
                        .insert((batch.clone(), direction), decision.action);
                }
            }
            worker::ConflictScope::ThisSession | worker::ConflictScope::FutureConnections => {
                s.session_rules.insert(direction, decision.action);
            }
        });
        if decision.scope == worker::ConflictScope::FutureConnections {
            let action = match decision.action {
                ConflictAction::Overwrite => ExistingFileAction::Overwrite,
                _ => ExistingFileAction::Skip,
            };
            let result = self.state.revise(|s| {
                s.config.set_transfer_rule(
                    s.selected.as_deref().unwrap_or(""),
                    direction,
                    Some(action),
                )
            });
            if let Err(error) = result {
                self.error(error.to_string());
            } else {
                self.save();
            }
        }
    }
    fn reset_conflict_rules(&self, server_id: &str) {
        self.state.revise(|s| {
            s.config
                .transfer_rules
                .retain(|rule| rule.server_id != server_id);
            if s.selected.as_deref() == Some(server_id) {
                s.session_rules.clear();
                s.queue_rules.clear();
            }
            s.host_menu = None;
        });
        self.save();
        self.log("该服务器的文件冲突处理已恢复为每次询问");
    }
    fn select_server(&self, id: String) {
        if self
            .state
            .with(|s| s.system_task_running && s.selected.as_ref() != Some(&id))
        {
            return;
        }
        if self.state.with(|s| s.selected.as_ref() != Some(&id)) {
            self.disconnect();
            self.state.revise(|s| {
                s.file_split = s
                    .config
                    .servers
                    .iter()
                    .find(|p| p.id == id)
                    .and_then(|p| p.file_pane_ratio)
                    .unwrap_or(500) as f64
                    / 1000.0;
                s.selected = Some(id);
                s.workspace_page = WorkspacePage::Files;
            });
        }
        self.state.revise(|s| {
            s.navigation = navigation::Page::Servers;
            s.host_menu = None;
            s.group_menu = None;
        });
        self.observability_enter();
        self.terminal_enter();
    }
    fn save_file_split(&self, ratio: f64) {
        let ratio = if ratio.is_finite() {
            ratio.clamp(0.2, 0.8)
        } else {
            0.5
        };
        let saved = self.state.revise(|s| {
            s.file_split = ratio;
            s.selected.clone().is_some_and(|id| {
                s.config
                    .set_file_pane_ratio(&id, (ratio * 1000.0).round() as u16)
                    .is_ok()
            })
        });
        if saved {
            self.save();
        }
    }
    fn open_dialog(&self, dialog: Dialog) {
        self.state.revise(|s| {
            s.dialog = Some(dialog);
            s.dialog_error.clear();
            s.host_menu = None;
            s.group_menu = None;
            s.file_menu = None;
        });
    }
    fn close_dialog(&self) {
        if self
            .state
            .with(|s| s.dialog == Some(Dialog::TransferConflict))
            && let Some(pending) = self.state.revise(|s| s.transfer_conflict.take())
        {
            self.cancel_all();
            let _ = pending.reply.send(worker::ConflictDecision {
                action: arcnix_core::transfer::ConflictAction::Cancel,
                scope: worker::ConflictScope::ThisFile,
            });
        }
        self.state.revise(|s| {
            if s.file_pending.is_some() || s.import_pending.is_some() {
                return;
            }
            if s.dialog == Some(Dialog::Connect) {
                s.secret.clear();
            }
            if s.dialog == Some(Dialog::PackageConfirm) {
                s.software.sudo_secret.clear();
            }
            s.dialog = None;
            s.dialog_error.clear();
        });
        if self.state.with(|s| s.dialog.is_none())
            && let Some(id) = self.shell_focus.get()
        {
            let _ = self.scope.action(glory::render::Action::Focus(id));
        }
        if self
            .state
            .with(|s| s.dialog.is_none() && s.transfer_conflict.is_some())
        {
            self.open_dialog(Dialog::TransferConflict);
        }
    }
    fn error(&self, message: impl AsRef<str>) {
        self.state
            .revise(|s| s.notice = message.as_ref().to_string());
        self.log(message);
    }
    fn log(&self, message: impl AsRef<str>) {
        let message = i18n::translate(message.as_ref());
        self.state.revise(|s| {
            if !s.logs.is_empty() && !s.logs.ends_with('\n') {
                s.logs.push('\n');
            }
            if s.navigation != navigation::Page::Notifications {
                s.unread_notifications = s.unread_notifications.saturating_add(1);
            }
            s.logs.push_str(&message);
            s.logs.push('\n');
            if s.logs.len() > 60000 {
                let boundary = s
                    .logs
                    .char_indices()
                    .map(|(i, _)| i)
                    .find(|i| *i >= 20000)
                    .unwrap_or(0);
                s.logs.drain(..boundary);
            }
        });
    }
    fn send(&self, action: Action) -> u64 {
        let (epoch, serial) = self.state.revise(|s| {
            s.request_serial += 1;
            (s.epoch, s.request_serial)
        });
        if self
            .tx
            .send(Request {
                epoch,
                serial,
                action,
            })
            .is_err()
        {
            self.log("后台任务已停止");
        }
        serial
    }
    fn retry(&self, id: Option<String>) {
        if !self.state.with(|s| s.connected) {
            self.log("请先连接原服务器再重试传输");
            return;
        }
        self.state.revise(|s| {
            let active = s.cancels.keys().cloned().collect();
            browser::retry(&mut s.jobs, &active, s.selected.as_deref(), id.as_deref());
            s.transfer_meters
                .retain(|id, _| s.jobs.iter().any(|j| &j.id == id));
        });
        self.schedule();
    }
    fn save(&self) {
        if !self.writable {
            self.log("配置读取失败，已禁止覆盖。请修复配置文件后重新启动。");
            return;
        }
        let config = self.state.with(|s| s.config.clone());
        if let Err(e) = self.store.save(&config) {
            self.log(format!("保存失败: {e:#}"));
        }
    }
    fn connect(&self) {
        if self.state.with(|s| s.system_task_running) {
            self.error("系统软件任务正在执行，请等待完成后切换服务器");
            return;
        }
        let profile = self.state.with(|s| {
            s.config
                .servers
                .iter()
                .find(|p| Some(&p.id) == s.selected.as_ref())
                .cloned()
        });
        if let Some(profile) = profile {
            self.cancel_all();
            self.state.revise(|s| {
                s.epoch += 1;
                s.cancels.clear();
                s.connected = false;
                s.connecting = true;
                s.terminal = Default::default();
                s.fingerprint = None;
                s.remote_ready = false;
                s.remote.clear();
                s.remote_selection.clear();
                s.remote_path = profile.initial_remote_path.clone();
                s.software = Default::default();
                s.metrics = Default::default();
                s.system_logs = Default::default();
            });
            let known = self.state.with(|s| {
                s.config
                    .trusted_fingerprint(&profile.host, profile.port)
                    .map(str::to_owned)
            });
            self.send(Action::Probe(profile, known));
        } else {
            self.log("请先新建并选择服务器");
        }
    }
    fn disconnect(&self) {
        if self.state.with(|s| s.system_task_running) {
            self.error("系统软件任务正在执行，请等待完成后断开连接");
            return;
        }
        self.cancel_all();
        if self
            .state
            .with(|s| s.dialog == Some(Dialog::TransferConflict))
        {
            self.close_dialog();
        }
        self.state.revise(|s| {
            s.epoch += 1;
            s.terminal = Default::default();
            s.cancels.clear();
            s.connected = false;
            s.sftp_available = false;
            s.connecting = false;
            s.fingerprint = None;
            s.secret.clear();
            s.remote_ready = false;
            s.remote.clear();
            s.remote_selection.clear();
            s.remote_request = 0;
            s.software = Default::default();
            s.metrics = Default::default();
            s.system_logs = Default::default();
            s.session_rules.clear();
            s.queue_rules.clear();
        });
        self.send(Action::Disconnect);
    }
    fn cancel_all(&self) {
        self.state.revise(|s| {
            s.transfer_now = std::time::Instant::now();
            for m in s.transfer_meters.values_mut() {
                m.finish(s.transfer_now);
            }
            for c in s.cancels.values() {
                c.store(true, Ordering::Relaxed);
            }
            for j in &mut s.jobs {
                if matches!(j.status, TransferStatus::Queued | TransferStatus::Running) {
                    j.status = TransferStatus::Cancelled;
                }
            }
        });
    }
    fn authenticate(&self, remember: bool) {
        let details = self.state.with(|s| {
            let profile = s
                .config
                .servers
                .iter()
                .find(|p| Some(&p.id) == s.selected.as_ref())?;
            Some((
                profile.host.clone(),
                profile.port,
                s.fingerprint.clone()?,
                s.secret.clone(),
            ))
        });
        if let Some((host, port, fingerprint, secret)) = details {
            if remember {
                let result = self
                    .state
                    .revise(|s| s.config.trust_host(&host, port, &fingerprint));
                if let Err(e) = result {
                    self.log(e.to_string());
                    return;
                }
                self.save();
            }
            self.state.revise(|s| {
                s.fingerprint = None;
                s.secret.clear();
            });
            self.send(Action::Authenticate {
                secret,
                fingerprint,
            });
        }
    }
    fn refresh(&self, remote: bool) {
        let path = self.state.revise(|s| {
            s.file_browser.reset(remote);
            if remote {
                s.remote_ready = false;
                s.remote_selection.clear();
                s.remote_path.clone()
            } else {
                s.local_ready = false;
                s.local_selection.clear();
                s.local_path.clone()
            }
        });
        let serial = self.send(if remote {
            Action::Remote(path)
        } else {
            Action::Local(path.into())
        });
        self.state.revise(|s| {
            if remote {
                s.remote_request = serial
            } else {
                s.local_request = serial
            }
        });
    }
    fn transfer(&self, direction: TransferDirection) {
        if direction == TransferDirection::Download
            && self.state.with(|s| {
                file_browser::selected_entries(s, true)
                    .iter()
                    .any(|f| !safe_local_name(&f.name))
            })
        {
            self.log("下载已拒绝：所选远程名称无法安全保存为本地文件名");
            return;
        }
        let jobs = self.state.with(|s| {
            if !s.connected || !s.local_ready || !s.remote_ready {
                return Vec::new();
            }
            let Some(server_id) = s.selected.clone() else {
                return Vec::new();
            };
            let entries =
                file_browser::selected_entries(s, direction == TransferDirection::Download);
            entries
                .iter()
                .map(|f| TransferJob {
                    id: new_id(),
                    server_id: server_id.clone(),
                    direction,
                    local_path: if direction == TransferDirection::Upload {
                        PathBuf::from(&f.path)
                    } else {
                        Path::new(&s.local_path).join(&f.name)
                    },
                    remote_path: if direction == TransferDirection::Download {
                        f.path.clone()
                    } else {
                        remote_join(&s.remote_path, &f.name)
                    },
                    total_bytes: if f.is_dir { 0 } else { f.size },
                    transferred_bytes: 0,
                    status: TransferStatus::Queued,
                })
                .collect::<Vec<_>>()
        });
        if jobs.is_empty() {
            self.log("请先连接并成功读取两个目录，再选择文件或目录；不传输符号链接");
            return;
        }
        let batch = new_id();
        self.state.revise(|s| {
            for job in &jobs {
                s.batch_ids.insert(job.id.clone(), batch.clone());
            }
            s.jobs.extend(jobs);
        });
        self.schedule();
    }
    fn schedule(&self) {
        let next = self.state.revise(|s| {
            if s.system_task_running
                || !s.connected
                || s.queue_paused
                || !s.cancels.is_empty()
                || s.jobs.iter().any(|j| j.status == TransferStatus::Running)
            {
                return None;
            }
            let job = s.jobs.iter_mut().find(|j| {
                j.status == TransferStatus::Queued && Some(&j.server_id) == s.selected.as_ref()
            })?;
            job.status = TransferStatus::Running;
            s.transfer_now = std::time::Instant::now();
            s.transfer_meters.insert(
                job.id.clone(),
                transfer_progress::Meter::new(s.transfer_now),
            );
            let cancel = Arc::new(AtomicBool::new(false));
            s.cancels.insert(job.id.clone(), cancel.clone());
            Some((job.clone(), cancel))
        });
        if let Some((job, cancel)) = next {
            use arcnix_core::transfer::ConflictAction;
            let policy = self.state.with(|s| {
                s.batch_ids
                    .get(&job.id)
                    .and_then(|batch| s.queue_rules.get(&(batch.clone(), job.direction)))
                    .copied()
                    .or_else(|| s.session_rules.get(&job.direction).copied())
                    .or_else(|| {
                        s.config
                            .transfer_rules
                            .iter()
                            .find(|rule| {
                                rule.server_id == job.server_id && rule.direction == job.direction
                            })
                            .map(|rule| match rule.action {
                                ExistingFileAction::Overwrite => ConflictAction::Overwrite,
                                ExistingFileAction::Skip => ConflictAction::Skip,
                            })
                    })
            });
            self.send(Action::TransferInteractive {
                job,
                cancel,
                policy,
            });
        }
    }
    fn mutate_file(&self, remote: bool, rename: bool) {
        let operation = self.state.with(|s| {
            if (remote && !s.remote_ready) || (!remote && !s.local_ready) {
                return Err("请先成功读取目标目录");
            }
            let name = s.name.trim();
            if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
                return Err("请输入不含路径分隔符的文件名");
            }
            if !remote && !safe_local_name(name) {
                return Err("本地名称包含无效字符、设备名或路径前缀");
            }
            let base = if remote {
                &s.remote_path
            } else {
                &s.local_path
            };
            let to = if remote {
                remote_join(base, name)
            } else {
                Path::new(base).join(name).to_string_lossy().into_owned()
            };
            if rename {
                let selected = if remote {
                    &s.remote_selection
                } else {
                    &s.local_selection
                };
                if selected.len() != 1 {
                    return Err("重命名时请选择一个项目");
                }
                let from = selected.iter().next().unwrap().clone();
                let to = if remote {
                    remote_join(
                        from.rsplit_once('/')
                            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
                            .unwrap_or("."),
                        name,
                    )
                } else {
                    Path::new(&from)
                        .parent()
                        .unwrap_or_else(|| Path::new(base))
                        .join(name)
                        .to_string_lossy()
                        .into_owned()
                };
                Ok(if remote {
                    Action::RemoteRename(from, to)
                } else {
                    Action::LocalRename(from.into(), to.into())
                })
            } else {
                Ok(if remote {
                    Action::RemoteMkdir(to)
                } else {
                    Action::LocalMkdir(to.into())
                })
            }
        });
        match operation {
            Ok(action) => {
                let serial = self.send(action);
                self.state.revise(|s| {
                    s.file_pending = Some(serial);
                    s.dialog_error.clear();
                });
            }
            Err(e) => self.state.revise(|s| s.dialog_error = e.into()),
        }
    }
}
fn import_profiles(
    existing: &AppConfig,
    profiles: Vec<ServerProfile>,
) -> anyhow::Result<(AppConfig, usize, usize)> {
    let mut config = existing.clone();
    let mut imported = 0;
    let mut skipped = 0;
    for mut profile in profiles {
        if config.servers.iter().any(|p| {
            p.host.eq_ignore_ascii_case(&profile.host)
                && p.port == profile.port
                && p.username == profile.username
                && p.auth == profile.auth
                && p.private_key == profile.private_key
        }) {
            skipped += 1;
            continue;
        }
        let group = if let Some(g) = config
            .groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case("SSH Config"))
        {
            g.id.clone()
        } else {
            config.add_group("SSH Config")?
        };
        profile.group_id = Some(group);
        config.upsert_server(profile)?;
        imported += 1;
    }
    config.validate()?;
    Ok((config, imported, skipped))
}
fn pane_entries(s: &State, remote: bool) -> Vec<FileEntry> {
    file_browser::entries(s, remote)
}
fn safe_local_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '\0']) {
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        if name.ends_with(['.', ' ']) || name.chars().any(|c| c < ' ' || "<>\"|?*".contains(c)) {
            return false;
        }
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return false;
        }
    }
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none()
}
fn remote_join(base: &str, name: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), name)
}
fn local_display(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{unc}");
    }
    value.strip_prefix("\\\\?\\").unwrap_or(&value).to_string()
}
fn command_display(command: &str) -> String {
    command
        .lines()
        .filter(|line| !line.starts_with("# arcnix:packages:"))
        .collect::<Vec<_>>()
        .join("\n")
}
fn accepts_listing(latest: u64, received: u64) -> bool {
    latest != 0 && latest == received
}
fn accepts_response(current: u64, received: u64) -> bool {
    current == received
}
fn finished_status(result: std::result::Result<(), String>, cancelled: bool) -> TransferStatus {
    match result {
        Ok(()) => TransferStatus::Completed,
        Err(_) if cancelled => TransferStatus::Cancelled,
        Err(e) => TransferStatus::Failed(e),
    }
}
fn modified(timestamp: Option<u64>) -> String {
    let Some(timestamp) = timestamp else {
        return "—".into();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = now.saturating_sub(timestamp);
    let label = if seconds < 60 {
        "刚刚".into()
    } else if seconds < 3600 {
        format!("{} 分钟前", seconds / 60)
    } else if seconds < 86400 {
        format!("{} 小时前", seconds / 3600)
    } else {
        format!("{} 天前", seconds / 86400)
    };
    i18n::translate(&label)
}
fn size(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.1} KB", bytes as f64 / 1024.)
    } else {
        format!("{bytes} B")
    }
}
fn btn(label: impl Into<String>, action: impl FnMut() + 'static) -> sapid::Button {
    sapid::button()
        .label(i18n::bind(label.into()))
        .class("button")
        .on_press(action)
}
fn field(
    label: &'static str,
    ui: &Ui,
    read: fn(&State) -> String,
    write: fn(&mut State, String),
) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(read))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        write(s, value);
                        s.dialog_error.clear();
                    });
                }
            }),
    ))
}
fn modal(
    ui: Ui,
    title: impl Into<String>,
    body: impl IntoChildren + 'static,
    footer: impl IntoChildren + 'static,
) -> impl Widget {
    let mut content = Some((title.into(), body, footer));
    widget(move |scope| {
        let (title, body, footer) = content
            .take()
            .expect("dialog builds once per mounted scope");
        let close = ui.clone();
        let scrim = ui.clone();
        let focus = scope.cage(None);
        let mounted = scope.mount_with(|| {
            col().class("modal-backdrop").fill((
                button()
                    .class("modal-scrim")
                    .label(i18n::bind("关闭对话框"))
                    .focus(FocusMode::Programmatic)
                    .enabled(
                        ui.state
                            .map(|s| s.file_pending.is_none() && s.import_pending.is_none()),
                    )
                    .on(activate, move |_| scrim.close_dialog()),
                col()
                    .class("modal-card")
                    .role(Role::Dialog)
                    .name(i18n::bind(title.clone()))
                    .focus(FocusMode::Programmatic)
                    .node_ref(focus)
                    .fill((
                        row().class("modal-header").fill((
                            text().class("dialog-title").content(i18n::bind(title)),
                            close_action("关闭对话框")
                                .enabled(ui.state.map(|s| {
                                    s.file_pending.is_none() && s.import_pending.is_none()
                                }))
                                .on_press(move || close.close_dialog()),
                        )),
                        col()
                            .class("modal-body")
                            .semantics(ui.state.map(|s| {
                                Some(Box::new(Semantics {
                                    inert: s.file_pending.is_some() || s.import_pending.is_some(),
                                    ..Default::default()
                                }))
                            }))
                            .fill(body),
                        text()
                            .class(ui.state.map(|s| {
                                if s.dialog_error.is_empty() {
                                    "hidden"
                                } else {
                                    "form-error"
                                }
                                .to_string()
                            }))
                            .content(i18n::bind(ui.state.map(|s| s.dialog_error.clone()))),
                        row().class("modal-footer").fill(footer),
                    )),
            ))
        })?;
        scope.retain(mounted)?;
        if let Some(id) = focus.get() {
            scope.action(glory::render::Action::Focus(id))?;
        }
        Ok(())
    })
}

fn module_tab(ui: Ui, page: WorkspacePage, label: &'static str) -> impl Widget {
    sapid::button()
        .class(ui.state.map(move |s| {
            if s.workspace_page == page {
                "module-tab active"
            } else {
                "module-tab"
            }
            .to_string()
        }))
        .label(i18n::bind(label))
        .on_press(move || {
            ui.state.revise(|s| s.workspace_page = page);
            ui.observability_enter();
            ui.terminal_enter();
        })
}

fn connection_controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    row().class("connection-controls").fill((
        col().class("connection-copy").fill((
            text().class("connection-target").content(state.map(|s| {
                s.config
                    .servers
                    .iter()
                    .find(|p| Some(&p.id) == s.selected.as_ref())
                    .map(|p| p.name.clone())
                    .unwrap_or_default()
            })),
            text()
                .class(state.map(|s| {
                    if s.connected {
                        "connection-state online"
                    } else if s.connecting {
                        "connection-state connecting"
                    } else {
                        "connection-state"
                    }
                    .to_string()
                }))
                .content(i18n::bind(state.map(|s| {
                    if s.connected {
                        "SSH 已连接"
                    } else if s.connecting {
                        "SSH 正在连接…"
                    } else if s.config.servers.is_empty() {
                        "尚未添加服务器"
                    } else if s.selected.is_none() {
                        "请选择左侧服务器"
                    } else {
                        "SSH 未连接"
                    }
                    .to_string()
                }))),
        )),
        sapid::button()
            .class(state.map(|s| {
                if s.connected || s.connecting {
                    "button connection-button"
                } else {
                    "button primary connection-button"
                }
                .to_string()
            }))
            .label(i18n::bind(state.map(|s| {
                if s.connected {
                    "断开连接"
                } else if s.connecting {
                    "取消连接"
                } else if s.config.servers.is_empty() {
                    "添加服务器"
                } else {
                    "连接服务器"
                }
                .to_string()
            })))
            .enabled(state.map(|s| {
                !s.system_task_running && (s.selected.is_some() || s.config.servers.is_empty())
            }))
            .on_press(move || {
                if state.with(|s| s.connected || s.connecting) {
                    ui.disconnect();
                } else if state.with(|s| s.config.servers.is_empty()) {
                    servers::new_server(&ui);
                } else {
                    ui.open_dialog(Dialog::Connect);
                }
            }),
    ))
}

fn app(ui: Ui) -> impl Widget {
    let state = ui.state;
    let menus = ui.clone();
    let trust = ui.clone();
    let once = ui.clone();
    let reject = ui.clone();
    let keyboard_ui = ui.clone();
    col()
        .class(state.map(|s| {
            if s.config.settings.theme.is_dark(s.system_dark) {
                "app theme-dark"
            } else {
                "app theme-light"
            }
            .to_string()
        }))
        .on(keyboard, move |event| {
            if let Event::Keyboard {
                command: glory::render::KeyboardCommand::Dismiss,
                ..
            } = event
            {
                if keyboard_ui.state.with(|s| s.fingerprint.is_some()) {
                    keyboard_ui.disconnect();
                } else {
                    keyboard_ui.close_dialog();
                    keyboard_ui.state.revise(|s| {
                        s.host_menu = None;
                        s.group_menu = None;
                        s.file_menu = None;
                    });
                }
            }
        })
        .fill((
            input()
                .class("system-theme-input hidden")
                .value("")
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        let dark = match value.as_str() {
                            "dark" => true,
                            "light" => false,
                            _ => return,
                        };
                        if state.with(|s| s.system_dark != dark) {
                            state.revise(|s| s.system_dark = dark);
                        }
                    }
                }),
            col()
                .class("app-shell")
                .focus(FocusMode::Programmatic)
                .node_ref(ui.shell_focus)
                .semantics(state.map(|s| {
                    Some(Box::new(Semantics {
                        inert: s.dialog.is_some() || s.fingerprint.is_some(),
                        ..Default::default()
                    }))
                }))
                .fill(navigation::shell(ui.clone())),
            button()
                .class(state.map(|s| {
                    if s.dialog.is_none()
                        && (s.host_menu.is_some()
                            || s.group_menu.is_some()
                            || s.file_menu.is_some())
                    {
                        "menu-dismiss"
                    } else {
                        "hidden"
                    }
                    .to_string()
                }))
                .label(i18n::bind("关闭操作菜单"))
                .focus(FocusMode::Programmatic)
                .on(activate, move |_| {
                    menus.state.revise(|s| {
                        s.host_menu = None;
                        s.group_menu = None;
                        s.file_menu = None;
                    })
                }),
            servers::dialogs(ui.clone()),
            workspace::file_dialog(ui.clone()),
            software::dialogs(ui.clone()),
            transfer_ui::dialogs(ui.clone()),
            when(state.map(|s| s.fingerprint.is_some()), move || {
                let trust = trust.clone();
                let once = once.clone();
                let reject = reject.clone();
                modal(
                    ui.clone(),
                    "确认服务器身份",
                    col().class("dialog-stack").fill((
                        text().content(i18n::bind("首次连接此服务器，请核对 SSH 主机指纹。")),
                        text()
                            .class("fingerprint")
                            .selectable(true)
                            .content(state.map(|s| s.fingerprint.clone().unwrap_or_default())),
                        text().class("hint").content(i18n::bind(
                            "确认指纹后再继续认证。可选择仅本次信任，或保存供下次连接校验。",
                        )),
                    )),
                    (
                        btn("取消", move || reject.disconnect()),
                        btn("仅本次信任", move || once.authenticate(false)),
                        sapid::button()
                            .class("button primary")
                            .label(i18n::bind("信任并保存"))
                            .on_press(move || trust.authenticate(true)),
                    ),
                )
            }),
        ))
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let store = Rc::new(match args.next().as_deref() {
        Some("--config") => {
            let path = args.next().ok_or("--config requires a path")?;
            if args.next().is_some() {
                return Err("Unexpected argument".into());
            }
            ConfigStore::at(path.into())
        }
        Some("--help") => {
            println!("Arcnix desktop\nUsage: arcnix [--config <path>]");
            return Ok(());
        }
        Some(_) => return Err("Usage: arcnix [--config <path>]".into()),
        None => ConfigStore::default_location()?,
    });
    let (config, error) = match store.load() {
        Ok(c) => (c, None),
        Err(e) => (
            AppConfig::default(),
            Some(format!("配置读取失败，禁止覆盖: {e:#}")),
        ),
    };
    let path = config
        .local_directory
        .clone()
        .unwrap_or(std::env::current_dir()?);
    let (tx, rx) = worker::start();
    let window = WindowApp::new();
    let scope = window.app().scope();
    i18n::init(scope, config.settings.language);
    let state = scope.cage(State {
        config,
        selected: None,
        group: None,
        collapsed_groups: HashSet::new(),
        search: String::new(),
        editor: ServerProfile::default(),
        dialog: None,
        dialog_error: String::new(),
        host_menu: None,
        group_menu: None,
        file_menu: None,
        file_pending: None,
        import_pending: None,
        activity_tab: 0,
        activity_open: true,
        local_filter_open: false,
        remote_filter_open: false,
        notice: error.clone().unwrap_or_default(),
        group_name: String::new(),
        port: "22".into(),
        secret: String::new(),
        connected: false,
        sftp_available: false,
        connecting: false,
        fingerprint: None,
        epoch: 0,
        request_serial: 0,
        local_request: 0,
        remote_request: 0,
        local_filter: String::new(),
        remote_filter: String::new(),
        local_sort: Sort::Name,
        remote_sort: Sort::Name,
        local_desc: false,
        remote_desc: false,
        queue_paused: false,
        queue_filter: 0,
        import_path: String::new(),
        local_path: local_display(&path),
        remote_path: ".".into(),
        local_ready: false,
        remote_ready: false,
        local: vec![],
        remote: vec![],
        local_selection: HashSet::new(),
        remote_selection: HashSet::new(),
        file_browser: Default::default(),
        terminal: Default::default(),
        name: String::new(),
        jobs: vec![],
        cancels: HashMap::new(),
        logs: error
            .clone()
            .map(|message| i18n::translate(&message))
            .unwrap_or_else(|| {
                i18n::translate("Arcnix 已就绪。新建服务器并连接，或浏览本地目录。")
            }),
        workspace_page: WorkspacePage::Files,
        navigation: Default::default(),
        settings_section: Default::default(),
        unread_notifications: 0,
        software: Default::default(),
        metrics: Default::default(),
        system_logs: Default::default(),
        system_task_running: false,
        settings: Default::default(),
        system_dark: true,
        file_split: 0.5,
        transfer_conflict: None,
        batch_ids: HashMap::new(),
        queue_rules: HashMap::new(),
        session_rules: HashMap::new(),
        transfer_now: std::time::Instant::now(),
        transfer_meters: HashMap::new(),
        transfer_summaries: HashMap::new(),
    });
    let ui = Ui {
        state,
        tx,
        store,
        writable: error.is_none(),
        scope: scope.clone(),
        shell_focus: scope.cage(None),
    };
    ui.refresh(false);
    // Coalesce ticks while the UI is busy. Sampling itself is also gated by
    // the active page and a single pending request in observability state.
    let (ticks, tick_rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if let Err(async_channel::TrySendError::Closed(_)) = ticks.try_send(()) {
                break;
            }
        }
    });
    let polling = ui.clone();
    scope.spawn(async move {
        while tick_rx.recv().await.is_ok() {
            polling.observability_tick();
            polling.transfer_tick();
        }
    })?;
    let events = ui.clone();
    scope.spawn(async move {while let Ok(response)=rx.recv().await {
        if !accepts_response(state.with(|s|s.epoch),response.epoch) && !matches!(&response.update, Update::Children {remote:false,..}){continue;}
        match response.update {
            Update::Terminal(event)=>terminal::update(&events,event),
            Update::Fingerprint(fingerprint)=>{
                let known=state.with(|s|s.config.servers.iter().find(|p|Some(&p.id)==s.selected.as_ref()).and_then(|p|s.config.trusted_fingerprint(&p.host,p.port)).map(str::to_owned));
                if known.as_ref().is_some_and(|f|f!=&fingerprint){
                    events.disconnect();events.log(format!("拒绝连接：主机密钥发生变化。已保存：{}；当前：{fingerprint}。请通过可信渠道核实后手动更新配置。",known.unwrap()));
                }else{state.revise(|s|s.fingerprint=Some(fingerprint));if known.is_some(){events.authenticate(false);}}
            }
            Update::Connected(sftp)=>{state.revise(|s|{s.connected=true;s.connecting=false;s.sftp_available=sftp;});events.log("SSH 认证成功");if sftp{events.refresh(true);}else{events.log("服务器未启用 SFTP；SSH 系统软件管理仍可使用。");}events.terminal_enter();events.observability_enter();if state.with(|s|s.workspace_page==WorkspacePage::Software){events.software_detect();}}
            Update::Disconnected=>events.log("连接已断开"),
            Update::Local(path,entries)=>{if !accepts_listing(state.with(|s|s.local_request),response.serial){continue;}state.revise(|s|{s.file_browser.reset(false);s.local_path=local_display(&path);s.local_ready=true;s.local=entries;s.local_selection.clear();s.config.local_directory=Some(path);});events.save();}
            Update::Remote(path,entries)=>{if !accepts_listing(state.with(|s|s.remote_request),response.serial){continue;}state.revise(|s|{s.file_browser.reset(true);s.remote_path=path;s.remote_ready=true;s.remote=entries;s.remote_selection.clear();});},
            Update::Children{remote,parent,result}=>file_browser::accept_children(&events,remote,parent,response.serial,result),
            Update::BrowseError{remote,message}=>{let latest=state.with(|s|if remote{s.remote_request}else{s.local_request});if accepts_listing(latest,response.serial){events.error(format!("读取目录失败：{message}"));}},
            Update::ConnectionLost(message)=>{terminal::disconnected(&events);software::on_connection_lost(&events,&message);state.revise(|s|{s.metrics=Default::default();s.system_logs=Default::default();s.session_rules.clear();s.queue_rules.clear();s.transfer_conflict=None;if s.dialog==Some(Dialog::TransferConflict){s.dialog=None;}s.connected=false;s.connecting=false;s.remote_ready=false;s.remote.clear();s.remote_selection.clear();s.remote_request=0;s.fingerprint=None;for j in &mut s.jobs {if j.status==TransferStatus::Queued{j.status=TransferStatus::Cancelled;}}});events.log(format!("SSH 连接已断开：{message}"));},
            Update::TransferConflict(id,info,reply)=>transfer_ui::show(&events,id,info,reply),
            Update::TransferReport(id,summary)=>{events.log(format!("传输结果：写入 {} 个文件，跳过 {} 个文件",summary.transferred_files,summary.skipped_files));state.revise(|s|{s.transfer_summaries.insert(id,summary);});},
            Update::Progress(id,done,total,actual)=>state.revise(|s|{s.transfer_now=std::time::Instant::now();if let Some(m)=s.transfer_meters.get_mut(&id){m.observe(actual,s.transfer_now);}
                if let Some(job)=s.jobs.iter_mut().find(|j|j.id==id){job.transferred_bytes=done;job.total_bytes=total;}}),
            Update::Finished(id,result)=>{state.revise(|s|{if s.transfer_conflict.as_ref().is_some_and(|c|c.job_id==id){s.transfer_conflict=None;if s.dialog==Some(Dialog::TransferConflict){s.dialog=None;}}});if let Err(error)=&result{events.log(format!("传输未完成：{error}"));}state.revise(|s|{s.transfer_now=std::time::Instant::now();if let Some(m)=s.transfer_meters.get_mut(&id){m.finish(s.transfer_now);}let cancelled=s.cancels.remove(&id).is_some_and(|c|c.load(Ordering::Relaxed));if let Some(job)=s.jobs.iter_mut().find(|j|j.id==id){job.status=finished_status(result,cancelled);}});events.refresh(false);if state.with(|s|s.connected){events.refresh(true);}events.schedule();}
            Update::Log(message)=>{
                events.log(message);
                let file=state.with(|s|if s.file_pending==Some(response.serial){s.dialog.clone()}else{None});
                if let Some(Dialog::File{remote,..})=file {state.revise(|s|{s.file_pending=None;s.dialog=None;s.dialog_error.clear();s.notice.clear();});events.refresh(remote);}
            },
            Update::Error(message)=>{
                let pending=state.with(|s|s.file_pending==Some(response.serial)||s.import_pending==Some(response.serial));
                if pending {state.revise(|s|{s.file_pending=None;s.import_pending=None;s.dialog_error=message.clone();});events.log(format!("操作失败：{message}"));}
                else {events.error(format!("错误：{message}"));}
            },
            Update::ConnectionError(message)=>{state.revise(|s|{s.connecting=false;s.fingerprint=None;s.secret.clear();});events.error(format!("连接失败：{message}"));},
            Update::Metrics(result)=>observability::metrics_event(&events,response.serial,result),
            Update::LogCapabilities(result)=>observability::logs_capabilities_event(&events,response.serial,result),
            Update::SystemLogs(result)=>observability::logs_event(&events,response.serial,result),
            Update::PackageEnvironment(result)=>software::event(&events,response.serial,software::Event::Environment(result)),
            Update::PackageList(result)=>software::event(&events,response.serial,software::Event::Packages(result)),
            Update::PackagePreview(result)=>software::event(&events,response.serial,software::Event::Preview(result)),
            Update::PackageStarted=>software::event(&events,response.serial,software::Event::Started),
            Update::PackageOutput(text)=>software::event(&events,response.serial,software::Event::Output(text)),
            Update::PackageFinished(result)=>{software::event(&events,response.serial,software::Event::Finished(result));events.schedule();},
            Update::Imported(report)=>{
                let warning_count=report.warnings.len();
                for warning in &report.warnings {events.log(format!("SSH 配置导入：{warning}"));}
                let result=state.with(|s|{let (config,imported,skipped)=import_profiles(&s.config,report.servers)?;anyhow::ensure!(events.writable,"配置读取失败，禁止覆盖");events.store.save(&config)?;Ok::<_,anyhow::Error>((config,imported,skipped))});
                state.revise(|s|s.import_pending=None);
                match result {
                    Ok((config,imported,skipped))=>{
                        let summary=format!("导入完成：新增 {imported} 台，跳过 {skipped} 台重复站点；{warning_count} 条提示");
                        state.revise(|s|{s.config=config;s.dialog=None;s.dialog_error.clear();s.notice=if warning_count==0{String::new()}else{format!("{summary}。{}",report.warnings[0])};});events.log(summary);
                    },
                    Err(e)=>{state.revise(|s|s.dialog_error=format!("导入失败：{e:#}"));events.log(format!("导入失败：{e:#}"));}
                }
            }
        }
    }})?;
    window.mount(
        when(
            state.map(|s| s.config.settings.language == Language::English),
            {
                let ui = ui.clone();
                move || app(ui.clone())
            },
        )
        .otherwise({
            let ui = ui.clone();
            move || app(ui.clone())
        }),
    )?;
    Shell::new().run(
        window,
        Config {
            #[cfg(windows)]
            native: Some(Box::new(app_icon::AppIcon)),
            initialization_script: Some(
                [
                    include_str!("../assets/system-theme.js"),
                    include_str!("../assets/file-list-events.js"),
                    include_str!("../assets/server-tree-events.js"),
                    include_str!("../assets/vendor/xterm/xterm.js"),
                    include_str!("../assets/vendor/addon-fit/addon-fit.js"),
                    include_str!("../assets/terminal.js"),
                ]
                .join("\n;\n"),
            ),
            title: i18n::translate("Arcnix — 服务器管理工作台"),
            width: 1440.,
            height: 980.,
            css: format!(
                "{}\n{}\n{}",
                include_str!("style.css"),
                icons::css(),
                include_str!("../assets/vendor/xterm/xterm.css")
            ),
            ..Default::default()
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_preserves_alternate_identities_and_reuses_group_case() {
        let mut existing = AppConfig::default();
        existing.add_group("ssh config").unwrap();
        let first = ServerProfile {
            name: "first".into(),
            host: "localhost".into(),
            username: "user".into(),
            ..Default::default()
        };
        let mut alternate = first.clone();
        alternate.id = new_id();
        alternate.name = "alternate".into();
        alternate.auth = AuthMethod::PrivateKey;
        alternate.private_key = Some("key.pem".into());
        let (config, added, skipped) =
            import_profiles(&existing, vec![first.clone(), first, alternate]).unwrap();
        assert_eq!((added, skipped), (2, 1));
        assert_eq!(config.groups.len(), 1);
        assert_eq!(config.servers.len(), 2);
    }
    #[test]
    fn failed_import_leaves_original_untouched() {
        let existing = AppConfig::default();
        let good = ServerProfile {
            name: "valid".into(),
            host: "localhost".into(),
            username: "user".into(),
            ..Default::default()
        };
        assert!(import_profiles(&existing, vec![good, ServerProfile::default()]).is_err());
        assert!(existing.servers.is_empty());
        assert!(existing.groups.is_empty());
    }
    #[test]
    fn directory_response_cannot_restore_invalidated_draft() {
        assert!(!accepts_listing(0, 42));
        assert!(!accepts_listing(43, 42));
        assert!(accepts_listing(43, 43));
    }
    #[test]
    fn old_server_responses_cannot_update_new_session() {
        assert!(!accepts_response(3, 2));
        assert!(accepts_response(3, 3));
        assert!(!accepts_response(3, 4));
    }
    #[test]
    fn late_cancel_cannot_hide_successful_transfer() {
        assert_eq!(finished_status(Ok(()), true), TransferStatus::Completed);
        assert_eq!(
            finished_status(Err("cancelled".into()), true),
            TransferStatus::Cancelled
        );
        assert_eq!(
            finished_status(Err("denied".into()), false),
            TransferStatus::Failed("denied".into())
        );
    }
    #[test]
    fn remote_paths_use_posix_separators() {
        assert_eq!(
            remote_join("/var/log/", "arcnix.log"),
            "/var/log/arcnix.log"
        );
        assert_eq!(remote_join("/", "中文.txt"), "/中文.txt");
    }
    #[test]
    fn downloaded_names_cannot_escape_local_directory() {
        for name in [
            "../escape",
            "..",
            "C:escape",
            "C:\\escape",
            "a/b",
            "a\\b",
            "",
        ] {
            assert!(!safe_local_name(name), "{name}");
        }
        assert!(safe_local_name("中文 file.txt"));
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_special_names_rejected() {
        for name in [
            "CON",
            "nul.txt",
            "COM1.log",
            "LPT3",
            "file.",
            "file ",
            "stream:secret",
        ] {
            assert!(!safe_local_name(name), "{name}");
        }
    }
}
