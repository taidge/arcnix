//! Application navigation is independent of the active SSH workspace.
use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Servers,
    Notifications,
    Settings,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SettingsSection {
    #[default]
    General,
    Appearance,
    Files,
    Monitoring,
}

impl Ui {
    pub(super) fn navigate(&self, page: Page) {
        self.state.revise(|s| {
            s.navigation = page;
            s.host_menu = None;
            s.group_menu = None;
            s.file_menu = None;
            if page == Page::Notifications {
                s.unread_notifications = 0;
            }
        });
        if page == Page::Servers {
            self.observability_enter();
            self.terminal_enter();
        }
    }
}

pub(super) fn workspace_visible(s: &State) -> bool {
    s.navigation == Page::Servers
        && s.connected
        && s.config
            .servers
            .iter()
            .any(|p| Some(&p.id) == s.selected.as_ref())
}

fn rail_button(
    ui: &Ui,
    page: Page,
    label: &'static str,
    icon: Icon,
    class: &'static str,
) -> impl Widget {
    let state = ui.state;
    let ui = ui.clone();
    sapid::button()
        .name(i18n::bind(label))
        .class(state.map(move |s| {
            format!(
                "rail-item {class}{}",
                if s.navigation == page { " active" } else { "" }
            )
        }))
        .fill(icons::glyph(icon))
        .on_press(move || ui.navigate(page))
}

fn rail(ui: Ui) -> impl Widget {
    col()
        .class("navigation-rail")
        .name(i18n::bind("主导航"))
        .fill((
            row()
                .class("rail-item rail-logo")
                .fill(text().class("brand-icon").content("")),
            text().class("rail-divider").content(""),
            rail_button(&ui, Page::Servers, "服务器", Icon::Server, "rail-servers"),
            col().class("rail-bottom").fill(rail_button(
                &ui,
                Page::Settings,
                "Settings",
                Icon::Settings,
                "settings-link",
            )),
        ))
}

fn sidebar_link(
    ui: &Ui,
    label: &'static str,
    icon: Icon,
    action: impl FnMut() + 'static,
) -> sapid::Button {
    let _ = ui;
    text_action(label, icon)
        .class("button with-icon context-link")
        .on_press(action)
}

fn context_sidebar(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("context-sidebar").fill((
        text()
            .class("context-title")
            .content(i18n::bind(state.map(|s| {
                match s.navigation {
                    Page::Notifications => "通知",
                    Page::Settings => "Settings",
                    Page::Servers => "服务器",
                }
                .to_owned()
            }))),
        when(state.map(|s| s.navigation == Page::Notifications), {
            let ui = ui.clone();
            move || {
                col().class("context-links").fill((
                    sidebar_link(&ui, "所有活动", Icon::Bell, || {})
                        .class("button with-icon context-link active"),
                    text()
                        .class("context-hint")
                        .content(i18n::bind("连接、传输和操作记录集中显示在这里。")),
                ))
            }
        }),
        when(state.map(|s| s.navigation == Page::Settings), {
            let ui = ui.clone();
            move || {
                col().class("context-links").fill(
                    each(vec![
                        SettingsSection::General,
                        SettingsSection::Appearance,
                        SettingsSection::Files,
                        SettingsSection::Monitoring,
                    ])
                    .key(|section| *section as u8)
                    .children({
                        let ui = ui.clone();
                        move |item| {
                            let section = item.value.get();
                            let choose = ui.clone();
                            sapid::button()
                                .class(state.map(move |s| {
                                    if s.settings_section == section {
                                        "context-link active"
                                    } else {
                                        "context-link"
                                    }
                                    .to_owned()
                                }))
                                .label(i18n::bind(match section {
                                    SettingsSection::General => "General",
                                    SettingsSection::Appearance => "Appearance",
                                    SettingsSection::Files => "Files",
                                    SettingsSection::Monitoring => "Monitoring",
                                }))
                                .on_press(move || {
                                    choose.state.revise(|s| s.settings_section = section)
                                })
                        }
                    }),
                )
            }
        }),
        col().class("context-footer").fill((
            text()
                .class("context-footer-title")
                .content(i18n::bind("你的服务器工作台")),
            text()
                .class("subtle")
                .content(i18n::bind("通过 SSH 连接，无需远程 Agent。")),
            text()
                .class(state.map(|s| {
                    if s.connected {
                        "session-indicator online"
                    } else {
                        "session-indicator"
                    }
                    .to_owned()
                }))
                .content(i18n::bind(state.map(|s| {
                    if s.connected {
                        "一个活动连接"
                    } else {
                        "当前没有活动连接"
                    }
                    .to_owned()
                }))),
        )),
    ))
}

