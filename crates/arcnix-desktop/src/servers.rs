use super::*;

pub(super) fn new_server(ui: &Ui) {
    ui.navigate(navigation::Page::Servers);
    ui.state.revise(|s| {
        s.editor = ServerProfile::default();
        s.editor.group_id = s.group.clone().filter(|id| !id.is_empty());
        s.port = "22".into();
    });
    ui.open_dialog(Dialog::Server);
}

fn new_group(ui: &Ui) {
    ui.state.revise(|s| s.group_name.clear());
    ui.open_dialog(Dialog::Group { id: None });
}

pub(super) fn edit_server(ui: &Ui, id: &str) {
    let profile = ui
        .state
        .with(|s| s.config.servers.iter().find(|p| p.id == id).cloned());
    if let Some(profile) = profile {
        ui.state.revise(|s| {
            s.port = profile.port.to_string();
            s.editor = profile;
        });
        ui.open_dialog(Dialog::Server);
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Reorder {
    group: String,
    target: String,
    after: bool,
    #[serde(rename = "sequence")]
    _sequence: u64,
}

fn reorder(ui: &Ui, value: &str) {
    if value.len() > 2048 {
        return;
    }
    let Ok(gesture) = serde_json::from_str::<Reorder>(value) else {
        return;
    };
    if ui
        .state
        .with(|s| !s.search.trim().is_empty() || s.dialog.is_some())
    {
        return;
    }
    let result = ui.state.with(|s| -> anyhow::Result<Option<AppConfig>> {
        let mut config = s.config.clone();
        if !config.move_group(&gesture.group, &gesture.target, gesture.after)? {
            return Ok(None);
        }
        persist(ui, &config)?;
        Ok(Some(config))
    });
    match result {
        Ok(Some(config)) => ui.state.revise(|s| s.config = config),
        Ok(None) => {}
        Err(error) => ui.error(format!("无法保存分组顺序：{error:#}")),
    }
}

fn expanded(s: &State, id: &str) -> bool {
    !s.search.trim().is_empty() || !s.collapsed_groups.contains(id)
}

fn matches_server(profile: &ServerProfile, query: &str) -> bool {
    format!("{} {} {}", profile.name, profile.host, profile.username)
        .to_lowercase()
        .contains(query)
}

fn matching_servers(s: &State, group: Option<&str>) -> Vec<ServerProfile> {
    let query = s.search.trim().to_lowercase();
    let group_matches = group
        .and_then(|id| s.config.groups.iter().find(|g| g.id == id))
        .is_some_and(|g| g.name.to_lowercase().contains(&query));
    s.config
        .servers
        .iter()
        .filter(|p| p.group_id.as_deref() == group && (group_matches || matches_server(p, &query)))
        .cloned()
        .collect()
}

fn group_menu(ui: Ui, id: String) -> impl Widget {
    let state = ui.state;
    let add = ui.clone();
    let rename = ui.clone();
    let remove = ui.clone();
    let destroy = ui.clone();
    let toggle = ui.clone();
    let add_id = id.clone();
    let rename_id = id.clone();
    let delete_id = id.clone();
    let destroy_id = id.clone();
    let toggle_id = id.clone();
    let guarded_id = id.clone();
    col().class("context-popover group-context-menu").fill((
        btn("新建服务器", move || {
            add.state.revise(|s| s.group = Some(add_id.clone()));
            new_server(&add);
        })
        .class("menu-item"),
        sapid::button()
            .class("menu-item")
            .label(i18n::bind(state.map(move |s| {
                if expanded(s, &id) {
                    "折叠分组"
                } else {
                    "展开分组"
                }
                .to_owned()
            })))
            .enabled(state.map(|s| s.search.trim().is_empty()))
            .on_press(move || {
                toggle.state.revise(|s| {
                    if !s.collapsed_groups.remove(&toggle_id) {
                        s.collapsed_groups.insert(toggle_id.clone());
                    }
                    s.group_menu = None;
                })
            }),
        btn("重命名分组", move || {
            if let Some(name) = state.with(|s| {
                s.config
                    .groups
                    .iter()
                    .find(|g| g.id == rename_id)
                    .map(|g| g.name.clone())
            }) {
                rename.state.revise(|s| s.group_name = name);
                rename.open_dialog(Dialog::Group {
                    id: Some(rename_id.clone()),
                });
            }
        })
        .class("menu-item"),
        btn("解散分组（保留服务器）", move || {
            remove.open_dialog(Dialog::DissolveGroup(delete_id.clone()))
        })
        .class("menu-item"),
        btn("删除分组及服务器", move || {
            destroy.open_dialog(Dialog::DeleteGroupAndServers(destroy_id.clone()))
        })
        .class("menu-item danger")
        .enabled(state.map({
            let id = guarded_id.clone();
            move |s| {
                !s.system_task_running
                    || !s.config.servers.iter().any(|p| {
                        s.selected.as_deref() == Some(&p.id) && p.group_id.as_deref() == Some(&id)
                    })
            }
        })),
    ))
}

fn root_menu(ui: Ui) -> impl Widget {
    let add = ui.clone();
    let group = ui.clone();
    let import = ui.clone();
    let expand = ui.clone();
    let collapse = ui.clone();
    col().class("context-popover tree-root-menu").fill((
        btn("新建服务器", move || {
            add.state.revise(|s| s.group = None);
            new_server(&add);
        })
        .class("menu-item"),
        btn("新建分组", move || new_group(&group)).class("menu-item"),
        btn("导入 SSH 配置", move || {
            import.open_dialog(Dialog::Import)
        })
        .class("menu-item"),
        btn("展开全部", move || {
            expand.state.revise(|s| {
                s.collapsed_groups.clear();
                s.group_menu = None;
            })
        })
        .class("menu-item"),
        btn("折叠全部", move || {
            collapse.state.revise(|s| {
                s.collapsed_groups = s.config.groups.iter().map(|g| g.id.clone()).collect();
                s.search.clear();
                s.group_menu = None;
            })
        })
        .class("menu-item"),
    ))
}

pub(super) fn sidebar(ui: Ui) -> impl Widget {
    let state = ui.state;
    let add = ui.clone();
    let group = ui.clone();
    let menu = ui.clone();
    col().class("sidebar server-explorer").fill((
        row()
            .class(state.map(|s| {
                if s.group_menu.as_deref() == Some("") {
                    "sidebar-heading tree-root-row menu-open"
                } else {
                    "sidebar-heading tree-root-row"
                }
                .to_owned()
            }))
            .fill((
                text().class("sidebar-title").content(i18n::bind("服务器")),
                row().class("tree-actions").fill((
                    icon_action("新建服务器", Icon::Plus)
                        .class("icon-button tree-new-server")
                        .on_press(move || new_server(&add)),
                    icon_action("新建分组", Icon::Folder)
                        .class("icon-button tree-new-group")
                        .on_press(move || new_group(&group)),
                    icon_action("服务器列表操作", Icon::More)
                        .class("icon-button tree-more")
                        .on_press(move || {
                            menu.state.revise(|s| {
                                s.host_menu = None;
                                s.group_menu = if s.group_menu.as_deref() == Some("") {
                                    None
                                } else {
                                    Some(String::new())
                                };
                            })
                        }),
                )),
                when(state.map(|s| s.group_menu.as_deref() == Some("")), {
                    let ui = ui.clone();
                    move || root_menu(ui.clone())
                }),
            )),
        row().class("server-search").fill((
            input()
                .name(i18n::bind("搜索"))
                .value(state.map(|s| s.search.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| s.search = value);
                    }
                }),
            text()
                .class("action-glyph glyph-search server-search-icon")
                .content(""),
        )),
        input()
            .class("server-tree-input hidden")
            .value("")
            .on(value_changed, {
                let ui = ui.clone();
                move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        reorder(&ui, &value);
                    }
                }
            }),
        col()
            .class(state.map(|s| {
                if s.search.trim().is_empty() {
                    "server-tree server-list"
                } else {
                    "server-tree server-list reorder-disabled"
                }
                .to_owned()
            }))
            .name(i18n::bind("服务器树"))
            .fill((
                each(state.map(|s| {
                    let query = s.search.trim().to_lowercase();
                    s.config
                        .groups
                        .iter()
                        .filter(|g| {
                            g.name.to_lowercase().contains(&query)
                                || !matching_servers(s, Some(&g.id)).is_empty()
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                }))
                .key(|g| (g.id.clone(), g.name.clone()))
                .children({
                    let ui = ui.clone();
                    move |item| {
                        let g = item.value;
                        let choose = ui.clone();
                        let menu = ui.clone();
                        col().class("tree-folder").fill((
                            row()
                                .class(state.map(move |s| {
                                    format!(
                                        "group-row tree-folder-row{}{}{}",
                                        if expanded(s, &g.get().id) {
                                            " expanded"
                                        } else {
                                            ""
                                        },
                                        if s.group.as_deref() == Some(&g.get().id) {
                                            " selected"
                                        } else {
                                            ""
                                        },
                                        if s.group_menu.as_deref() == Some(&g.get().id) {
                                            " menu-open"
                                        } else {
                                            ""
                                        }
                                    )
                                }))
                                .fill((
                                    text()
                                        .class("tree-group-id hidden")
                                        .content(g.map(|g| g.id.clone())),
                                    sapid::button()
                                        .class("group-button")
                                        .name(g.map(|g| g.name.clone()))
                                        .fill((
                                            text()
                                                .class(
                                                    "tree-chevron action-glyph glyph-chevron-right",
                                                )
                                                .content(""),
                                            icons::glyph(Icon::Folder),
                                            text()
                                                .class("tree-folder-name")
                                                .content(g.map(|g| g.name.clone())),
                                            text().class("tree-count").content(state.map(
                                                move |s| {
                                                    s.config
                                                        .servers
                                                        .iter()
                                                        .filter(|p| {
                                                            p.group_id.as_deref()
                                                                == Some(&g.get().id)
                                                        })
                                                        .count()
                                                        .to_string()
                                                },
                                            )),
                                        ))
                                        .on_press(move || {
                                            choose.state.revise(|s| {
                                                let id = g.get().id;
                                                s.group = Some(id.clone());
                                                s.host_menu = None;
                                                s.group_menu = None;
                                                if s.search.trim().is_empty()
                                                    && !s.collapsed_groups.remove(&id)
                                                {
                                                    s.collapsed_groups.insert(id);
                                                }
                                            })
                                        }),
                                    icon_action(format!("{}的分组操作", g.get().name), Icon::More)
                                        .class("icon-button group-more")
                                        .on_press(move || {
                                            menu.state.revise(|s| {
                                                let id = g.get().id;
                                                s.group = Some(id.clone());
                                                s.host_menu = None;
                                                s.group_menu = if s.group_menu.as_ref() == Some(&id)
                                                {
                                                    None
                                                } else {
                                                    Some(id)
                                                };
                                            })
                                        }),
                                    when(
                                        state.map(move |s| {
                                            s.group_menu.as_deref() == Some(&g.get().id)
                                        }),
                                        {
                                            let ui = ui.clone();
                                            move || group_menu(ui.clone(), g.get().id)
                                        },
                                    ),
                                )),
                            when(state.map(move |s| expanded(s, &g.get().id)), {
                                let ui = ui.clone();
                                move || {
                                    let id = g.get().id;
                                    col().class("tree-children").fill((
                                        server_rows(ui.clone(), Some(id.clone())),
                                        when(
                                            state.map(move |s| {
                                                matching_servers(s, Some(&id)).is_empty()
                                            }),
                                            || {
                                                text()
                                                    .class("tree-empty-group")
                                                    .content(i18n::bind("空分组"))
                                            },
                                        ),
                                    ))
                                }
                            }),
                        ))
                    }
                }),
                server_rows(ui.clone(), None),
                when(
                    state.map(|s| {
                        !s.search.trim().is_empty()
                            && !s
                                .config
                                .servers
                                .iter()
                                .any(|p| matches_server(p, &s.search.trim().to_lowercase()))
                            && !s.config.groups.iter().any(|g| {
                                g.name
                                    .to_lowercase()
                                    .contains(&s.search.trim().to_lowercase())
                            })
                    }),
                    || {
                        text()
                            .class("tree-no-results")
                            .content(i18n::bind("未找到匹配的服务器或分组"))
                    },
                ),
                when(
                    state.map(|s| s.config.servers.is_empty() && s.config.groups.is_empty()),
                    || {
                        text()
                            .class("tree-empty-state")
                            .content(i18n::bind("新建分组来整理服务器，或添加第一台服务器。"))
                    },
                ),
            )),
        text()
            .class("tree-footer")
            .content(i18n::bind(state.map(|s| {
                format!(
                    "{} 个分组 · {} 台服务器",
                    s.config.groups.len(),
                    s.config.servers.len()
                )
            }))),
    ))
}

