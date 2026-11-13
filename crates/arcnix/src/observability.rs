use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arcnix_core::metrics::{self, ResourceSnapshot, ResourceUsage};
use arcnix_core::system_logs::{JournalPeriod, LogCapabilities, LogQuery, LogSnapshot, LogSource};

use super::*;

#[derive(Clone, Default)]
pub(super) struct MetricState {
    pub pending: Option<u64>,
    previous: Option<ResourceSnapshot>,
    pub usage: Option<ResourceUsage>,
    pub error: String,
    completed: Option<Instant>,
    updated: String,
}
#[derive(Clone, Default)]
pub(super) struct LogsState {
    pub pending: Option<u64>,
    discovering: bool,
    pub capabilities: Option<LogCapabilities>,
    pub query: LogQuery,
    pub snapshot: Option<LogSnapshot>,
    pub filter: String,
    pub auto_refresh: bool,
    pub error: String,
    completed: Option<Instant>,
    updated: String,
}
fn clock_label() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        % 86400;
    format!(
        "{:02}:{:02}:{:02} UTC",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
fn due(last: Option<Instant>, seconds: u64) -> bool {
    last.is_none_or(|at| at.elapsed() >= Duration::from_secs(seconds))
}
impl Ui {
    pub(super) fn metrics_refresh(&self) {
        if !self
            .state
            .with(|s| s.connected && !s.system_task_running && s.metrics.pending.is_none())
        {
            return;
        }
        let serial = self.send(Action::MetricsSample);
        self.state.revise(|s| {
            s.metrics.pending = Some(serial);
            s.metrics.error.clear();
        });
    }
    pub(super) fn logs_discover(&self) {
        if !self.state.with(log_ready) {
            return;
        }
        let serial = self.send(Action::LogsDiscover);
        self.state.revise(|s| {
            s.system_logs.pending = Some(serial);
            s.system_logs.discovering = true;
            s.system_logs.error.clear();
        });
    }
    pub(super) fn logs_refresh(&self) {
        if !self.state.with(log_ready) {
            return;
        }
        let query = self.state.with(|s| s.system_logs.query.clone());
        let serial = self.send(Action::LogsRead(query));
        self.state.revise(|s| {
            s.system_logs.pending = Some(serial);
            s.system_logs.discovering = false;
            s.system_logs.error.clear();
        });
    }
    pub(super) fn observability_enter(&self) {
        if !self.state.with(navigation::workspace_visible) {
            return;
        }
        match self.state.with(|s| s.workspace_page) {
            WorkspacePage::Resources => self.metrics_refresh(),
            WorkspacePage::SystemLogs => {
                if self.state.with(|s| s.system_logs.capabilities.is_none()) {
                    self.logs_discover();
                } else {
                    self.logs_refresh();
                }
            }
            _ => {}
        }
    }
    pub(super) fn observability_tick(&self) {
        if !self
            .state
            .with(|s| navigation::workspace_visible(s) && !s.system_task_running)
        {
            return;
        }
        match self.state.with(|s| s.workspace_page) {
            WorkspacePage::Resources
                if self.state.with(|s| {
                    s.metrics.pending.is_none()
                        && due(
                            s.metrics.completed,
                            s.config.settings.resource_interval_seconds,
                        )
                }) =>
            {
                self.metrics_refresh()
            }
            WorkspacePage::SystemLogs
                if self.state.with(|s| {
                    s.system_logs.auto_refresh
                        && s.system_logs.pending.is_none()
                        && s.system_logs.capabilities.is_some()
                        && due(s.system_logs.completed, 5)
                }) =>
            {
                self.logs_refresh()
            }
            _ => {}
        }
    }
}
pub(super) fn metrics_event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<ResourceSnapshot, String>,
) {
    if !ui.state.with(|s| s.metrics.pending == Some(serial)) {
        return;
    }
    ui.state.revise(|s| {
        let m = &mut s.metrics;
        m.pending = None;
        m.completed = Some(Instant::now());
        match result {
            Ok(snapshot) => {
                m.usage = Some(metrics::usage(&snapshot, m.previous.as_ref()));
                m.previous = Some(snapshot);
                m.updated = clock_label();
                m.error.clear();
            }
            Err(error) => {
                m.error = error;
                m.previous = None;
            }
        }
    });
}
pub(super) fn logs_capabilities_event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<LogCapabilities, String>,
) {
    if !ui
        .state
        .with(|s| s.system_logs.pending == Some(serial) && s.system_logs.discovering)
    {
        return;
    }
    let success = result.as_ref().is_ok_and(|c| !c.sources.is_empty());
    ui.state.revise(|s| {
        let l = &mut s.system_logs;
        l.pending = None;
        l.discovering = false;
        l.completed = Some(Instant::now());
        match result {
            Ok(capabilities) => {
                if !capabilities.sources.contains(&l.query.source)
                    && let Some(source) = capabilities.sources.first()
                {
                    l.query.source = *source;
                    l.snapshot = None;
                    l.updated.clear();
                }
                if capabilities.sources.is_empty() {
                    l.error =
                        "没有可读取的日志源。请检查服务器日志服务和当前账号的读取权限。".into();
                }
                l.capabilities = Some(capabilities);
            }
            Err(error) => l.error = error,
        }
    });
    if success
        && ui
            .state
            .with(|s| s.workspace_page == WorkspacePage::SystemLogs)
    {
        ui.logs_refresh();
    }
}
pub(super) fn logs_event(ui: &Ui, serial: u64, result: std::result::Result<LogSnapshot, String>) {
    if !ui
        .state
        .with(|s| s.system_logs.pending == Some(serial) && !s.system_logs.discovering)
    {
        return;
    }
    ui.state.revise(|s| {
        let l = &mut s.system_logs;
        l.pending = None;
        l.completed = Some(Instant::now());
        match result {
            Ok(snapshot) => {
                l.snapshot = Some(snapshot);
                l.updated = clock_label();
                l.error.clear();
            }
            Err(error) => l.error = error,
        }
    });
}
fn log_ready(s: &State) -> bool {
    s.connected && !s.system_task_running && s.system_logs.pending.is_none()
}
fn bytes(value: u64) -> String {
    if value >= 1024 * 1024 * 1024 {
        format!("{:.2} GiB", value as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if value >= 1024 * 1024 {
        format!("{:.1} MiB", value as f64 / (1024.0 * 1024.0))
    } else if value >= 1024 {
        format!("{:.1} KiB", value as f64 / 1024.0)
    } else {
        format!("{value} B")
    }
}
fn rate(value: Option<f64>) -> String {
    value
        .map(|n| format!("{}/s", bytes(n.max(0.0) as u64)))
        .unwrap_or_else(|| i18n::translate("等待下一次采样"))
}
fn missing() -> String {
    "—".into()
}
fn metric_card(
    title: &'static str,
    value: impl Into<Vary<String>> + 'static,
    detail: impl Into<Vary<String>> + 'static,
) -> impl Widget {
    col().class("metric-card").fill((
        text().class("subtle").content(i18n::bind(title)),
        text().class("metric-value").content(i18n::bind(value)),
        text().class("subtle").content(i18n::bind(detail)),
    ))
}

fn update_query(ui: &Ui, change: impl FnOnce(&mut LogQuery)) {
    if !ui.state.with(log_ready) {
        return;
    }
    ui.state.revise(|s| {
        change(&mut s.system_logs.query);
        s.system_logs.snapshot = None;
        s.system_logs.updated.clear();
    });
    ui.logs_refresh();
}
fn priority_label(priority: Option<u8>) -> &'static str {
    match priority {
        Some(3) => "错误及以上",
        Some(4) => "警告及以上",
        _ => "全部级别",
    }
}
fn period_choice(ui: Ui, period: JournalPeriod, label: &'static str) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(state.map(move |s| {
            if s.system_logs.query.period == period {
                "button compact choice active"
            } else {
                "button compact choice"
            }
            .to_string()
        }))
        .name(i18n::bind(format!("日志时间范围：{label}")))
        .label(i18n::bind(label))
        .enabled(state.map(log_ready))
        .on_press(move || update_query(&ui, |q| q.period = period))
}
fn priority_choice(ui: Ui, priority: Option<u8>) -> impl Widget {
    let state = ui.state;
    let label = priority_label(priority);
    sapid::button()
        .class(state.map(move |s| {
            if s.system_logs.query.priority == priority {
                "button compact choice active"
            } else {
                "button compact choice"
            }
            .to_string()
        }))
        .name(i18n::bind(format!("日志严重级别：{label}")))
        .label(i18n::bind(label))
        .enabled(state.map(log_ready))
        .on_press(move || update_query(&ui, |q| q.priority = priority))
}
fn filtered_logs(s: &State) -> String {
    let l = &s.system_logs;
    let Some(snapshot) = &l.snapshot else {
        return String::new();
    };
    let needle = l.filter.to_lowercase();
    let lines: Vec<String> = snapshot
        .entries
        .iter()
        .filter(|e| {
            needle.is_empty()
                || format!(
                    "{} {} {}",
                    e.timestamp.as_deref().unwrap_or(""),
                    e.unit.as_deref().unwrap_or(""),
                    e.message
                )
                .to_lowercase()
                .contains(&needle)
        })
        .map(|e| {
            let mut prefix = Vec::new();
            if let Some(t) = &e.timestamp {
                prefix.push(t.clone());
            }
            if let Some(p) = e.priority {
                prefix.push(format!(
                    "[{}]",
                    i18n::translate(match p {
                        0 => "紧急",
                        1 => "警报",
                        2 => "严重",
                        3 => "错误",
                        4 => "警告",
                        5 => "通知",
                        6 => "信息",
                        _ => "调试",
                    })
                ));
            }
            if let Some(u) = &e.unit {
                prefix.push(u.clone());
            }
            if prefix.is_empty() {
                e.message.clone()
            } else {
                format!("{}  {}", prefix.join(" "), e.message)
            }
        })
        .collect();
    if lines.is_empty() {
        i18n::translate(if needle.is_empty() {
            "当前条件下没有日志记录。"
        } else {
            "没有匹配此筛选的记录。"
        })
    } else {
        lines.join("\n")
    }
}
pub(super) fn logs(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    let discover = ui.clone();
    let source_ui = ui.clone();
    let query_ui = ui.clone();
    let period_ui = ui.clone();
    let priority_ui = ui.clone();
    let auto_ui = ui.clone();
    let commit_ui = ui.clone();
    col().class("observability-page system-logs-page").fill((
        row().class("observability-heading").fill((
            col().fill((
                text().class("page-title").content(i18n::bind("系统日志")),
                text().class("subtle").content(i18n::bind(
                    "读取最近 200 条记录；仅使用当前账号的读取权限。",
                )),
            )),
            row().class("toolbar-actions").fill((
                icon_action("重新检测日志源", Icon::Refresh)
                    .enabled(state.map(log_ready))
                    .on_press(move || discover.logs_discover()),
                icon_action("刷新系统日志", Icon::Refresh)
                    .enabled(state.map(log_ready))
                    .on_press(move || refresh.logs_refresh()),
            )),
        )),
        when(state.map(|s| !s.connected), || {
            text()
                .class("software-empty")
                .content(i18n::bind("连接服务器后查看系统日志。"))
        }),
        when(state.map(|s| s.connected), move || {
            let source_ui = source_ui.clone();
            let query_ui = query_ui.clone();
            let period_ui = period_ui.clone();
            let priority_ui = priority_ui.clone();
            let auto_ui = auto_ui.clone();
            let commit_ui = commit_ui.clone();
            col().class("observability-content").fill((
                row().class("logs-sources").fill(
                    each(state.map(|s| {
                        s.system_logs
                            .capabilities
                            .as_ref()
                            .map(|c| c.sources.clone())
                            .unwrap_or_default()
                    }))
                    .key(|source| source.label().to_string())
                    .children(move |item| {
                        let switch = source_ui.clone();
                        let source = item.value;
                        sapid::button()
                            .class(state.map(move |s| {
                                if s.system_logs.query.source == source.get() {
                                    "button active"
                                } else {
                                    "button"
                                }
                                .to_string()
                            }))
                            .label(i18n::bind(source.map(|v| v.label().to_string())))
                            .enabled(state.map(log_ready))
                            .on_press(move || update_query(&switch, |q| q.source = source.get()))
                    }),
                ),
                when(
                    state.map(|s| s.system_logs.query.source == LogSource::Journal),
                    move || {
                        let query_ui = query_ui.clone();
                        let period_ui = period_ui.clone();
                        let priority_ui = priority_ui.clone();
                        let commit_ui = commit_ui.clone();
                        row().class("logs-query").fill((
                            col().class("field logs-unit").fill((
                                text()
                                    .class("field-label")
                                    .content(i18n::bind("服务单元（留空查看全部）")),
                                input()
                                    .name(i18n::bind("日志服务单元"))
                                    .value(state.map(|s| s.system_logs.query.unit.clone()))
                                    .enabled(state.map(log_ready))
                                    .on(value_changed, move |e| {
                                        if let glory::ui::Event::ValueChanged { value, .. } = e {
                                            state.revise(|s| {
                                                s.system_logs.query.unit = value;
                                                s.system_logs.auto_refresh = false;
                                                s.system_logs.snapshot = None;
                                                s.system_logs.updated.clear();
                                            });
                                        }
                                    })
                                    .on(commit, move |_| commit_ui.logs_refresh()),
                            )),
                            col().class("field").fill((
                                text().class("field-label").content(i18n::bind("时间范围")),
                                row().class("logs-options").fill((
                                    period_choice(
                                        period_ui.clone(),
                                        JournalPeriod::RecentHour,
                                        "最近一小时",
                                    ),
                                    period_choice(period_ui.clone(), JournalPeriod::Today, "今天"),
                                    period_choice(
                                        period_ui.clone(),
                                        JournalPeriod::Boot,
                                        "本次启动",
                                    ),
                                )),
                            )),
                            col().class("field").fill((
                                text().class("field-label").content(i18n::bind("严重级别")),
                                row().class("logs-options").fill((
                                    priority_choice(priority_ui.clone(), None),
                                    priority_choice(priority_ui.clone(), Some(4)),
                                    priority_choice(priority_ui.clone(), Some(3)),
                                )),
                            )),
                            sapid::button()
                                .class("button")
                                .label(i18n::bind("查询日志"))
                                .enabled(state.map(log_ready))
                                .on_press(move || query_ui.logs_refresh()),
                        ))
                    },
                ),
                row().class("logs-toolbar").fill((
                    input()
                        .name(i18n::bind("筛选当前日志"))
                        .semantics(Some(Box::new(Semantics {
                            placeholder: Some(i18n::translate("筛选当前结果…")),
                            ..Default::default()
                        })))
                        .class("logs-filter")
                        .value(state.map(|s| s.system_logs.filter.clone()))
                        .on(value_changed, move |e| {
                            if let glory::ui::Event::ValueChanged { value, .. } = e {
                                state.revise(|s| s.system_logs.filter = value);
                            }
                        }),
                    sapid::button()
                        .class(state.map(|s| {
                            if s.system_logs.auto_refresh {
                                "button active"
                            } else {
                                "button"
                            }
                            .to_string()
                        }))
                        .name(i18n::bind("切换日志自动刷新"))
                        .label(i18n::bind(state.map(|s| {
                            if s.system_logs.auto_refresh {
                                "自动刷新：开 · 5 秒"
                            } else {
                                "自动刷新：关"
                            }
                            .to_string()
                        })))
                        .enabled(state.map(log_ready))
                        .on_press(move || {
                            auto_ui.state.revise(|s| {
                                s.system_logs.auto_refresh = !s.system_logs.auto_refresh
                            })
                        }),
                )),
                text()
                    .class("observability-status")
                    .content(i18n::bind(state.map(|s| {
                        let l = &s.system_logs;
                        if s.system_task_running {
                            "系统任务执行中，日志刷新已暂停".into()
                        } else if l.discovering {
                            "正在检测可用日志源…".into()
                        } else if l.pending.is_some() {
                            "正在读取日志…".into()
                        } else if l.updated.is_empty() {
                            "等待查询".into()
                        } else {
                            format!(
                                "{} 条记录 · 最近读取 {}{}",
                                l.snapshot.as_ref().map(|r| r.entries.len()).unwrap_or(0),
                                l.updated,
                                if l.error.is_empty() {
                                    ""
                                } else {
                                    " · 显示上次成功结果"
                                }
                            )
                        }
                    }))),
                text()
                    .class(state.map(|s| {
                        if s.system_logs.error.is_empty() {
                            "hidden"
                        } else {
                            "software-error"
                        }
                        .to_string()
                    }))
                    .content(i18n::bind(state.map(|s| s.system_logs.error.clone()))),
                text()
                    .class("observability-warning")
                    .content(i18n::bind(state.map(|s| {
                        let mut w = s
                            .system_logs
                            .capabilities
                            .as_ref()
                            .map(|c| c.warnings.clone())
                            .unwrap_or_default();
                        if let Some(r) = &s.system_logs.snapshot {
                            w.extend(r.warnings.clone());
                        }
                        w.join("\n")
                    }))),
                text()
                    .class("system-log-output")
                    .selectable(true)
                    .content(state.map(filtered_logs)),
            ))
        }),
    ))
}
pub(super) fn resources(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    col().class("observability-page resources-page").fill((
        row().class("observability-heading").fill((
            col().fill((
                text().class("page-title").content(i18n::bind("资源概览")),
                text().class("subtle").content(i18n::bind(state.map(|s| {
                    format!(
                        "每 {} 秒读取一次数值，仅在此页面自动刷新。",
                        s.config.settings.resource_interval_seconds
                    )
                }))),
            )),
            icon_action("刷新资源数值", Icon::Refresh)
                .enabled(
                    state.map(|s| {
                        s.connected && !s.system_task_running && s.metrics.pending.is_none()
                    }),
                )
                .on_press(move || refresh.metrics_refresh()),
        )),
        when(state.map(|s| !s.connected), || {
            text()
                .class("software-empty")
                .content(i18n::bind("连接服务器后查看 CPU、内存、磁盘和网络。"))
        }),
        when(state.map(|s| s.connected), move || {
            col().class("observability-content").fill((
                text()
                    .class("observability-status")
                    .content(i18n::bind(state.map(|s| {
                        if s.system_task_running {
                            "系统任务执行中，自动采样已暂停".into()
                        } else if s.metrics.pending.is_some() {
                            "正在采样…".into()
                        } else if s.metrics.updated.is_empty() {
                            "等待采样".into()
                        } else {
                            format!(
                                "最近采样 {}{}",
                                s.metrics.updated,
                                if s.metrics.error.is_empty() {
                                    ""
                                } else {
                                    " · 显示上次成功数值"
                                }
                            )
                        }
                    }))),
                text()
                    .class(state.map(|s| {
                        if s.metrics.error.is_empty() {
                            "hidden"
                        } else {
                            "software-error"
                        }
                        .to_string()
                    }))
                    .content(i18n::bind(state.map(|s| s.metrics.error.clone()))),
                row().class("metric-grid").fill((
                    metric_card(
                        "CPU 使用率",
                        state.map(|s| {
                            s.metrics
                                .usage
                                .as_ref()
                                .and_then(|u| u.cpu_percent)
                                .map(|n| format!("{n:.1}%"))
                                .unwrap_or_else(missing)
                        }),
                        state.map(|s| {
                            s.metrics
                                .usage
                                .as_ref()
                                .and_then(|u| u.snapshot.cpu.as_ref())
                                .map(|c| {
                                    format!(
                                        "{} 逻辑处理器 · {}",
                                        c.logical_cpus,
                                        if s.metrics
                                            .usage
                                            .as_ref()
                                            .is_some_and(|u| u.cpu_percent.is_some())
                                        {
                                            "两次采样计算"
                                        } else {
                                            "等待下一次采样"
                                        }
                                    )
                                })
                                .unwrap_or_else(|| "等待 CPU 采样".into())
                        }),
                    ),
                    metric_card(
                        "内存使用",
                        state.map(|s| {
                            s.metrics
                                .usage
                                .as_ref()
                                .and_then(|u| u.snapshot.memory.as_ref())
                                .map(|m| format!("{:.1}%", m.used_percent))
                                .unwrap_or_else(missing)
                        }),
                        state.map(|s| {
                            s.metrics
                                .usage
                                .as_ref()
                                .and_then(|u| u.snapshot.memory.as_ref())
                                .map(|m| {
                                    format!(
                                        "{} / {} · 可用 {}",
                                        bytes(m.used_bytes),
                                        bytes(m.total_bytes),
                                        bytes(m.available_bytes)
                                    )
                                })
                                .unwrap_or_else(missing)
                        }),
                    ),
                    metric_card(
                        "交换空间",
                        state.map(|s| {
                            s.metrics
                                .usage
                                .as_ref()
                                .and_then(|u| u.snapshot.swap.as_ref())
                                .map(|m| {
                                    m.used_percent
                                        .map(|p| format!("{p:.1}%"))
                                        .unwrap_or_else(|| "未启用".into())
                                })
                                .unwrap_or_else(missing)
                        }),
                        state.map(|s| {
                            s.metrics
                                .usage
                                .as_ref()
                                .and_then(|u| u.snapshot.swap.as_ref())
                                .map(|m| {
                                    format!("{} / {}", bytes(m.used_bytes), bytes(m.total_bytes))
                                })
                                .unwrap_or_else(missing)
                        }),
                    ),
                    metric_card(
                        "系统负载",
                        state.map(|s| {
                            s.metrics
                                .usage
                                .as_ref()
                                .and_then(|u| u.snapshot.load_average)
                                .map(|l| format!("{:.2} / {:.2} / {:.2}", l[0], l[1], l[2]))
                                .unwrap_or_else(missing)
                        }),
                        state.map(|_| "1 / 5 / 15 分钟".into()),
                    ),
                )),
                text().class("subtle").content(i18n::bind(state.map(|s| {
                    s.metrics
                        .usage
                        .as_ref()
                        .and_then(|u| u.snapshot.uptime_seconds)
                        .map(|n| {
                            format!(
                                "运行时间 {} 天 {} 小时 {} 分钟",
                                n as u64 / 86400,
                                n as u64 / 3600 % 24,
                                n as u64 / 60 % 60
                            )
                        })
                        .unwrap_or_else(missing)
                }))),
                text()
                    .class("observability-warning")
                    .content(i18n::bind(state.map(|s| {
                        s.metrics
                            .usage
                            .as_ref()
                            .map(|u| u.snapshot.warnings.join("\n"))
                            .unwrap_or_default()
                    }))),
                text().class("pane-title").content(i18n::bind("磁盘")),
                col().class("resource-table").fill(
                    each(state.map(|s| {
                        s.metrics
                            .usage
                            .as_ref()
                            .map(|u| u.snapshot.disks.clone())
                            .unwrap_or_default()
                    }))
                    .key(|d| format!("{}:{}", d.filesystem, d.mount))
                    .children(|item| {
                        row().class("resource-row").fill((
                            text()
                                .class("resource-name")
                                .content(item.value.map(|d| d.mount.clone())),
                            text()
                                .class("resource-detail")
                                .content(i18n::bind(item.value.map(|d| {
                                    format!(
                                        "{} / {} · 可用 {}",
                                        bytes(d.used_bytes),
                                        bytes(d.total_bytes),
                                        bytes(d.available_bytes)
                                    )
                                }))),
                            text().class("resource-value").content(item.value.map(|d| {
                                d.used_percent
                                    .map(|p| format!("{p:.1}%"))
                                    .unwrap_or_else(missing)
                            })),
                        ))
                    }),
                ),
                text().class("pane-title").content(i18n::bind("网络接口")),
                text().class("subtle").content(i18n::bind(
                    "接收 / 发送速率由两次采样计算；累计数值自接口启动起计。",
                )),
                col().class("resource-table").fill(
                    each(state.map(|s| {
                        s.metrics
                            .usage
                            .as_ref()
                            .map(|u| u.networks.clone())
                            .unwrap_or_default()
                    }))
                    .key(|n| n.interface.clone())
                    .children(|item| {
                        row().class("resource-row").fill((
                            text()
                                .class("resource-name")
                                .content(item.value.map(|n| n.interface.clone())),
                            text()
                                .class("resource-detail")
                                .content(i18n::bind(item.value.map(|n| {
                                    format!(
                                        "接收 {} · 发送 {}",
                                        rate(n.receive_bytes_per_second),
                                        rate(n.transmit_bytes_per_second)
                                    )
                                }))),
                            text()
                                .class("subtle")
                                .content(i18n::bind(item.value.map(|n| {
                                    format!(
                                        "累计 ↓ {} / ↑ {}",
                                        bytes(n.received_bytes),
                                        bytes(n.transmitted_bytes)
                                    )
                                }))),
                        ))
                    }),
                ),
            ))
        }),
    ))
}
