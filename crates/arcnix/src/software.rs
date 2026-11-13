use arcnix_core::packages::{Package, PackageAction, PackagePlan, SystemEnvironment};

use super::*;

#[derive(Clone, Default)]
pub(super) struct SoftwareState {
    pub environment: Option<SystemEnvironment>,
    pub packages: Vec<Package>,
    pub query: String,
    pub installed_view: bool,
    pub pending: Option<u64>,
    pub plan: Option<PackagePlan>,
    pub plan_id: Option<u64>,
    pub apply_serial: Option<u64>,
    pub sudo_secret: String,
    pub log: String,
    pub status: String,
    pub error: String,
    pub exit_code: Option<i32>,
    task_started: bool,
}

pub(super) enum Event {
    Environment(std::result::Result<SystemEnvironment, String>),
    Packages(std::result::Result<Vec<Package>, String>),
    Preview(std::result::Result<PackagePlan, String>),
    Started,
    Output(String),
    Finished(std::result::Result<CommandOutput, String>),
}

fn append_log(log: &mut String, text: &str) {
    log.push_str(text);
    if log.len() > 128 * 1024 {
        let boundary = log
            .char_indices()
            .map(|(i, _)| i)
            .find(|i| *i >= log.len() - 96 * 1024)
            .unwrap_or(0);
        log.drain(..boundary);
    }
}

impl Ui {
    fn software_available(&self, require_manager: bool) -> bool {
        self.state.with(|s| {
            s.connected
                && !s.system_task_running
                && s.software.pending.is_none()
                && (!require_manager
                    || s.software
                        .environment
                        .as_ref()
                        .is_some_and(|e| e.manager.supported()))
        })
    }
    pub(super) fn software_detect(&self) {
        if !self.software_available(false) {
            return;
        }
        let serial = self.send(Action::PackageDetect);
        self.state.revise(|s| {
            s.software.pending = Some(serial);
            s.software.error.clear();
            s.software.status = "正在识别系统和包管理器…".into();
        });
    }
    pub(super) fn software_search(&self) {
        if !self.software_available(true) {
            return;
        }
        let query = self.state.with(|s| s.software.query.trim().to_string());
        if query.is_empty() {
            self.state
                .revise(|s| s.software.error = "请输入软件包名称，例如 nginx、git 或 curl".into());
            return;
        }
        let serial = self.send(Action::PackageSearch(query));
        self.state.revise(|s| {
            s.software.pending = Some(serial);
            s.software.installed_view = false;
            s.software.error.clear();
            s.software.status = "正在搜索软件包…".into();
        });
    }
    pub(super) fn software_installed(&self) {
        if !self.software_available(true) {
            return;
        }
        let serial = self.send(Action::PackageInstalled);
        self.state.revise(|s| {
            s.software.pending = Some(serial);
            s.software.installed_view = true;
            s.software.error.clear();
            s.software.status = "正在读取已安装软件…".into();
        });
    }
    pub(super) fn software_preview(&self, action: PackageAction) {
        if !self.software_available(true) {
            return;
        }
        let serial = self.send(Action::PackagePreview(action));
        self.state.revise(|s| {
            s.software.pending = Some(serial);
            s.software.plan = None;
            s.software.plan_id = None;
            s.software.sudo_secret.clear();
            s.software.error.clear();
            s.software.status = "正在生成操作预览…".into();
        });
    }
    pub(super) fn software_apply(&self) {
        if !self.software_available(true)
            || !self
                .state
                .with(|s| s.dialog == Some(Dialog::PackageConfirm))
        {
            return;
        }
        let details = self.state.with(|s| {
            let env = s.software.environment.as_ref()?;
            if !env.is_root && !env.sudo_available {
                return None;
            }
            Some((s.software.plan_id?, s.software.sudo_secret.clone()))
        });
        let Some((plan_id, secret)) = details else {
            self.state
                .revise(|s| s.dialog_error = "当前连接没有可用的管理员权限".into());
            return;
        };
        if secret.len() > 1024 || secret.contains(['\n', '\r', '\0']) {
            self.state
                .revise(|s| s.dialog_error = "管理员密码包含无效字符或过长".into());
            return;
        }
        self.state.revise(|s| {
            s.system_task_running = true;
            s.dialog = None;
            s.dialog_error.clear();
            s.software.sudo_secret.clear();
            s.software.log.clear();
            s.software.exit_code = None;
            s.software.error.clear();
            s.software.status = "系统任务已排队，正在等待 SSH 通道…".into();
            s.software.task_started = false;
        });
        let serial = self.send(Action::PackageApply { plan_id, secret });
        self.state
            .revise(|s| s.software.apply_serial = Some(serial));
    }
}

