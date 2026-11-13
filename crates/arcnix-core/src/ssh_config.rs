//! Conservative, non-executing import of OpenSSH client configuration.
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};

use crate::{AuthMethod, ServerProfile};

const MAX_BYTES: usize = 1024 * 1024;
const MAX_ALIASES: usize = 512;
const MAX_DIRECTIVES: usize = 8192;

#[derive(Debug, Default)]
pub struct ImportReport {
    pub servers: Vec<ServerProfile>,
    pub warnings: Vec<String>,
}

pub fn import_file(path: &Path) -> Result<ImportReport> {
    let mut bytes = Vec::new();
    File::open(path)
        .context("无法打开 SSH 配置文件")?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_BYTES, "SSH 配置文件不能超过 1 MiB");
    parse_config(std::str::from_utf8(&bytes).context("SSH 配置必须是 UTF-8 文本")?)
}

struct Directive {
    patterns: Arc<[String]>,
    key: String,
    values: Vec<String>,
    line: usize,
}

pub fn parse_config(text: &str) -> Result<ImportReport> {
    ensure!(text.len() <= MAX_BYTES, "SSH 配置文件不能超过 1 MiB");
    let mut report = ImportReport::default();
    let mut patterns: Arc<[String]> = vec!["*".to_owned()].into();
    let mut aliases = Vec::new();
    let mut seen = HashSet::new();
    let mut directives = Vec::new();
    let mut unsupported_global = false;
    let mut ignored = HashSet::new();
    let mut pattern_work = 0usize;
    for (index, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let number = index + 1;
        let mut words =
            tokenize(line).with_context(|| format!("SSH 配置第 {number} 行格式错误"))?;
        if words.is_empty() {
            continue;
        }
        let first = words.remove(0);
        let key = if let Some((key, value)) = first.split_once('=') {
            if !value.is_empty() {
                words.insert(0, value.to_owned());
            }
            key.to_ascii_lowercase()
        } else {
            first.to_ascii_lowercase()
        };
        if !first.contains('=') && words.first().is_some_and(|v| v.starts_with('=')) {
            let value = words.remove(0);
            if value.len() > 1 {
                words.insert(0, value[1..].to_owned());
            }
        }
        ensure!(!words.is_empty(), "SSH 配置第 {number} 行缺少选项值");
        if key == "include" || key == "match" {
            unsupported_global = true;
            report.warnings.push(format!(
                "第 {number} 行包含 {key}：无法可靠还原其条件或外部配置，已跳过整个文件的站点导入"
            ));
        }
        if key == "host" {
            ensure!(
                words.len() <= 512 && words.iter().all(|p| p.len() <= 1024),
                "Host 模式过多或过长"
            );
            patterns = words.into();
            for alias in patterns.iter() {
                if !alias.starts_with('!')
                    && !alias.contains(['*', '?'])
                    && seen.insert(alias.to_ascii_lowercase())
                {
                    ensure!(
                        aliases.len() < MAX_ALIASES,
                        "SSH 配置最多导入 {MAX_ALIASES} 个明确站点"
                    );
                    aliases.push(alias.clone());
                }
            }
        } else {
            ensure!(
                directives.len() < MAX_DIRECTIVES,
                "SSH 配置选项数量超过 {MAX_DIRECTIVES}"
            );
            pattern_work += patterns.len();
            ensure!(pattern_work <= 32768, "SSH 配置模式匹配规则过于复杂");
            if !matches!(
                key.as_str(),
                "hostname"
                    | "user"
                    | "port"
                    | "identityfile"
                    | "proxyjump"
                    | "proxycommand"
                    | "certificatefile"
                    | "identityagent"
                    | "remotecommand"
                    | "localcommand"
                    | "include"
                    | "match"
            ) {
                ignored.insert(key.clone());
            }
            directives.push(Directive {
                patterns: patterns.clone(),
                key,
                values: words,
                line: number,
            });
        }
    }
    if unsupported_global {
        return Ok(report);
    }
    let mut ignored: Vec<_> = ignored.into_iter().collect();
    ignored.sort();
    if !ignored.is_empty() {
        report.warnings.push(format!(
            "以下选项不导入，连接使用 Arcnix 的设置：{}",
            ignored.join(", ")
        ));
    }
    for alias in aliases {
        let mut values: HashMap<&str, &Directive> = HashMap::new();
        let mut identities = 0;
        for directive in &directives {
            if matches_patterns(&directive.patterns, &alias) {
                if directive.key == "identityfile" {
                    identities += 1;
                }
                values.entry(&directive.key).or_insert(directive);
            }
        }
        let unsupported = [
            "proxyjump",
            "proxycommand",
            "certificatefile",
            "identityagent",
            "remotecommand",
            "localcommand",
        ]
        .into_iter()
        .find(|key| {
            values
                .get(key)
                .is_some_and(|d| d.values.len() != 1 || !d.values[0].eq_ignore_ascii_case("none"))
        });
        if let Some(key) = unsupported {
            report.warnings.push(format!(
                "站点 {alias} 使用不支持的 {key}，已跳过；不会绕过代理直接连接"
            ));
            continue;
        }
        if identities > 1 {
            report.warnings.push(format!(
                "站点 {alias} 匹配多个 IdentityFile，仅导入第一个；可在服务器配置中修改"
            ));
        }
        let profile = build_profile(&alias, &values);
        match profile {
            Ok(profile) => report.servers.push(profile),
            Err(error) => report
                .warnings
                .push(format!("站点 {alias} 未导入：{error:#}")),
        }
    }
    if report.servers.is_empty() && report.warnings.is_empty() {
        report
            .warnings
            .push("没有找到可导入的明确 Host 别名（通配符本身不会生成服务器）".into());
    }
    Ok(report)
}

