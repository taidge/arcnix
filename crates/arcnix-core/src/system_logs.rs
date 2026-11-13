//! Bounded, read-only log snapshots. No arbitrary paths, shell expressions or sudo.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

use crate::remote::RemoteSession;

pub const MAX_LINES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSource {
    Journal,
    Syslog,
    Messages,
    Auth,
    Kernel,
    AptHistory,
}
impl LogSource {
    pub const ALL: [Self; 6] = [
        Self::Journal,
        Self::Syslog,
        Self::Messages,
        Self::Auth,
        Self::Kernel,
        Self::AptHistory,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Journal => "系统日志（journalctl）",
            Self::Syslog => "系统日志 /var/log/syslog",
            Self::Messages => "系统日志 /var/log/messages",
            Self::Auth => "认证日志 /var/log/auth.log",
            Self::Kernel => "内核日志 /var/log/kern.log",
            Self::AptHistory => "APT 安装历史",
        }
    }
    pub fn path(self) -> Option<&'static str> {
        match self {
            Self::Journal => None,
            Self::Syslog => Some("/var/log/syslog"),
            Self::Messages => Some("/var/log/messages"),
            Self::Auth => Some("/var/log/auth.log"),
            Self::Kernel => Some("/var/log/kern.log"),
            Self::AptHistory => Some("/var/log/apt/history.log"),
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::Journal => "journal",
            Self::Syslog => "syslog",
            Self::Messages => "messages",
            Self::Auth => "auth",
            Self::Kernel => "kernel",
            Self::AptHistory => "apt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalPeriod {
    RecentHour,
    Today,
    Boot,
}
impl JournalPeriod {
    pub fn label(self) -> &'static str {
        match self {
            Self::RecentHour => "最近一小时",
            Self::Today => "今天",
            Self::Boot => "本次启动",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogQuery {
    pub source: LogSource,
    pub unit: String,
    /// Maximum severity number; includes all more urgent levels (0..=priority).
    pub priority: Option<u8>,
    pub period: JournalPeriod,
    pub limit: usize,
}
impl Default for LogQuery {
    fn default() -> Self {
        Self {
            source: LogSource::Journal,
            unit: String::new(),
            priority: None,
            period: JournalPeriod::RecentHour,
            limit: 200,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp: Option<String>,
    pub unit: Option<String>,
    pub priority: Option<u8>,
    pub message: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogCapabilities {
    pub sources: Vec<LogSource>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSnapshot {
    pub source: LogSource,
    pub entries: Vec<LogEntry>,
    pub warnings: Vec<String>,
}

pub fn discover_command() -> String {
    let mut command = String::from(
        "# arcnix:logs:discover\nprintf 'ARCNIX_LOGS_V1\\n'; if command -v journalctl >/dev/null 2>&1; then printf 'journal\\n'; fi;",
    );
    for source in LogSource::ALL {
        if let Some(path) = source.path() {
            command.push_str(&format!(" if [ -f '{path}' ]; then if [ -r '{path}' ]; then printf '{}\\n'; else printf 'denied\\t{}\\n'; fi; fi;", source.id(), source.id()));
        }
    }
    command
}
pub fn discover(session: &RemoteSession) -> Result<LogCapabilities> {
    let output = session.exec(&discover_command())?;
    ensure!(
        output.exit_code == 0,
        "日志来源检测失败：{}",
        output.stderr.trim()
    );
    let mut caps = parse_capabilities(&output.stdout)?;
    if !output.stderr.trim().is_empty() {
        caps.warnings.push(output.stderr.trim().to_owned());
    }
    Ok(caps)
}
pub fn parse_capabilities(text: &str) -> Result<LogCapabilities> {
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some("ARCNIX_LOGS_V1"),
        "日志来源检测输出不完整"
    );
    let mut caps = LogCapabilities::default();
    for line in lines {
        let (denied, id) = line
            .strip_prefix("denied\t")
            .map_or((false, line), |id| (true, id));
        if let Some(source) = LogSource::ALL.into_iter().find(|source| source.id() == id) {
            if denied {
                caps.warnings
                    .push(format!("当前 SSH 用户无权读取 {}", source.label()));
            } else if !caps.sources.contains(&source) {
                caps.sources.push(source);
            }
        }
    }
    if caps.sources.is_empty() {
        caps.warnings
            .push("未发现当前用户可读取的系统日志来源".to_owned());
    }
    Ok(caps)
}

pub fn read_command(query: &LogQuery) -> Result<String> {
    ensure!(
        (1..=MAX_LINES).contains(&query.limit),
        "日志行数应为 1–{MAX_LINES}"
    );
    ensure!(
        query.priority.is_none_or(|priority| priority <= 7),
        "日志优先级应为 0–7"
    );
    if let Some(path) = query.source.path() {
        return Ok(format!(
            "# arcnix:logs:read\nLC_ALL=C tail -n {} -- '{path}'",
            query.limit
        ));
    }
    let unit = query.unit.trim();
    ensure!(
        unit.len() <= 256
            && (unit.is_empty()
                || (unit.as_bytes()[0].is_ascii_alphanumeric()
                    && unit
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"@._:+-".contains(&b)))),
        "服务名称仅支持字母、数字和 @ . _ : + -，且需以字母或数字开头"
    );
    // JSON mode implies --quiet in journalctl, which hides privilege warnings.
    // A zero-entry text probe retains those warnings on stderr without reading
    // messages or mixing non-JSON text into stdout.
    let mut command = format!(
        "# arcnix:logs:read\nLC_ALL=C journalctl --no-pager -n 0 >/dev/null; LC_ALL=C journalctl --no-pager --output=json --output-fields=__REALTIME_TIMESTAMP,_SYSTEMD_UNIT,SYSLOG_IDENTIFIER,PRIORITY,MESSAGE -n {}",
        query.limit
    );
    command.push_str(match query.period {
        JournalPeriod::RecentHour => " --since='1 hour ago'",
        JournalPeriod::Today => " --since=today",
        JournalPeriod::Boot => " --boot=0",
    });
    if !unit.is_empty() {
        command.push_str(&format!(" --unit='{unit}'"));
    }
    if let Some(priority) = query.priority {
        command.push_str(&format!(" --priority={priority}"));
    }
    Ok(command)
}
pub fn read(session: &RemoteSession, query: &LogQuery) -> Result<LogSnapshot> {
    let output = session.exec(&read_command(query)?)?;
    ensure!(
        output.exit_code == 0,
        "无法读取 {}（退出码 {}）：{}{}",
        query.source.label(),
        output.exit_code,
        output.stderr.trim(),
        output.stdout.trim()
    );
    let mut snapshot = parse_snapshot(query, &output.stdout)?;
    if !output.stderr.trim().is_empty() {
        snapshot.warnings.push(output.stderr.trim().to_owned());
    }
    Ok(snapshot)
}

pub fn parse_snapshot(query: &LogQuery, text: &str) -> Result<LogSnapshot> {
    ensure!((1..=MAX_LINES).contains(&query.limit), "日志行数超出限制");
    let mut snapshot = LogSnapshot {
        source: query.source,
        entries: Vec::new(),
        warnings: Vec::new(),
    };
    // Keep the newest requested lines, including when an older/foreign server returns more.
    let lines: Vec<_> = text.lines().collect();
    for line in lines.iter().skip(lines.len().saturating_sub(query.limit)) {
        if query.source != LogSource::Journal {
            snapshot.entries.push(LogEntry {
                timestamp: None,
                unit: None,
                priority: None,
                message: (*line).to_owned(),
            });
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .context("journalctl 返回了无效 JSON，未将解析失败当作空日志")?;
        ensure!(value.is_object(), "journalctl 日志记录应为 JSON 对象");
        let timestamp = scalar(&value["__REALTIME_TIMESTAMP"])
            .and_then(|t| t.parse::<u64>().ok())
            .and_then(format_timestamp);
        let unit = scalar(&value["_SYSTEMD_UNIT"]).or_else(|| scalar(&value["SYSLOG_IDENTIFIER"]));
        let priority = scalar(&value["PRIORITY"])
            .and_then(|p| p.parse::<u8>().ok())
            .filter(|p| *p <= 7);
        let message = match &value["MESSAGE"] {
            Value::String(message) => message.clone(),
            Value::Array(parts)
                if parts
                    .iter()
                    .all(|part| part.as_u64().is_some_and(|n| n <= 255)) =>
            {
                let bytes: Vec<u8> = parts
                    .iter()
                    .filter_map(|part| part.as_u64().map(|n| n as u8))
                    .collect();
                String::from_utf8_lossy(&bytes).into_owned()
            }
            Value::Array(parts) => parts
                .iter()
                .map(|part| scalar(part).unwrap_or_else(|| "[二进制内容]".to_owned()))
                .collect::<Vec<_>>()
                .join("\n"),
            Value::Null => {
                snapshot
                    .warnings
                    .push("部分消息内容被 journalctl 省略或为二进制，未显示完整内容".to_owned());
                "[消息内容不可用]".to_owned()
            }
            _ => bail!("journalctl 返回了不支持的消息格式"),
        };
        snapshot.entries.push(LogEntry {
            timestamp,
            unit,
            priority,
            message,
        });
    }
    snapshot.warnings.dedup();
    Ok(snapshot)
}
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// Gregorian civil date from Unix days; no host locale/time-zone dependency.
fn format_timestamp(micros: u64) -> Option<String> {
    let seconds = micros / 1_000_000;
    if seconds > 253_402_300_799 {
        return None;
    }
    let days = (seconds / 86400) as i64;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let rem = seconds % 86400;
    Some(format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03} UTC",
        rem / 3600,
        rem / 60 % 60,
        rem % 60,
        micros % 1_000_000 / 1000
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_shell_and_option_injection() {
        for unit in [
            "; rm -rf /",
            "$(id)",
            "ssh.service'",
            "--help",
            "a\nb",
            "*.service",
            "x/y",
            "a\\b",
        ] {
            let query = LogQuery {
                unit: unit.to_owned(),
                ..Default::default()
            };
            assert!(read_command(&query).is_err(), "{unit}");
        }
        let mut query = LogQuery {
            unit: "nginx@worker-1.service".to_owned(),
            priority: Some(4),
            period: JournalPeriod::Boot,
            ..Default::default()
        };
        let cmd = read_command(&query).unwrap();
        assert!(cmd.contains("--unit='nginx@worker-1.service' --priority=4"));
        assert!(cmd.contains("--boot=0"));
        query.limit = 501;
        assert!(read_command(&query).is_err());
        query.limit = 0;
        assert!(read_command(&query).is_err());
        query.limit = 200;
        query.priority = Some(8);
        assert!(read_command(&query).is_err());
    }
    #[test]
    fn capabilities_preserve_permission_warnings() {
        let caps =
            parse_capabilities("ARCNIX_LOGS_V1\njournal\nsyslog\nsyslog\ndenied\tauth\nunknown\n")
                .unwrap();
        assert_eq!(caps.sources, vec![LogSource::Journal, LogSource::Syslog]);
        assert_eq!(caps.warnings.len(), 1);
        assert!(parse_capabilities("oops").is_err());
        assert!(
            !parse_capabilities("ARCNIX_LOGS_V1\n")
                .unwrap()
                .warnings
                .is_empty()
        );
    }
    #[test]
    fn parses_journal_timestamps_binary_and_omitted_messages() {
        let snapshot=parse_snapshot(&LogQuery::default(), concat!(
            "{\"__REALTIME_TIMESTAMP\":\"0\",\"_SYSTEMD_UNIT\":\"ssh.service\",\"PRIORITY\":\"6\",\"MESSAGE\":\"hello\\nworld\"}\n",
            "{\"__REALTIME_TIMESTAMP\":\"1709164800123456\",\"MESSAGE\":[228,189,160,229,165,189],\"PRIORITY\":3}\n",
            "{\"MESSAGE\":null}\n")).unwrap();
        assert_eq!(
            snapshot.entries[0].timestamp.as_deref(),
            Some("1970-01-01 00:00:00.000 UTC")
        );
        assert_eq!(
            snapshot.entries[1].timestamp.as_deref(),
            Some("2024-02-29 00:00:00.123 UTC")
        );
        assert_eq!(snapshot.entries[1].message, "你好");
        assert_eq!(snapshot.entries[1].priority, Some(3));
        assert_eq!(snapshot.warnings.len(), 1);
        assert!(parse_snapshot(&LogQuery::default(), "not json").is_err());
        assert!(parse_snapshot(&LogQuery::default(), "[]").is_err());
    }
    #[test]
    fn file_sources_are_fixed_and_limit_newest_lines() {
        let query = LogQuery {
            source: LogSource::Auth,
            limit: 2,
            ..Default::default()
        };
        assert_eq!(
            read_command(&query).unwrap(),
            "# arcnix:logs:read\nLC_ALL=C tail -n 2 -- '/var/log/auth.log'"
        );
        let snapshot = parse_snapshot(&query, "old\nsecond\nlast\n").unwrap();
        assert_eq!(
            snapshot
                .entries
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>(),
            vec!["second", "last"]
        );
        assert!(snapshot.entries[0].timestamp.is_none());
    }
}