fn empty_state(
    title: &'static str,
    description: &'static str,
    actions: impl IntoChildren + 'static,
) -> impl Widget {
    col().class("navigation-empty").fill((
        icons::glyph(Icon::Server),
        text().class("empty-title").content(i18n::bind(title)),
        text()
            .class("empty-description")
            .content(i18n::bind(description)),
        row().class("empty-actions").fill(actions),
    ))
}

fn server_landing(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("server-landing page-scroll").fill((
        when(state.map(|s| s.selected.is_none()), {
            let ui = ui.clone();
            move || {
                empty_state(
                    "选择一台服务器",
                    "从左侧选择服务器，连接后即可开始管理。",
                    text_action("新建服务器", Icon::Plus)
                        .class("button with-icon primary")
                        .on_press({
                            let ui = ui.clone();
                            move || servers::new_server(&ui)
                        }),
                )
            }
        }),
        when(state.map(|s| s.selected.is_some()), {
            let ui = ui.clone();
            move || {
                let connect = ui.clone();
                let edit = ui.clone();
                col().class("server-overview").fill((
                    text()
                        .class("overview-server-icon action-glyph glyph-server")
                        .content(""),
                    text().class("home-title").content(state.map(|s| {
                        s.config
                            .servers
                            .iter()
                            .find(|p| Some(&p.id) == s.selected.as_ref())
                            .map(|p| p.name.clone())
                            .unwrap_or_default()
                    })),
                    text()
                        .class("home-description")
                        .content(i18n::bind(state.map(|s| {
                            if s.connecting {
                                "正在建立安全连接…"
                            } else {
                                "连接后即可使用文件、软件、资源、日志与终端。"
                            }
                            .to_owned()
                        }))),
                    row().class("server-facts").fill((
                        detail("主机", &ui, |p| p.host.clone()),
                        detail("端口", &ui, |p| p.port.to_string()),
                        detail("用户", &ui, |p| p.username.clone()),
                        detail("认证方式", &ui, |p| {
                            i18n::translate(if p.auth == AuthMethod::Password {
                                "密码"
                            } else {
                                "私钥"
                            })
                        }),
                    )),
                    row().class("home-actions").fill((
                        sapid::button()
                            .class("button primary overview-connect")
                            .label(i18n::bind(state.map(|s| {
                                if s.connecting {
                                    "取消连接"
                                } else {
                                    "连接服务器"
                                }
                                .to_owned()
                            })))
                            .on_press(move || {
                                if state.with(|s| s.connecting) {
                                    connect.disconnect();
                                } else {
                                    connect.open_dialog(Dialog::Connect);
                                }
                            }),
                        btn("编辑服务器", move || {
                            if let Some(id) = state.with(|s| s.selected.clone()) {
                                servers::edit_server(&edit, &id);
                            }
                        }),
                    )),
                    text()
                        .class("overview-footnote subtle")
                        .content(i18n::bind("密码与私钥口令仅用于本次连接，不会保存到配置。")),
                ))
            }
        }),
    ))
}

fn detail(label: &'static str, ui: &Ui, value: fn(&ServerProfile) -> String) -> impl Widget {
    col().class("server-fact").fill((
        text().class("subtle").content(i18n::bind(label)),
        text()
            .class("server-fact-value")
            .content(ui.state.map(move |s| {
                s.config
                    .servers
                    .iter()
                    .find(|p| Some(&p.id) == s.selected.as_ref())
                    .map(value)
                    .unwrap_or_default()
            })),
    ))
}

fn notifications(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("notifications-page page-scroll").fill((
        text().class("page-title").content(i18n::bind("通知与活动")),
        text().class("subtle").content(i18n::bind(
            "本次会话中的连接、传输与操作记录，最新记录显示在前。",
        )),
        col().class("notification-feed").fill(
            each(state.map(|s| {
                s.logs
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .enumerate()
                    .map(|(index, line)| (index, line.to_owned()))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .take(100)
                    .collect::<Vec<_>>()
            }))
            .key(|(index, line)| (*index, line.clone()))
            .children(|item| {
                row().class("notification-entry").fill((
                    icons::glyph(Icon::Bell),
                    text()
                        .class("notification-message")
                        .selectable(true)
                        .content(item.value.map(|(_, line)| line.clone())),
                ))
            }),
        ),
        text().class("subtle").content(i18n::bind(
            "显示最近 100 条记录。活动记录仅保留在本次会话。",
        )),
    ))
}

