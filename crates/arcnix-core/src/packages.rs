//! Typed, noninteractive APT operations. Only a confirmed plan may be executed.
use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail, ensure};

use crate::remote::RemoteSession;

pub const SEARCH_LIMIT: usize = 200;
pub const INSTALLED_LIMIT: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Apt,
    Dnf,
    Yum,
    Apk,
    Pacman,
    Unknown,
}
impl PackageManager {
    pub fn label(self) -> &'static str {
        match self {
            Self::Apt => "APT",
            Self::Dnf => "DNF",
            Self::Yum => "YUM",
            Self::Apk => "APK",
            Self::Pacman => "Pacman",
            Self::Unknown => "未识别",
        }
    }
    pub fn supported(self) -> bool {
        self == Self::Apt
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemEnvironment {
    pub distribution: String,
    pub manager: PackageManager,
    pub is_root: bool,
    pub sudo_available: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub summary: String,
    pub installed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageAction {
    Install(Vec<String>),
    RefreshIndex,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagePlan {
    pub action: PackageAction,
    pub command: String,
    pub preview: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Execution {
    pub command: String,
    pub use_stdin_password: bool,
}

pub const DETECT_COMMAND: &str = "# arcnix:packages:detect\nprintf 'ARCNIX_OS_BEGIN\\n'; cat /etc/os-release 2>/dev/null; printf '\\nARCNIX_OS_END\\nARCNIX_MANAGER\\t'; if command -v apt-get >/dev/null 2>&1 && command -v apt-cache >/dev/null 2>&1 && command -v dpkg-query >/dev/null 2>&1; then printf 'apt\\n'; elif command -v dnf >/dev/null 2>&1; then printf 'dnf\\n'; elif command -v yum >/dev/null 2>&1; then printf 'yum\\n'; elif command -v apk >/dev/null 2>&1; then printf 'apk\\n'; elif command -v pacman >/dev/null 2>&1; then printf 'pacman\\n'; else printf 'unknown\\n'; fi; printf 'ARCNIX_UID\\t'; id -u; printf 'ARCNIX_SUDO\\t'; if command -v sudo >/dev/null 2>&1; then printf '1\\n'; else printf '0\\n'; fi";
pub const INSTALLED_COMMAND: &str = "# arcnix:packages:installed\nLC_ALL=C dpkg-query -W -f='${binary:Package}\\t${Version}\\t${db:Status-Abbrev}\\t${binary:Summary}\\n'";

fn output(session: &RemoteSession, command: &str) -> Result<String> {
    let result = session.exec(command)?;
    ensure!(
        result.exit_code == 0,
        "远程命令失败（退出码 {}）：{}{}",
        result.exit_code,
        result.stdout.trim(),
        result.stderr.trim()
    );
    Ok(result.stdout)
}
fn require_apt(env: &SystemEnvironment) -> Result<()> {
    ensure!(
        env.manager.supported(),
        "已识别 {}，当前版本仅支持 Debian/Ubuntu 的 APT 操作",
        env.manager.label()
    );
    Ok(())
}
pub fn detect(session: &RemoteSession) -> Result<SystemEnvironment> {
    parse_environment(&output(session, DETECT_COMMAND)?)
}
pub fn parse_environment(text: &str) -> Result<SystemEnvironment> {
    let (_, rest) = text
        .split_once("ARCNIX_OS_BEGIN\n")
        .context("服务器系统识别输出不完整")?;
    let (os, fields) = rest
        .split_once("ARCNIX_OS_END\n")
        .context("服务器系统识别输出不完整")?;
    let get = |key: &str| {
        fields
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .context("服务器系统识别缺少字段")
    };
    let manager = match get("ARCNIX_MANAGER\t")? {
        "apt" => PackageManager::Apt,
        "dnf" => PackageManager::Dnf,
        "yum" => PackageManager::Yum,
        "apk" => PackageManager::Apk,
        "pacman" => PackageManager::Pacman,
        _ => PackageManager::Unknown,
    };
    let uid: u32 = get("ARCNIX_UID\t")?.parse().context("服务器用户编号无效")?;
    let sudo_available = match get("ARCNIX_SUDO\t")? {
        "1" => true,
        "0" => false,
        _ => bail!("sudo 识别结果无效"),
    };
    let distribution = os
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .unwrap_or("Linux（发行版未知）")
        .trim_matches(['\"', '\''])
        .chars()
        .filter(|c| !c.is_control())
        .take(256)
        .collect();
    Ok(SystemEnvironment {
        distribution,
        manager,
        is_root: uid == 0,
        sudo_available,
    })
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn validate_package(name: &str) -> Result<()> {
    ensure!(
        name.len() >= 2 && name.len() <= 128 && name.is_ascii(),
        "软件包名称须为 2–128 个 ASCII 字符"
    );
    let mut parts = name.split(':');
    let base = parts.next().unwrap_or("");
    ensure!(
        base.len() >= 2
            && base
                .as_bytes()
                .first()
                .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && base
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"+.-".contains(&c))
            && !base.ends_with('-'),
        "软件包名称无效；不接受选项、路径、版本表达式或移除后缀"
    );
    if let Some(arch) = parts.next() {
        ensure!(
            !arch.is_empty()
                && arch
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && parts.next().is_none(),
            "软件包架构无效"
        );
    }
    Ok(())
}
fn normalize_names(names: &[String]) -> Result<Vec<String>> {
    ensure!(
        !names.is_empty() && names.len() <= 20,
        "每次请选择 1–20 个软件包"
    );
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for name in names {
        validate_package(name)?;
        if seen.insert(name.clone()) {
            result.push(name.clone());
        }
    }
    Ok(result)
}
fn args(names: &[String]) -> String {
    names.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
}
fn literal_query(query: &str) -> Result<String> {
    ensure!(
        (2..=64).contains(&query.len())
            && query.is_ascii()
            && query
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(&c)),
        "请输入 2–64 个字母、数字或 +.- 作为软件包关键词"
    );
    Ok(query
        .chars()
        .flat_map(|c| {
            if "+.".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect())
}
pub fn parse_installed(text: &str) -> Vec<Package> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let name = fields.next()?;
            let version = fields.next()?;
            let status = fields.next()?;
            if validate_package(name).is_err()
                || status.as_bytes().get(1) != Some(&b'i')
                || status.as_bytes().get(2).is_some_and(|c| *c != b' ')
            {
                return None;
            }
            Some(Package {
                name: name.into(),
                version: version.into(),
                summary: fields.next().unwrap_or("").into(),
                installed: true,
            })
        })
        .collect()
}
pub fn installed(session: &RemoteSession, env: &SystemEnvironment) -> Result<Vec<Package>> {
    require_apt(env)?;
    let mut rows = parse_installed(&output(session, INSTALLED_COMMAND)?);
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows.truncate(INSTALLED_LIMIT);
    Ok(rows)
}
pub fn search(
    session: &RemoteSession,
    env: &SystemEnvironment,
    query: &str,
) -> Result<Vec<Package>> {
    require_apt(env)?;
    let query = literal_query(query.trim())?;
    let command = format!(
        "# arcnix:packages:search\nLC_ALL=C apt-cache --names-only search -- {}",
        quote(&query)
    );
    let results = output(session, &command)?;
    let installed = parse_installed(&output(session, INSTALLED_COMMAND)?);
    let index: HashMap<_, _> = installed.iter().map(|p| (p.name.as_str(), p)).collect();
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for line in results.lines() {
        let Some((name, summary)) = line.split_once(" - ") else {
            continue;
        };
        if validate_package(name).is_err() || !seen.insert(name.to_owned()) {
            continue;
        }
        let previous = index.get(name).copied().or_else(|| {
            installed
                .iter()
                .find(|p| p.name.split(':').next() == Some(name))
        });
        rows.push(Package {
            name: name.into(),
            summary: summary.into(),
            version: previous.map(|p| p.version.clone()).unwrap_or_default(),
            installed: previous.is_some(),
        });
        if rows.len() == SEARCH_LIMIT {
            break;
        }
    }
    Ok(rows)
}

fn verify_exact_packages(text: &str, names: &[String]) -> Result<()> {
    let records = text
        .split("\n\n")
        .map(|paragraph| {
            let field = |prefix: &str| paragraph.lines().find_map(|line| line.strip_prefix(prefix));
            (field("Package: "), field("Architecture: "))
        })
        .collect::<Vec<_>>();
    for requested in names {
        let (base, arch) = requested
            .split_once(':')
            .map_or((requested.as_str(), None), |(n, a)| (n, Some(a)));
        ensure!(
            records
                .iter()
                .any(|(n, a)| *n == Some(base) && arch.is_none_or(|expected| *a == Some(expected))),
            "未找到精确软件包 {requested}，拒绝 APT 通配/正则回退"
        );
    }
    Ok(())
}
fn validate_simulation(text: &str) -> Result<()> {
    ensure!(
        !text
            .lines()
            .any(|line| line.trim_start().starts_with("Remv ")),
        "安装预览涉及移除现有软件包，已拒绝执行"
    );
    Ok(())
}
pub fn preview(
    session: &RemoteSession,
    env: &SystemEnvironment,
    action: PackageAction,
) -> Result<PackagePlan> {
    require_apt(env)?;
    let (action,preview)=match action {
        PackageAction::Install(names)=>{
            let names=normalize_names(&names)?;
            let metadata=output(session,&format!("# arcnix:packages:verify\nLC_ALL=C apt-cache --no-all-versions show -- {}",args(&names)))?;
            verify_exact_packages(&metadata,&names)?;
            let preview=output(session,&format!("# arcnix:packages:preview\nLC_ALL=C apt-get --simulate --no-remove install -- {}",args(&names)))?;
            validate_simulation(&preview)?;
            (PackageAction::Install(names),preview)
        },
        PackageAction::RefreshIndex=>(PackageAction::RefreshIndex,"更新 APT 软件源索引，以读取最新可用软件包信息。不会升级已安装的软件包。\n此操作需要网络与管理员权限。".into()),
    };
    let mut plan = PackagePlan {
        action,
        command: String::new(),
        preview,
    };
    plan.command = build_execution(env, &plan, false)?.command;
    Ok(plan)
}
pub fn build_execution(
    env: &SystemEnvironment,
    plan: &PackagePlan,
    use_password: bool,
) -> Result<Execution> {
    require_apt(env)?;
    ensure!(
        env.is_root || env.sudo_available,
        "当前账户不是 root，且服务器未提供 sudo"
    );
    let (marker, operation) = match &plan.action {
        PackageAction::Install(names) => {
            let names = normalize_names(names)?;
            (
                "install",
                format!(
                    "apt-get -y --no-remove -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold install -- {}",
                    args(&names)
                ),
            )
        }
        PackageAction::RefreshIndex => (
            "update",
            "apt-get -o APT::Update::Error-Mode=any update".into(),
        ),
    };
    let sudo = if env.is_root {
        ""
    } else if use_password {
        "sudo -S -p '' -- "
    } else {
        "sudo -n -- "
    };
    Ok(Execution {
        command: format!(
            "# arcnix:packages:{marker}\n{sudo}env LC_ALL=C DEBIAN_FRONTEND=noninteractive {operation}"
        ),
        use_stdin_password: !env.is_root && use_password,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn env() -> SystemEnvironment {
        SystemEnvironment {
            distribution: "Ubuntu".into(),
            manager: PackageManager::Apt,
            is_root: false,
            sudo_available: true,
        }
    }
    #[test]
    fn rejects_shell_and_apt_option_injection() {
        for name in [
            "-y",
            "nginx-",
            "foo=1",
            "foo/stable",
            "a;id",
            "$(id)",
            "a b",
            "foo*",
            "foo:amd64:all",
            "Apt",
            ".foo",
            "a\nxx",
        ] {
            assert!(validate_package(name).is_err(), "{name}");
        }
        for name in ["nginx", "g++", "libc6:amd64", "libfoo.1", "python3-pip"] {
            assert!(validate_package(name).is_ok(), "{name}");
        }
        assert!(literal_query("nginx;id").is_err());
        assert_eq!(literal_query("g++").unwrap(), "g\\+\\+");
    }
    #[test]
    fn exact_metadata_blocks_regex_fallback_and_wrong_architecture() {
        let text = "Package: libfoo1\nArchitecture: amd64\n\nPackage: nginx\nArchitecture: amd64\n";
        assert!(verify_exact_packages(text, &["libfoo.1".into()]).is_err());
        assert!(verify_exact_packages(text, &["nginx:arm64".into()]).is_err());
        assert!(verify_exact_packages(text, &["nginx:amd64".into()]).is_ok());
    }
    #[test]
    fn execution_is_rebuilt_without_trusting_display_command_or_secret() {
        let plan = PackagePlan {
            action: PackageAction::Install(vec!["nginx".into()]),
            command: "rm -rf /".into(),
            preview: String::new(),
        };
        let execution = build_execution(&env(), &plan, false).unwrap();
        assert!(execution.command.contains("sudo -n -- env"));
        assert!(execution.command.contains("--no-remove"));
        assert!(!execution.command.contains("rm -rf"));
        assert!(!execution.use_stdin_password);
        let execution = build_execution(&env(), &plan, true).unwrap();
        assert!(execution.command.contains("sudo -S -p '' --"));
        assert!(execution.use_stdin_password);
        let mut root = env();
        root.is_root = true;
        let execution = build_execution(&root, &plan, true).unwrap();
        assert!(!execution.command.contains("sudo"));
        assert!(!execution.use_stdin_password);
    }
    #[test]
    fn unsupported_manager_and_no_privilege_cannot_install() {
        let plan = PackagePlan {
            action: PackageAction::RefreshIndex,
            command: String::new(),
            preview: String::new(),
        };
        let mut info = env();
        info.manager = PackageManager::Dnf;
        assert!(build_execution(&info, &plan, false).is_err());
        info.manager = PackageManager::Apt;
        info.sudo_available = false;
        assert!(build_execution(&info, &plan, false).is_err());
    }
    #[test]
    fn only_truly_installed_dpkg_states_are_listed() {
        let packages = parse_installed(
            "curl\t8.5\tii \tTransfer\nold\t1\trc \tRemoved\nheld\t2\thi \tHeld installed\nbad\t1\tiiR\tBroken\n",
        );
        assert_eq!(
            packages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["curl", "held"]
        );
    }
    #[test]
    fn simulation_never_accepts_removal_and_install_sets_are_bounded() {
        assert!(validate_simulation("Inst nginx (1.2)\nConf nginx (1.2)\n").is_ok());
        assert!(validate_simulation("Inst nginx (1.2)\n Remv libc6 [1.2]\n").is_err());
        assert!(normalize_names(&[]).is_err());
        assert!(normalize_names(&vec!["nginx".into(); 21]).is_err());
        assert_eq!(
            normalize_names(&["nginx".into(), "nginx".into()]).unwrap(),
            ["nginx"]
        );
        assert!(validate_package("a:amd64").is_err());
    }
    #[test]
    fn parses_detected_system_without_executing_os_release() {
        let info=parse_environment("ARCNIX_OS_BEGIN\nPRETTY_NAME=\"Ubuntu $(do-not-run)\"\nARCNIX_OS_END\nARCNIX_MANAGER\tapt\nARCNIX_UID\t1000\nARCNIX_SUDO\t1\n").unwrap();
        assert_eq!(info.distribution, "Ubuntu $(do-not-run)");
        assert!(!info.is_root);
        assert!(info.sudo_available);
        assert!(parse_environment("unexpected output").is_err());
    }
}