fn server_rows(ui: Ui, group: Option<String>) -> impl Widget {
    let state = ui.state;
    each(state.map(move |s| matching_servers(s, group.as_deref())))
        .key(|p| (p.id.clone(), p.name.clone(), p.host.clone(), p.port))
        .children({
            let ui = ui.clone();
            move |item| {
                let value = item.value;
                let select = ui.clone();
                let menu = ui.clone();
                let edit = ui.clone();
                let delete = ui.clone();
                let connect = ui.clone();
                let reset = ui.clone();
                row()
                    .class(state.map(move |s| {
                        format!(
                            "host-row{}{}",
                            if s.selected.as_deref() == Some(&value.get().id) {
                                " selected"
                            } else {
                                ""
                            },
                            if s.host_menu.as_deref() == Some(&value.get().id) {
                                " menu-open"
                            } else {
                                ""
                            }
                        )
                    }))
                    .fill((
                        icons::glyph(Icon::Server),
                        sapid::button()
                            .class("host-select")
                            .enabled(state.map(move |s| {
                                !s.system_task_running
                                    || s.selected.as_deref() == Some(&value.get().id)
                            }))
                            .name(value.map(|p| {
                                format!("{}，{}@{}:{}", p.name, p.username, p.host, p.port)
                            }))
                            .fill((
                                text()
                                    .class("host-name")
                                    .content(value.map(|p| p.name.clone())),
                                text().class("host-address").content(i18n::bind(state.map(
                                    move |s| {
                                        let p = value.get();
                                        let status = if s.selected.as_deref() == Some(&p.id)
                                            && s.connected
                                        {
                                            " · 已连接"
                                        } else if s.selected.as_deref() == Some(&p.id)
                                            && s.connecting
                                        {
                                            " · 连接中"
                                        } else {
                                            ""
                                        };
                                        let status = i18n::translate(status);
                                        format!("{}@{}:{}{status}", p.username, p.host, p.port)
                                    },
                                ))),
                            ))
                            .on_press(move || {
                                let id = value.get().id;
                                select.select_server(id);
                                state.revise(|s| s.group = value.get().group_id);
                            }),
                        icon_action(format!("{}的服务器操作", value.get().name), Icon::More)
                            .class("icon-button host-more")
                            .on_press(move || {
                                menu.state.revise(|s| {
                                    let id = value.get().id;
                                    s.group_menu = None;
                                    s.host_menu = if s.host_menu.as_ref() == Some(&id) {
                                        None
                                    } else {
                                        Some(id)
                                    };
                                })
                            }),
                        when(
                            state.map(move |s| s.host_menu.as_deref() == Some(&value.get().id)),
                            move || {
                                let edit = edit.clone();
                                let delete = delete.clone();
                                let connect = connect.clone();
                                let reset = reset.clone();
                                col().class("context-popover").fill((
                                    sapid::button()
                                        .class("menu-item")
                                        .label(i18n::bind(state.map(move |s| {
                                            if s.selected.as_deref() == Some(&value.get().id)
                                                && s.connected
                                            {
                                                "断开连接"
                                            } else if s.selected.as_deref() == Some(&value.get().id)
                                                && s.connecting
                                            {
                                                "取消连接"
                                            } else {
                                                "连接服务器"
                                            }
                                            .to_string()
                                        })))
                                        .enabled(state.map(|s| !s.system_task_running))
                                        .on_press(move || {
                                            let id = value.get().id;
                                            if connect.state.with(|s| {
                                                s.selected.as_ref() == Some(&id)
                                                    && (s.connected || s.connecting)
                                            }) {
                                                connect.disconnect();
                                            } else {
                                                connect.select_server(id);
                                                connect.open_dialog(Dialog::Connect);
                                            }
                                            connect.state.revise(|s| s.host_menu = None);
                                        }),
                                    btn("编辑服务器", move || {
                                        edit_server(&edit, &value.get().id)
                                    })
                                    .class("menu-item"),
                                    btn("重置文件冲突选择", move || {
                                        reset.reset_conflict_rules(&value.get().id)
                                    })
                                    .class("menu-item"),
                                    btn("删除服务器", move || {
                                        delete.open_dialog(Dialog::DeleteServer(value.get().id))
                                    })
                                    .class("menu-item danger"),
                                ))
                            },
                        ),
                    ))
            }
        })
}