pub(super) fn event(ui: &Ui, serial: u64, event: Event) {
    match event {
        Event::Environment(result) => {
            if !ui.state.with(|s| s.software.pending == Some(serial)) {
                return;
            }
            let supported = result.as_ref().is_ok_and(|e| e.manager.supported());
            ui.state.revise(|s| {
                s.software.pending = None;
                match result {
                    Ok(env) => {
                        s.software.error.clear();
                        s.software.packages.clear();
                        s.software.status = if env.manager.supported() {
                            "系统识别完成".into()
                        } else {
                            format!("已识别 {}；当前版本支持 APT 软件管理", env.manager.label())
                        };
                        s.software.environment = Some(env);
                    }
                    Err(e) => {
                        s.software.error = e;
                        s.software.status = "系统识别失败".into();
                    }
                }
            });
            if supported {
                ui.software_installed();
            }
        }
        Event::Packages(result) => {
            if !ui.state.with(|s| s.software.pending == Some(serial)) {
                return;
            }
            ui.state.revise(|s| {
                s.software.pending = None;
                match result {
                    Ok(packages) => {
                        s.software.status = format!(
                            "{} · {} 项",
                            if s.software.installed_view {
                                "已安装软件"
                            } else {
                                "搜索结果"
                            },
                            packages.len()
                        );
                        s.software.packages = packages;
                        s.software.error.clear();
                    }
                    Err(e) => {
                        s.software.error = e;
                        s.software.status = "软件包查询失败".into();
                    }
                }
            });
        }
        Event::Preview(result) => {
            if !ui.state.with(|s| s.software.pending == Some(serial)) {
                return;
            }
            let success = result.is_ok();
            ui.state.revise(|s| {
                s.software.pending = None;
                match result {
                    Ok(plan) => {
                        s.software.plan = Some(plan);
                        s.software.plan_id = Some(serial);
                        s.software.status = "操作预览已就绪，尚未执行".into();
                        s.software.sudo_secret.clear();
                    }
                    Err(e) => {
                        s.software.error = e;
                        s.software.status = "无法生成操作预览".into();
                    }
                }
            });
            if success {
                ui.open_dialog(Dialog::PackageConfirm);
            }
        }
        Event::Started => {
            if ui.state.with(|s| s.software.apply_serial == Some(serial)) {
                ui.state.revise(|s| {
                    s.software.status = "正在执行系统任务…".into();
                    s.software.task_started = true;
                });
            }
        }
        Event::Output(output) => {
            if ui.state.with(|s| s.software.apply_serial == Some(serial)) {
                ui.state
                    .revise(|s| append_log(&mut s.software.log, &output));
            }
        }
        Event::Finished(result) => {
            if !ui.state.with(|s| s.software.apply_serial == Some(serial)) {
                return;
            }
            let success = result.as_ref().is_ok_and(|output| output.exit_code == 0);
            ui.state.revise(|s| {
                s.system_task_running = false;
                s.software.apply_serial = None;
                s.software.plan = None;
                s.software.plan_id = None;
                s.software.sudo_secret.clear();
                match result {
                    Ok(output) => {
                        s.software.exit_code = Some(output.exit_code);
                        s.software.status = if output.exit_code == 0 {
                            "系统任务已完成".into()
                        } else {
                            format!("系统任务失败 · 退出码 {}", output.exit_code)
                        };
                        if output.exit_code != 0 {
                            s.software.error =
                                "操作返回错误，部分变更可能已生效。请查看输出并重新查询已安装状态，修复原因后再生成预览。"
                                    .into();
                        }
                        append_log(
                            &mut s.software.log,
                            &i18n::translate(&format!("\n[退出码 {}]\n", output.exit_code)),
                        );
                    }
                    Err(e) => {
                        s.software.status = if s.software.task_started {"任务结果未确认，请重新连接并检查已安装状态"}else{"系统任务未执行"}.into();
                        s.software.error = e.clone();
                        append_log(&mut s.software.log, &i18n::translate(&format!("\n[任务错误] {e}\n")));
                    }
                }
            });
            if success {
                if ui
                    .state
                    .with(|s| !s.software.installed_view && !s.software.query.trim().is_empty())
                {
                    ui.software_search();
                } else {
                    ui.software_installed();
                }
            }
            ui.schedule();
        }
    }
}