fn build_profile(alias: &str, values: &HashMap<&str, &Directive>) -> Result<ServerProfile> {
    let value = |key| -> Result<Option<&str>> {
        let Some(d) = values.get(key) else {
            return Ok(None);
        };
        ensure!(
            d.values.len() == 1,
            "第 {} 行的 {key} 需要单个值；含空格路径请使用引号",
            d.line
        );
        Ok(Some(d.values[0].as_str()))
    };
    let mut profile = ServerProfile {
        name: alias.into(),
        host: alias.into(),
        ..Default::default()
    };
    if let Some(host) = value("hostname")? {
        profile.host = expand_tokens(host, alias, alias, "", 22)?;
    }
    profile.username = value("user")?
        .map(str::to_owned)
        .or_else(|| std::env::var("USERNAME").ok())
        .or_else(|| std::env::var("USER").ok())
        .context("未指定 User，且无法确定本机用户名")?;
    if let Some(port) = value("port")? {
        profile.port = port.parse().context("Port 应为 1–65535 的数字")?;
    }
    if let Some(identity) = value("identityfile")?.filter(|v| !v.eq_ignore_ascii_case("none")) {
        let identity = expand_tokens(
            identity,
            alias,
            &profile.host,
            &profile.username,
            profile.port,
        )?;
        profile.private_key = Some(expand_home(&identity)?);
        profile.auth = AuthMethod::PrivateKey;
    }
    ensure!(
        !profile.host.contains(['$', '`', '%']) && !profile.username.contains(['$', '`', '%']),
        "不支持动态主机名或用户名"
    );
    profile.validate()?;
    Ok(profile)
}

fn expand_tokens(value: &str, alias: &str, host: &str, user: &str, port: u16) -> Result<String> {
    ensure!(!value.contains(['$', '`']), "不展开环境变量或执行命令");
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => result.push('%'),
            Some('n') => result.push_str(alias),
            Some('h') => result.push_str(host),
            Some('r') if !user.is_empty() => result.push_str(user),
            Some('p') => result.push_str(&port.to_string()),
            Some('d') => result.push_str(
                directories::BaseDirs::new()
                    .context("无法确定用户主目录")?
                    .home_dir()
                    .to_str()
                    .context("用户主目录不是 Unicode")?,
            ),
            _ => anyhow::bail!("不支持此配置中的百分号替换标记"),
        }
    }
    Ok(result)
}

fn expand_home(value: &str) -> Result<PathBuf> {
    if value == "~" || value.starts_with("~/") || value.starts_with("~\\") {
        let home = directories::BaseDirs::new().context("无法确定用户主目录")?;
        return Ok(home.home_dir().join(value.get(2..).unwrap_or("")));
    }
    ensure!(!value.starts_with('~'), "不支持其他用户的主目录替换");
    Ok(PathBuf::from(value))
}

fn matches_patterns(patterns: &[String], alias: &str) -> bool {
    let alias = alias.to_ascii_lowercase();
    let mut positive = false;
    for pattern in patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if glob(&negated.to_ascii_lowercase(), &alias) {
                return false;
            }
        } else if glob(&pattern.to_ascii_lowercase(), &alias) {
            positive = true;
        }
    }
    positive
}

fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < t.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            retry += 1;
            j = retry;
            i = s + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == b'*' {
        i += 1;
    }
    i == p.len()
}