fn cancel(ui: Ui) -> sapid::Button {
    let state = ui.state;
    btn("取消", move || ui.close_dialog())
        .class("button quiet")
        .enabled(state.map(|s| s.file_pending.is_none() && s.import_pending.is_none()))
}

fn persist(ui: &Ui, config: &AppConfig) -> anyhow::Result<()> {
    anyhow::ensure!(
        ui.writable,
        "当前配置读取失败，不能覆盖。请修复配置后重启。"
    );
    ui.store.save(config)
}

fn server_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let title = state.with(|s| {
        if s.config.servers.iter().any(|p| p.id == s.editor.id) {
            "编辑服务器"
        } else {
            "新建服务器"
        }
    });
    let password = ui.clone();
    let key = ui.clone();
    let ungrouped = ui.clone();
    let save = ui.clone();
    let body = col().class("form-stack").fill((
        field(
            "名称",
            &ui,
            |s| s.editor.name.clone(),
            |s, v| s.editor.name = v,
        ),
        row().class("form-row host-port").fill((
            field(
                "主机地址",
                &ui,
                |s| s.editor.host.clone(),
                |s, v| s.editor.host = v,
            ),
            field("端口", &ui, |s| s.port.clone(), |s, v| s.port = v),
        )),
        field(
            "用户名",
            &ui,
            |s| s.editor.username.clone(),
            |s, v| s.editor.username = v,
        ),
        col().class("field").fill((
            text().class("field-label").content(i18n::bind("认证方式")),
            row().class("segmented").fill((
                sapid::button()
                    .class(state.map(|s| {
                        if s.editor.auth == AuthMethod::Password {
                            "segment active"
                        } else {
                            "segment"
                        }
                        .to_owned()
                    }))
                    .label(i18n::bind("密码"))
                    .on_press(move || {
                        password
                            .state
                            .revise(|s| s.editor.auth = AuthMethod::Password)
                    }),
                sapid::button()
                    .class(state.map(|s| {
                        if s.editor.auth == AuthMethod::PrivateKey {
                            "segment active"
                        } else {
                            "segment"
                        }
                        .to_owned()
                    }))
                    .label(i18n::bind("私钥"))
                    .on_press(move || key.state.revise(|s| s.editor.auth = AuthMethod::PrivateKey)),
            )),
        )),
        when(state.map(|s| s.editor.auth == AuthMethod::PrivateKey), {
            let ui = ui.clone();
            move || file_picker::field(ui.clone())
        }),
        col().class("field").fill((
            text().class("field-label").content(i18n::bind("分组")),
            row().class("choice-list").fill((
                sapid::button()
                    .class(state.map(|s| {
                        if s.editor.group_id.is_none() {
                            "choice active"
                        } else {
                            "choice"
                        }
                        .to_owned()
                    }))
                    .label(i18n::bind("未分组"))
                    .on_press(move || ungrouped.state.revise(|s| s.editor.group_id = None)),
                each(state.map(|s| s.config.groups.clone()))
                    .key(|g| g.id.clone())
                    .children({
                        let ui = ui.clone();
                        move |item| {
                            let value = item.value;
                            let ui = ui.clone();
                            sapid::button()
                                .class(state.map(move |s| {
                                    if s.editor.group_id.as_deref() == Some(&value.get().id) {
                                        "choice active"
                                    } else {
                                        "choice"
                                    }
                                    .to_owned()
                                }))
                                .label(value.map(|g| g.name.clone()))
                                .on_press(move || {
                                    ui.state
                                        .revise(|s| s.editor.group_id = Some(value.get().id))
                                })
                        }
                    }),
            )),
        )),
        field(
            "初始远程目录",
            &ui,
            |s| s.editor.initial_remote_path.clone(),
            |s, v| s.editor.initial_remote_path = v,
        ),
        text()
            .class("subtle")
            .content(i18n::bind("密码与私钥口令将在连接时输入。")),
    ));
    modal(
        ui.clone(),
        title,
        body,
        (
            cancel(ui),
            btn("保存服务器", move || {
                let result = save
                    .state
                    .with(|s| -> anyhow::Result<(AppConfig, ServerProfile)> {
                        let mut profile = s.editor.clone();
                        anyhow::ensure!(
                            !s.system_task_running || s.selected.as_ref() != Some(&profile.id),
                            "系统软件任务正在执行，完成后才能修改当前服务器"
                        );
                        profile.name = profile.name.trim().into();
                        profile.host = profile.host.trim().into();
                        profile.username = profile.username.trim().into();
                        profile.port = s
                            .port
                            .trim()
                            .parse()
                            .map_err(|_| anyhow::anyhow!("端口应为 1–65535 的数字"))?;
                        let mut config = s.config.clone();
                        config.upsert_server(profile.clone())?;
                        persist(&save, &config)?;
                        Ok((config, profile))
                    });
                match result {
                    Ok((config, profile)) => {
                        let same = save
                            .state
                            .with(|s| s.selected.as_ref() == Some(&profile.id));
                        if same {
                            save.disconnect();
                        }
                        save.state.revise(|s| {
                            s.config = config;
                            // Reveal the saved destination even if its folder was collapsed.
                            s.group = Some(profile.group_id.clone().unwrap_or_default());
                            if let Some(group) = &profile.group_id {
                                s.collapsed_groups.remove(group);
                            }
                            s.search.clear();
                            s.jobs.retain(|j| {
                                j.server_id != profile.id || j.status == TransferStatus::Completed
                            });
                            // Creating another site must not interrupt an active SSH
                            // session; disconnected workspaces can select the saved site.
                            if same || (!s.connected && !s.connecting) {
                                s.file_split =
                                    profile.file_pane_ratio.unwrap_or(500) as f64 / 1000.0;
                                s.selected = Some(profile.id);
                            }
                            s.dialog = None;
                            s.dialog_error.clear();
                            s.notice.clear();
                        });
                    }
                    Err(error) => save.state.revise(|s| s.dialog_error = format!("{error:#}")),
                }
            })
            .class("button primary"),
        ),
    )
}