fn tabs(ui: Ui) -> impl Widget {
    let state = ui.state;
    row().class("module-nav").fill((
        module_tab(ui.clone(), WorkspacePage::Files, "文件管理"),
        sapid::button()
            .class(state.map(|s| {
                if s.workspace_page == WorkspacePage::Software {
                    "module-tab active"
                } else {
                    "module-tab"
                }
                .to_owned()
            }))
            .label(i18n::bind("系统软件"))
            .on_press({
                let ui = ui.clone();
                move || {
                    ui.state
                        .revise(|s| s.workspace_page = WorkspacePage::Software);
                    if ui.state.with(|s| {
                        s.connected
                            && s.software.environment.is_none()
                            && s.software.pending.is_none()
                    }) {
                        ui.software_detect();
                    }
                }
            }),
        module_tab(ui.clone(), WorkspacePage::Resources, "资源概览"),
        module_tab(ui.clone(), WorkspacePage::SystemLogs, "系统日志"),
        module_tab(ui, WorkspacePage::Terminal, "终端"),
    ))
}

fn server_modules(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("server-modules").fill((
        when(state.map(|s| s.workspace_page == WorkspacePage::Files), {
            let ui = ui.clone();
            move || {
                col().class("files-module").fill((
                    workspace::files(ui.clone()),
                    workspace::activity(ui.clone()),
                ))
            }
        }),
        when(
            state.map(|s| s.workspace_page == WorkspacePage::Software),
            {
                let ui = ui.clone();
                move || software::page(ui.clone())
            },
        ),
        when(
            state.map(|s| s.workspace_page == WorkspacePage::Resources),
            {
                let ui = ui.clone();
                move || observability::resources(ui.clone())
            },
        ),
        when(
            state.map(|s| s.workspace_page == WorkspacePage::SystemLogs),
            move || observability::logs(ui.clone()),
        ),
    ))
}

pub(super) fn shell(ui: Ui) -> impl Widget {
    let state = ui.state;
    let view_log = ui.clone();
    let dismiss = ui.clone();
    row().class("navigation-shell").fill((
        rail(ui.clone()),
        when(state.map(|s| s.navigation == Page::Servers), {
            let ui = ui.clone();
            move || servers::sidebar(ui.clone())
        })
        .otherwise({
            let ui = ui.clone();
            move || context_sidebar(ui.clone())
        }),
        col().class("main-panel").fill((
            row().class("page-header").fill((
                text()
                    .class("page-header-title")
                    .content(i18n::bind(state.map(|s| {
                        match s.navigation {
                            Page::Servers => "服务器",
                            Page::Notifications => "所有活动",
                            Page::Settings => "应用偏好设置",
                        }
                        .to_owned()
                    }))),
                when(
                    state.map(|s| s.navigation == Page::Servers && s.selected.is_some()),
                    {
                        let ui = ui.clone();
                        move || connection_controls(ui.clone())
                    },
                ),
                when(state.map(|s| s.navigation != Page::Servers), || {
                    text().class("header-product").content("Arcnix")
                }),
            )),
            when(state.map(workspace_visible), {
                let ui = ui.clone();
                move || tabs(ui.clone())
            }),
            col().class("workspace-content").fill((
                row()
                    .class(state.map(|s| {
                        if s.notice.is_empty() {
                            "hidden"
                        } else {
                            "notice"
                        }
                        .to_owned()
                    }))
                    .fill((
                        text()
                            .class("notice-text")
                            .content(i18n::bind(state.map(|s| s.notice.clone()))),
                        btn("查看操作日志", move || {
                            view_log.navigate(Page::Notifications)
                        }),
                        close_action("忽略提示")
                            .on_press(move || dismiss.state.revise(|s| s.notice.clear())),
                    )),
                when(state.map(|s| s.navigation == Page::Settings), {
                    let ui = ui.clone();
                    move || settings::page(ui.clone())
                }),
                when(state.map(|s| s.navigation == Page::Notifications), {
                    let ui = ui.clone();
                    move || notifications(ui.clone())
                }),
                when(
                    state.map(|s| s.navigation == Page::Servers && !workspace_visible(s)),
                    {
                        let ui = ui.clone();
                        move || server_landing(ui.clone())
                    },
                ),
                when(
                    state.map(|s| {
                        workspace_visible(s) && s.workspace_page != WorkspacePage::Terminal
                    }),
                    {
                        let ui = ui.clone();
                        move || server_modules(ui.clone())
                    },
                ),
                // Keep the terminal surface mounted when navigating; its native SSH session
                // continues.
                terminal::page(ui),
            )),
        )),
    ))
}