pub(super) fn on_connection_lost(ui: &Ui, message: &str) {
    ui.state.revise(|s| {
        if s.system_task_running {
            s.software.error =
                "SSH 连接中断，远程任务结果尚未确认。请重连并查询已安装软件。".into();
            append_log(
                &mut s.software.log,
                &i18n::translate(&format!("\n[连接中断] {message}\n")),
            );
        }
        s.system_task_running = false;
        s.software.pending = None;
        s.software.sudo_secret.clear();
        s.software.plan = None;
        s.software.plan_id = None;
    });
}

fn ready(s: &State) -> bool {
    s.connected
        && !s.system_task_running
        && s.software.pending.is_none()
        && s.software
            .environment
            .as_ref()
            .is_some_and(|e| e.manager.supported())
}

pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let detect = ui.clone();
    let search = ui.clone();
    let enter = ui.clone();
    let installed = ui.clone();
    let refresh = ui.clone();
    col().class("software-page").fill((
        row().class("software-heading").fill((
            col().fill((
                text().class("page-title").content(i18n::bind("系统软件")),
                text()
                    .class("subtle")
                    .content(i18n::bind("查询服务器软件，预览并执行软件安装。")),
            )),
            icon_action("重新识别系统", Icon::Refresh)
                .enabled(
                    state.map(|s| {
                        s.connected && !s.system_task_running && s.software.pending.is_none()
                    }),
                )
                .on_press(move || detect.software_detect()),
        )),
        when(state.map(|s| !s.connected), move || {
            col().class("software-empty").fill((
                text()
                    .class("pane-title")
                    .content(i18n::bind("连接服务器后管理系统软件")),
                text().class("subtle").content(i18n::bind(
                    "选择左侧服务器并连接。软件管理目前支持 Debian / Ubuntu 的 APT。",
                )),
            ))
        }),
        when(state.map(|s| s.connected), move || {
            let search = search.clone();
            let enter = enter.clone();
            let installed = installed.clone();
            let refresh = refresh.clone();
            let table_ui = ui.clone();
            col().class("software-connected").fill((
                row().class("software-environment").fill((
                    text().content(i18n::bind(state.map(|s| {
                        s.software
                            .environment
                            .as_ref()
                            .map(|e| format!("{}  ·  {}", e.distribution, e.manager.label()))
                            .unwrap_or_else(|| "尚未识别系统".into())
                    }))),
                    text().class("subtle").content(i18n::bind(state.map(|s| {
                        s.software
                            .environment
                            .as_ref()
                            .map(|e| {
                                if e.is_root {
                                    "root 会话"
                                } else if e.sudo_available {
                                    "通过 sudo 执行管理员操作"
                                } else {
                                    "无可用管理员权限"
                                }
                                .to_string()
                            })
                            .unwrap_or_default()
                    }))),
                )),
                row().class("software-toolbar").fill((
                    input()
                        .name(i18n::bind("搜索软件包"))
                        .semantics(Some(Box::new(Semantics {
                            placeholder: Some(i18n::translate("软件包名称，如 nginx、git、curl")),
                            ..Default::default()
                        })))
                        .class("software-search")
                        .value(state.map(|s| s.software.query.clone()))
                        .enabled(state.map(ready))
                        .on(value_changed, move |e| {
                            if let glory::ui::Event::ValueChanged { value, .. } = e {
                                state.revise(|s| s.software.query = value);
                            }
                        })
                        .on(commit, move |e| {
                            if let glory::ui::Event::Commit { value, .. } = e {
                                enter.state.revise(|s| s.software.query = value);
                                enter.software_search();
                            }
                        }),
                    sapid::button()
                        .class("button primary")
                        .label(i18n::bind("搜索"))
                        .enabled(state.map(ready))
                        .on_press(move || search.software_search()),
                    sapid::button()
                        .class("button")
                        .label(i18n::bind("已安装"))
                        .enabled(state.map(ready))
                        .on_press(move || installed.software_installed()),
                    sapid::button()
                        .class("button")
                        .label(i18n::bind("更新软件源索引"))
                        .enabled(state.map(ready))
                        .on_press(move || refresh.software_preview(PackageAction::RefreshIndex)),
                )),
                text()
                    .class("software-status")
                    .content(i18n::bind(state.map(|s| s.software.status.clone()))),
                text()
                    .class(state.map(|s| {
                        if s.software.error.is_empty() {
                            "hidden"
                        } else {
                            "software-error"
                        }
                        .to_string()
                    }))
                    .content(i18n::bind(state.map(|s| s.software.error.clone()))),
                package_table(table_ui),
                col().class("software-task").fill((
                    row().class("software-task-heading").fill((
                        text().content(i18n::bind(state.map(|s| {
                            s.software
                                .exit_code
                                .map(|code| {
                                    format!(
                                        "任务输出 · {} · 退出码 {code}",
                                        if code == 0 {
                                            "执行成功"
                                        } else {
                                            "执行失败"
                                        }
                                    )
                                })
                                .unwrap_or_else(|| "任务输出".into())
                        }))),
                        text().class("subtle").content(i18n::bind(state.map(|s| {
                            if s.system_task_running {
                                "任务执行期间不能切换服务器，请保持窗口和连接。"
                            } else {
                                "安装操作需先预览并确认。"
                            }
                            .to_string()
                        }))),
                    )),
                    text()
                        .class("software-output")
                        .selectable(true)
                        .content(i18n::bind(state.map(|s| {
                            if s.software.log.is_empty() {
                                "尚未执行系统任务".into()
                            } else {
                                s.software.log.clone()
                            }
                        }))),
                )),
            ))
        }),
    ))
}