fn group_dialog(ui: Ui) -> impl Widget {
    let id = ui.state.with(|s| match &s.dialog {
        Some(Dialog::Group { id }) => id.clone(),
        _ => None,
    });
    let title = if id.is_some() {
        "重命名分组"
    } else {
        "新建分组"
    };
    let save = ui.clone();
    modal(
        ui.clone(),
        title,
        field(
            "分组名称",
            &ui,
            |s| s.group_name.clone(),
            |s, v| s.group_name = v,
        ),
        (
            cancel(ui),
            btn("保存分组", move || {
                let result = save.state.with(|s| -> anyhow::Result<(AppConfig, String)> {
                    let mut config = s.config.clone();
                    let group_id = if let Some(id) = &id {
                        config.rename_group(id, s.group_name.clone())?;
                        id.clone()
                    } else {
                        config.add_group(s.group_name.clone())?
                    };
                    persist(&save, &config)?;
                    Ok((config, group_id))
                });
                match result {
                    Ok((config, group_id)) => save.state.revise(|s| {
                        s.config = config;
                        s.collapsed_groups.remove(&group_id);
                        s.group = Some(group_id);
                        s.search.clear();
                        s.dialog = None;
                        s.notice.clear();
                    }),
                    Err(error) => save.state.revise(|s| s.dialog_error = format!("{error:#}")),
                }
            })
            .class("button primary"),
        ),
    )
}