fn tokenize(line: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && chars
                .peek()
                .is_some_and(|c| matches!(c, '"' | '\'' | '#' | ' ' | '\t'))
        {
            word.push(chars.next().unwrap());
            started = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.push(c);
            }
            started = true;
        } else if c == '#' {
            break;
        } else if c == '"' || c == '\'' {
            quote = Some(c);
            started = true;
        } else if c.is_whitespace() {
            if started {
                words.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(c);
            started = true;
        }
    }
    ensure!(quote.is_none(), "引号未闭合");
    if started {
        words.push(word);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_value_and_pattern_defaults_match_openssh_order() -> Result<()> {
        let r = parse_config(
            "User default\nHost web web2\n HostName %h.example.com\n User overridden\n Port 2222\nHost * !web2\n Port 2200\nHost web2\n Port 2300\n",
        )?;
        assert_eq!(r.servers.len(), 2);
        assert_eq!(r.servers[0].username, "default");
        assert_eq!(r.servers[0].host, "web.example.com");
        assert_eq!(r.servers[1].port, 2222);
        assert_eq!(r.servers[0].port, 2222);
        Ok(())
    }
    #[test]
    fn quoted_paths_comments_equals_and_duplicate_aliases() -> Result<()> {
        let r = parse_config(
            "Host demo DEMO\n HostName=host.example\n User = deploy # comment\n IdentityFile \"C:\\my keys\\key#1.pem\" # ignored\n",
        )?;
        assert_eq!(r.servers.len(), 1);
        assert_eq!(r.servers[0].username, "deploy");
        assert_eq!(r.servers[0].auth, AuthMethod::PrivateKey);
        assert_eq!(
            r.servers[0].private_key,
            Some(PathBuf::from("C:\\my keys\\key#1.pem"))
        );
        Ok(())
    }
    #[test]
    fn proxies_never_become_direct_connections() -> Result<()> {
        let r = parse_config(
            "Host private\n User u\n ProxyJump gateway\nHost direct\n User u\nHost *\n ProxyCommand none\n",
        )?;
        assert_eq!(r.servers.len(), 1);
        assert_eq!(r.servers[0].name, "direct");
        assert!(r.warnings.iter().any(|w| w.contains("private")));
        Ok(())
    }
    #[test]
    fn include_and_match_are_conservatively_rejected_without_execution() -> Result<()> {
        for directive in ["Include ~/.ssh/extra", "Match exec \"touch sentinel\""] {
            let r = parse_config(&format!("Host test\n User u\n{directive}\n"))?;
            assert!(r.servers.is_empty());
            assert!(!r.warnings.is_empty());
        }
        let r = parse_config("Host test\n User u\n IdentityFile $(touch sentinel)\n")?;
        assert!(r.servers.is_empty());
        Ok(())
    }
    #[test]
    fn invalid_profile_is_skipped_but_good_alias_survives() -> Result<()> {
        let r = parse_config(
            "Host bad\n User u\n Port 0\nHost good\n User u\nHost *.internal\n User default\n",
        )?;
        assert_eq!(r.servers.len(), 1);
        assert_eq!(r.servers[0].name, "good");
        assert!(r.warnings.iter().any(|w| w.contains("bad")));
        assert!(parse_config("Host \"unclosed").is_err());
        assert!(parse_config(&"x".repeat(MAX_BYTES + 1)).is_err());
        Ok(())
    }
    #[test]
    fn negative_patterns_override_positive_matches() -> Result<()> {
        let r = parse_config("Host * !blocked\n Port 2200\nHost allowed blocked\n User user\n")?;
        assert_eq!(r.servers[0].port, 2200);
        assert_eq!(r.servers[1].port, 22);
        Ok(())
    }

    #[test]
    fn home_tokens_and_multiple_identities_are_explicit() -> Result<()> {
        let r = parse_config(
            "Host token\n HostName=example.org\n User =deploy\n Port =2222\n IdentityFile \"~/.ssh/%n-%r-%p\"\n IdentityFile /second/key\n",
        )?;
        let profile = &r.servers[0];
        assert_eq!(profile.port, 2222);
        assert_eq!(profile.username, "deploy");
        let home = directories::BaseDirs::new().context("home missing")?;
        assert_eq!(
            profile.private_key,
            Some(home.home_dir().join(".ssh/token-deploy-2222"))
        );
        assert!(r.warnings.iter().any(|w| w.contains("多个 IdentityFile")));
        Ok(())
    }

    #[test]
    fn file_size_and_utf8_are_checked_before_parsing() -> Result<()> {
        let path = std::env::temp_dir().join(format!("arcnix-ssh-config-{}", crate::new_id()));
        let outcome = (|| -> Result<()> {
            std::fs::write(&path, b"Host file\n User deploy\n")?;
            assert_eq!(import_file(&path)?.servers[0].name, "file");
            std::fs::write(&path, [0xff, 0xfe])?;
            assert!(import_file(&path).is_err());
            std::fs::write(&path, vec![b' '; MAX_BYTES + 1])?;
            assert!(import_file(&path).is_err());
            Ok(())
        })();
        std::fs::remove_file(&path)?;
        outcome
    }
}