fn package_table(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("software-packages").fill((
        row().class("software-table-header").fill((
            text().class("package-name").content(i18n::bind("软件包")),
            text()
                .class("package-version")
                .content(i18n::bind("已安装版本")),
            text()
                .class("package-description")
                .content(i18n::bind("说明")),
            text().class("package-action").content(i18n::bind("操作")),
        )),
        col().class("software-package-list").fill(
            each(state.map(|s| s.software.packages.clone()))
                .key(|p| p.name.clone())
                .children(move |item| {
                    let package = item.value;
                    let install = ui.clone();
                    row().class("software-package-row").fill((
                        text()
                            .class("package-name")
                            .content(package.map(|p| p.name.clone())),
                        text().class("package-version").content(package.map(|p| {
                            if p.version.is_empty() {
                                "—".into()
                            } else {
                                p.version.clone()
                            }
                        })),
                        text()
                            .class("package-description")
                            .content(package.map(|p| p.summary.clone())),
                        sapid::button()
                            .class("button compact package-action")
                            .name(i18n::bind(package.map(|p| {
                                format!(
                                    "{} {}",
                                    if p.installed {
                                        "已安装"
                                    } else {
                                        "预览安装"
                                    },
                                    p.name
                                )
                            })))
                            .label(i18n::bind(package.map(|p| {
                                if p.installed {
                                    "已安装"
                                } else {
                                    "安装…"
                                }
                                .to_string()
                            })))
                            .enabled(state.map(move |s| ready(s) && !package.get().installed))
                            .on_press(move || {
                                install.software_preview(PackageAction::Install(vec![
                                    package.get().name,
                                ]))
                            }),
                    ))
                }),
        ),
        text()
            .class(state.map(|s| {
                if s.software.packages.is_empty() {
                    "software-list-empty"
                } else {
                    "hidden"
                }
                .to_string()
            }))
            .content(i18n::bind(state.map(|s| {
                if s.software.pending.is_some() {
                    "正在读取…"
                } else {
                    "暂无软件包结果。输入名称搜索，或查看已安装软件。"
                }
                .to_string()
            }))),
    ))
}

pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::PackageConfirm)),
        move || {
            let submit = ui.clone();
            let cancel = ui.clone();
            modal(
                ui.clone(),
                "确认系统操作",
                col().class("form-stack").fill((
                    text()
                        .class("dialog-description")
                        .content(i18n::bind(state.map(|s| {
                            s.config
                                .servers
                                .iter()
                                .find(|p| Some(&p.id) == s.selected.as_ref())
                                .map(|p| {
                                    format!(
                                        "目标服务器：{} · {}@{}:{}",
                                        p.name, p.username, p.host, p.port
                                    )
                                })
                                .unwrap_or_default()
                        }))),
                    text()
                        .class("pane-title")
                        .content(i18n::bind(state.map(|s| {
                            s.software
                                .plan
                                .as_ref()
                                .map(|p| match &p.action {
                                    PackageAction::Install(names) => {
                                        format!("安装 {}", names.join("、"))
                                    }
                                    PackageAction::RefreshIndex => "更新软件源索引".into(),
                                })
                                .unwrap_or_default()
                        }))),
                    text()
                        .class("subtle")
                        .content(i18n::bind("请检查预览。只有点击下方确认按钮后才会执行。")),
                    text()
                        .class("package-preview")
                        .selectable(true)
                        .content(i18n::bind(state.map(|s| {
                            s.software
                                .plan
                                .as_ref()
                                .map(|p| p.preview.clone())
                                .unwrap_or_default()
                        }))),
                    text()
                        .class("field-label")
                        .content(i18n::bind("将执行的命令")),
                    text()
                        .class("package-command")
                        .selectable(true)
                        .content(state.map(|s| {
                            match (&s.software.environment, &s.software.plan) {
                                (Some(env), Some(plan)) => arcnix_core::packages::build_execution(
                                    env,
                                    plan,
                                    !s.software.sudo_secret.is_empty(),
                                )
                                .map(|e| command_display(&e.command))
                                .unwrap_or_else(|e| e.to_string()),
                                _ => String::new(),
                            }
                        })),
                    when(
                        state.map(|s| {
                            s.software
                                .environment
                                .as_ref()
                                .is_some_and(|e| !e.is_root && e.sudo_available)
                        }),
                        move || {
                            col().class("field").fill((
                                text()
                                    .class("field-label")
                                    .content(i18n::bind("管理员密码（sudo 需要时填写）")),
                                input()
                                    .name(i18n::bind("sudo 管理员密码"))
                                    .purpose(Purpose::Password)
                                    .value(state.map(|s| s.software.sudo_secret.clone()))
                                    .on(value_changed, move |e| {
                                        if let glory::ui::Event::ValueChanged { value, .. } = e {
                                            state.revise(|s| s.software.sudo_secret = value);
                                        }
                                    }),
                                text().class("subtle").content(i18n::bind(
                                    "仅用于本次提权，不保存，也不会写入命令或日志。",
                                )),
                            ))
                        },
                    ),
                    text().class("subtle").content(i18n::bind(state.map(|s| {
                        s.software
                            .environment
                            .as_ref()
                            .map(|e| {
                                if e.is_root {
                                    "当前为 root 会话，无需再次提权。"
                                } else if e.sudo_available {
                                    "留空时使用非交互 sudo；需要密码时请填写本次管理员密码。"
                                } else {
                                    "当前服务器没有 sudo，请使用 root 或具备管理员权限的账号连接。"
                                }
                                .to_string()
                            })
                            .unwrap_or_default()
                    }))),
                )),
                (
                    btn("取消", move || {
                        cancel.state.revise(|s| s.software.sudo_secret.clear());
                        cancel.close_dialog();
                    }),
                    sapid::button()
                        .class("button primary")
                        .label(i18n::bind("确认执行"))
                        .enabled(state.map(|s| {
                            ready(s)
                                && s.software.plan_id.is_some()
                                && s.software
                                    .environment
                                    .as_ref()
                                    .is_some_and(|e| e.is_root || e.sudo_available)
                        }))
                        .on_press(move || submit.software_apply()),
                ),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_log_is_bounded_without_splitting_unicode() {
        let mut log = "测".repeat(50000);
        append_log(&mut log, "\n完成");
        assert!(log.len() <= 128 * 1024);
        assert!(log.ends_with("完成"));
    }
}