fn import_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let submit = ui.clone();
    modal(
        ui.clone(),
        "导入 SSH 配置",
        col().class("form-stack").fill((
            text().class("subtle").content(i18n::bind(
                "导入 Host 站点与密钥路径，不读取私钥内容或执行配置命令。",
            )),
            field(
                "SSH 配置文件路径",
                &ui,
                |s| s.import_path.clone(),
                |s, v| s.import_path = v,
            ),
            text()
                .class("subtle")
                .content(i18n::bind("不支持的代理、Include 和 Match 会报告并跳过。")),
        )),
        (
            cancel(ui),
            sapid::button()
                .class("button primary")
                .label(i18n::bind(state.map(|s| {
                    if s.import_pending.is_some() {
                        "正在导入…"
                    } else {
                        "导入站点"
                    }
                    .to_owned()
                })))
                .enabled(state.map(|s| s.import_pending.is_none()))
                .on_press(move || {
                    let path = submit.state.with(|s| s.import_path.trim().to_owned());
                    if path.is_empty() {
                        submit
                            .state
                            .revise(|s| s.dialog_error = "请输入 SSH 配置文件路径".into());
                        return;
                    }
                    let serial = submit.send(Action::ImportSshConfig(path.into()));
                    submit.state.revise(|s| {
                        s.import_pending = Some(serial);
                        s.dialog_error.clear();
                    });
                }),
        ),
    )
}

fn connect_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let connect = ui.clone();
    let enter = ui.clone();
    modal(
        ui.clone(),
        "连接服务器",
        col().class("form-stack").fill((
            text()
                .class("dialog-description")
                .content(i18n::bind(state.map(|s| {
                    s.config
                        .servers
                        .iter()
                        .find(|p| Some(&p.id) == s.selected.as_ref())
                        .map(|p| format!("{}\n{}@{}:{}", p.name, p.username, p.host, p.port))
                        .unwrap_or_else(|| "请选择服务器".into())
                }))),
            col().class("field").fill((
                text()
                    .class("field-label")
                    .content(i18n::bind(state.map(|s| {
                        if s.config
                            .servers
                            .iter()
                            .find(|p| Some(&p.id) == s.selected.as_ref())
                            .is_some_and(|p| p.auth == AuthMethod::PrivateKey)
                        {
                            "私钥口令（未加密可留空）"
                        } else {
                            "密码"
                        }
                        .to_owned()
                    }))),
                input()
                    .name(i18n::bind("密码或私钥口令"))
                    .purpose(Purpose::Password)
                    .value(state.map(|s| s.secret.clone()))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event {
                            state.revise(|s| s.secret = value);
                        }
                    })
                    .on(commit, move |event| {
                        if let Event::Commit { value, .. } = event {
                            enter.state.revise(|s| s.secret = value);
                            begin_connect(&enter);
                        }
                    }),
            )),
            text()
                .class("subtle")
                .content(i18n::bind("凭据仅用于本次连接，不会保存到配置。")),
        )),
        (
            cancel(ui),
            btn("连接", move || {
                begin_connect(&connect);
            })
            .class("button primary"),
        ),
    )
}

fn begin_connect(ui: &Ui) {
    if !ui
        .state
        .with(|s| s.dialog == Some(Dialog::Connect) && !s.connecting && !s.connected)
    {
        return;
    }
    ui.state.revise(|s| {
        s.dialog = None;
        s.dialog_error.clear();
    });
    ui.connect();
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeleteKind {
    Server,
    DissolveGroup,
    GroupAndServers,
}

fn delete_dialog(ui: Ui, kind: DeleteKind) -> impl Widget {
    let state = ui.state;
    let group = kind != DeleteKind::Server;
    let target = state.with(|s| match &s.dialog {
        Some(Dialog::DeleteServer(id))
        | Some(Dialog::DissolveGroup(id))
        | Some(Dialog::DeleteGroupAndServers(id)) => id.clone(),
        _ => String::new(),
    });
    let name = state
        .with(|s| {
            if group {
                s.config
                    .groups
                    .iter()
                    .find(|g| g.id == target)
                    .map(|g| g.name.clone())
            } else {
                s.config
                    .servers
                    .iter()
                    .find(|p| p.id == target)
                    .map(|p| p.name.clone())
            }
        })
        .unwrap_or_default();
    let members = state.with(|s| {
        s.config
            .servers
            .iter()
            .filter(|p| p.group_id.as_deref() == Some(&target))
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
    });
    let title = match kind {
        DeleteKind::Server => "删除服务器",
        DeleteKind::DissolveGroup => "解散分组",
        DeleteKind::GroupAndServers => "删除分组及服务器",
    };
    let details = match kind {
        DeleteKind::Server => i18n::translate("仅删除保存的站点配置，不会删除远程服务器上的文件。"),
        DeleteKind::DissolveGroup => {
            i18n::translate("只移除分组，所有服务器配置保留并移到根目录。")
        }
        DeleteKind::GroupAndServers => i18n::translate(&format!(
            "将删除此分组及其 {} 台服务器的本地配置。不会删除远程服务器或远程文件。",
            members.len()
        )),
    };
    let targets = if kind == DeleteKind::GroupAndServers {
        members.join("\n")
    } else {
        String::new()
    };
    let confirm = ui.clone();
    modal(
        ui.clone(),
        title,
        col().class("form-stack").fill((
            text()
                .class("dialog-description")
                .content(format!("{} “{name}”？", i18n::translate(title))),
            text().class("subtle").content(details),
            text()
                .class(if targets.is_empty() {
                    "hidden"
                } else {
                    "delete-targets"
                })
                .content(targets),
        )),
        (
            cancel(ui),
            btn(title, move || {
                let result = state.with(|s| -> anyhow::Result<AppConfig> {
                    let affects_active = kind != DeleteKind::DissolveGroup
                        && s.config.servers.iter().any(|p| {
                            s.selected.as_deref() == Some(&p.id)
                                && if group {
                                    p.group_id.as_deref() == Some(&target)
                                } else {
                                    p.id == target
                                }
                        });
                    anyhow::ensure!(
                        !s.system_task_running || !affects_active,
                        "系统软件任务正在执行，完成后才能删除当前服务器"
                    );
                    let mut config = s.config.clone();
                    let removed = match kind {
                        DeleteKind::Server => config.delete_server(&target),
                        DeleteKind::DissolveGroup => config.delete_group(&target),
                        DeleteKind::GroupAndServers => config.delete_group_and_servers(&target),
                    };
                    anyhow::ensure!(removed, "目标已不存在，请关闭对话框后重试。");
                    persist(&confirm, &config)?;
                    Ok(config)
                });
                match result {
                    Ok(config) => {
                        let removed = state.with(|s| {
                            s.config
                                .servers
                                .iter()
                                .filter(|p| !config.servers.iter().any(|q| q.id == p.id))
                                .map(|p| p.id.clone())
                                .collect::<HashSet<_>>()
                        });
                        let affected = state
                            .with(|s| s.selected.as_ref().is_some_and(|id| removed.contains(id)));
                        if affected {
                            confirm.disconnect();
                        }
                        state.revise(|s| {
                            s.config = config;
                            if group {
                                s.collapsed_groups.remove(&target);
                                if s.group.as_ref() == Some(&target) {
                                    s.group = None;
                                }
                            }
                            if affected {
                                s.selected = None;
                                s.file_split = 0.5;
                            }
                            s.jobs.retain(|j| !removed.contains(&j.server_id));
                            s.dialog = None;
                            s.notice.clear();
                        });
                    }
                    Err(error) => state.revise(|s| s.dialog_error = format!("{error:#}")),
                }
            })
            .class(if kind == DeleteKind::DissolveGroup {
                "button primary"
            } else {
                "button danger"
            }),
        ),
    )
}

pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        when(state.map(|s| s.dialog == Some(Dialog::Server)), {
            let ui = ui.clone();
            move || server_dialog(ui.clone())
        }),
        when(
            state.map(|s| matches!(s.dialog, Some(Dialog::Group { .. }))),
            {
                let ui = ui.clone();
                move || group_dialog(ui.clone())
            },
        ),
        when(state.map(|s| s.dialog == Some(Dialog::Import)), {
            let ui = ui.clone();
            move || import_dialog(ui.clone())
        }),
        when(state.map(|s| s.dialog == Some(Dialog::Connect)), {
            let ui = ui.clone();
            move || connect_dialog(ui.clone())
        }),
        when(
            state.map(|s| matches!(s.dialog, Some(Dialog::DeleteServer(_)))),
            {
                let ui = ui.clone();
                move || delete_dialog(ui.clone(), DeleteKind::Server)
            },
        ),
        when(
            state.map(|s| matches!(s.dialog, Some(Dialog::DissolveGroup(_)))),
            {
                let ui = ui.clone();
                move || delete_dialog(ui.clone(), DeleteKind::DissolveGroup)
            },
        ),
        when(
            state.map(|s| matches!(s.dialog, Some(Dialog::DeleteGroupAndServers(_)))),
            move || delete_dialog(ui.clone(), DeleteKind::GroupAndServers),
        ),
    ))
}
