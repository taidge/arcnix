use super::*;

pub(super) fn files(ui: Ui) -> impl Widget {
    splitter::panes(ui.clone(), pane(ui.clone(), false), pane(ui, true))
}

fn ready(s: &State, remote: bool) -> bool {
    if remote {
        s.connected && s.sftp_available && s.remote_ready
    } else {
        s.local_ready
    }
}
fn selected(s: &State, remote: bool) -> &HashSet<String> {
    if remote {
        &s.remote_selection
    } else {
        &s.local_selection
    }
}
fn selection(s: &mut State, remote: bool) -> &mut HashSet<String> {
    if remote {
        &mut s.remote_selection
    } else {
        &mut s.local_selection
    }
}
fn begin_file(ui: &Ui, remote: bool, rename: bool) {
    ui.state.revise(|s| {
        s.name = if rename {
            let entries = file_browser::entries(s, remote);
            entries
                .iter()
                .find(|f| selected(s, remote).contains(&f.path))
                .map(|f| f.name.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };
    });
    ui.open_dialog(Dialog::File { remote, rename });
}
fn sort_button(
    ui: Ui,
    remote: bool,
    sort: Sort,
    label: &'static str,
    class: &'static str,
) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(class)
        .label(i18n::bind(state.map(move |s| {
            let label = i18n::translate(label);
            let (current, descending) = if remote {
                (s.remote_sort, s.remote_desc)
            } else {
                (s.local_sort, s.local_desc)
            };
            if current == sort {
                format!("{label} {}", if descending { "↓" } else { "↑" })
            } else {
                label
            }
        })))
        .on_press(move || {
            ui.state.revise(|s| {
                let (current, descending) = if remote {
                    (&mut s.remote_sort, &mut s.remote_desc)
                } else {
                    (&mut s.local_sort, &mut s.local_desc)
                };
                if *current == sort {
                    *descending = !*descending
                } else {
                    *current = sort;
                    *descending = false
                }
            })
        })
}
fn pane(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let transfer = ui.clone();
    let filter = ui.clone();
    let menu = ui.clone();
    let mkdir = ui.clone();
    let rename = ui.clone();
    let all = ui.clone();
    let clear = ui.clone();
    col().class(state.map(move |s| if file_browser::icons(s, remote) { "file-pane icons-view" } else { "file-pane details-view" }.to_owned())).fill((
        row().class("pane-heading").fill((
            col().class("pane-heading-copy").fill((
                text().class("pane-title").content(i18n::bind(if remote {
                    "远程文件"
                } else {
                    "本地文件"
                })),
                text().class("pane-summary").content(i18n::bind(state.map(move |s| {
                    if remote && !s.connected {
                        return if s.connecting {
                            "SFTP · 等待 SSH 认证"
                        } else {
                            "SFTP · 等待 SSH 连接"
                        }
                        .into();
                    }
                    if remote && !s.sftp_available {
                        return "SFTP · 不可用".into();
                    }
                    let count = if remote {
                        s.remote.len()
                    } else {
                        s.local.len()
                    };
                    if remote {
                        format!("SFTP · {count} 个项目")
                    } else {
                        format!("{count} 个项目")
                    }
                }))),
            )),
            text_action(
                if remote { "下载" } else { "上传" },
                if remote { Icon::Download } else { Icon::Upload },
            )
            .class("button with-icon primary")
            .enabled(state.map(move |s| {
                s.connected && s.local_ready && s.remote_ready && !file_browser::selected_entries(s, remote).is_empty()
            }))
            .on_press(move || {
                transfer.transfer(if remote {
                    TransferDirection::Download
                } else {
                    TransferDirection::Upload
                })
            }),
            file_browser::view_controls(ui.clone(), remote),
        )),
        file_browser::pathbar(ui.clone(), remote),
        row().class("pane-tools").fill((
            text()
                .class("pane-summary")
                .content(i18n::bind(state.map(move |s| format!("已选 {} 项", selected(s, remote).len())))),
            btn("全选", move || {
                all.state.revise(|s| {
                    let paths = pane_entries(s, remote)
                        .into_iter()
                        .map(|f| f.path)
                        .collect();
                    *selection(s, remote) = paths;
                })
            }),
            btn("清空", move || {
                clear.state.revise(|s| selection(s, remote).clear())
            }),
            icon_action(
                if remote {
                    "筛选远程文件"
                } else {
                    "筛选本地文件"
                },
                Icon::Filter,
            )
            .class(state.map(move |s| {
                if if remote {
                    s.remote_filter_open
                } else {
                    s.local_filter_open
                } {
                    "icon-button active"
                } else {
                    "icon-button"
                }
                .to_string()
            }))
            .on_press(move || {
                filter.state.revise(|s| {
                    if remote {
                        s.remote_filter_open = !s.remote_filter_open
                    } else {
                        s.local_filter_open = !s.local_filter_open
                    }
                })
            }),
            col().class("pane-menu-anchor").fill((
                icon_action(
                    if remote {
                        "远程文件操作"
                    } else {
                        "本地文件操作"
                    },
                    Icon::More,
                )
                .on_press(move || {
                    menu.state.revise(|s| {
                        s.host_menu = None;
                        s.group_menu = None;
                        s.file_menu = if s.file_menu == Some(remote) {
                            None
                        } else {
                            Some(remote)
                        }
                    })
                }),
                col()
                    .class(state.map(move |s| {
                        if s.file_menu == Some(remote) {
                            "pane-menu"
                        } else {
                            "hidden"
                        }
                        .into()
                    }))
                    .fill((
                        sapid::button()
                            .class("menu-item")
                            .label(i18n::bind("新建文件夹"))
                            .enabled(state.map(move |s| ready(s, remote)))
                            .on_press(move || begin_file(&mkdir, remote, false)),
                        sapid::button()
                            .class("menu-item")
                            .label(i18n::bind("重命名"))
                            .enabled(
                                state.map(move |s| {
                                    ready(s, remote) && selected(s, remote).len() == 1
                                }),
                            )
                            .on_press(move || begin_file(&rename, remote, true)),
                    )),
            )),
        )),
        row()
            .class(state.map(move |s| {
                if if remote {
                    s.remote_filter_open
                } else {
                    s.local_filter_open
                } {
                    "filter-row"
                } else {
                    "hidden"
                }
                .into()
            }))
            .fill((
                input()
                    .class("filter-input")
                    .name(i18n::bind(if remote {
                        "筛选远程文件"
                    } else {
                        "筛选本地文件"
                    }))
                    .value(state.map(move |s| {
                        if remote {
                            s.remote_filter.clone()
                        } else {
                            s.local_filter.clone()
                        }
                    }))
                    .on(value_changed, move |e| {
                        if let Event::ValueChanged { value, .. } = e {
                            state.revise(|s| {
                                selection(s, remote).clear();
                                if remote {
                                    s.remote_filter = value
                                } else {
                                    s.local_filter = value
                                }
                            });
                        }
                    }),
                text().class("subtle").content(i18n::bind("按文件名筛选")),
            )),
        row().class("file-header").fill((
            sort_button(ui.clone(), remote, Sort::Name, "名称", "column-name"),
            sort_button(ui.clone(), remote, Sort::Size, "大小", "column-size"),
            sort_button(
                ui.clone(),
                remote,
                Sort::Modified,
                "修改时间",
                "column-modified",
            ),
        )),
        file_browser::listing(ui.clone(), remote),
        text()
            .class(state.map(move |s| {
                if pane_entries(s, remote).is_empty() {
                    "file-empty"
                } else {
                    "hidden"
                }
                .into()
            }))
            .content(i18n::bind(state.map(move |s| {
                if remote && !s.connected {
                    "在右上角连接 SSH 后，即可浏览 SFTP 文件".into()
                } else if remote && !s.sftp_available {
                    "SSH 已连接，但 SFTP 不可用（服务器未启用或当前账号无访问权限）。系统软件、资源概览和系统日志仍可使用。".into()
                } else if !ready(s, remote) {
                    "输入路径后按 Enter".into()
                } else if !(if remote {
                    &s.remote_filter
                } else {
                    &s.local_filter
                })
                .is_empty()
                {
                    "没有匹配的项目".into()
                } else {
                    "此文件夹为空".into()
                }
            }))),
        file_browser::information(ui.clone(), remote),
    ))
}

pub(super) fn file_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| matches!(s.dialog, Some(Dialog::File { .. }))),
        move || {
            let (remote, rename) = ui.state.with(|s| match s.dialog {
                Some(Dialog::File { remote, rename }) => (remote, rename),
                _ => (false, false),
            });
            let submit = ui.clone();
            let enter = ui.clone();
            let cancel = ui.clone();
            modal(
                ui.clone(),
                if rename {
                    "重命名"
                } else {
                    "新建文件夹"
                },
                (
                    text()
                        .class("dialog-description")
                        .content(i18n::bind(state.map(move |s| {
                            let chosen = selected(s, remote).iter().next();
                            let directory = if rename {
                                chosen.map(|from| {
                                    if remote {
                                        from.rsplit_once('/')
                                            .map(
                                                |(parent, _)| {
                                                    if parent.is_empty() { "/" } else { parent }
                                                },
                                            )
                                            .unwrap_or(".")
                                            .to_owned()
                                    } else {
                                        Path::new(from)
                                            .parent()
                                            .map(local_display)
                                            .unwrap_or_else(|| s.local_path.clone())
                                    }
                                })
                            } else {
                                None
                            };
                            if remote {
                                format!(
                                    "远程目录：{}",
                                    directory.as_deref().unwrap_or(&s.remote_path)
                                )
                            } else {
                                format!(
                                    "本地目录：{}",
                                    directory.as_deref().unwrap_or(&s.local_path)
                                )
                            }
                        }))),
                    col().class("field").fill((
                        text().class("field-label").content(i18n::bind(if rename {
                            "新名称"
                        } else {
                            "文件夹名称"
                        })),
                        input()
                            .name(i18n::bind(if rename {
                                "新名称"
                            } else {
                                "文件夹名称"
                            }))
                            .value(state.map(|s| s.name.clone()))
                            .on(value_changed, move |e| {
                                if let Event::ValueChanged { value, .. } = e {
                                    state.revise(|s| {
                                        s.name = value;
                                        s.dialog_error.clear();
                                    });
                                }
                            })
                            .on(commit, move |_| {
                                if enter.state.with(|s| s.file_pending.is_none()) {
                                    enter.mutate_file(remote, rename)
                                }
                            }),
                    )),
                ),
                (
                    sapid::button()
                        .class("button")
                        .label(i18n::bind("取消"))
                        .enabled(state.map(|s| s.file_pending.is_none()))
                        .on_press(move || cancel.close_dialog()),
                    sapid::button()
                        .class("button primary")
                        .label(i18n::bind(state.map(move |s| {
                            if s.file_pending.is_some() {
                                "正在处理…"
                            } else if rename {
                                "重命名"
                            } else {
                                "创建文件夹"
                            }
                            .to_string()
                        })))
                        .enabled(state.map(|s| s.file_pending.is_none()))
                        .on_press(move || submit.mutate_file(remote, rename)),
                ),
            )
        },
    )
}

fn queue_toggle(ui: Ui, paused: bool) -> sapid::ActionIcon {
    icon_action(
        if paused {
            "继续队列"
        } else {
            "暂停队列"
        },
        if paused { Icon::Play } else { Icon::Pause },
    )
    .on_press(move || {
        ui.state.revise(|s| s.queue_paused = !s.queue_paused);
        ui.schedule();
    })
}
fn dock_toggle(ui: Ui, open: bool) -> sapid::ActionIcon {
    icon_action(
        if open {
            "收起活动面板"
        } else {
            "展开活动面板"
        },
        if open { Icon::Collapse } else { Icon::Expand },
    )
    .on_press(move || ui.state.revise(|s| s.activity_open = !s.activity_open))
}
fn queue(ui: Ui) -> impl Widget {
    let state = ui.state;
    let pause = ui.clone();
    let retry = ui.clone();
    let clear = ui.clone();
    col().class("queue-content").fill((
        row().class("queue-toolbar").fill((
            row().class("queue-filters").fill((
                queue_filter(ui.clone(), 0, "全部"),
                queue_filter(ui.clone(), 1, "进行中"),
                queue_filter(ui.clone(), 2, "失败 / 取消"),
                queue_filter(ui.clone(), 3, "已完成"),
            )),
            when(state.map(|s| s.queue_paused), {
                let pause = pause.clone();
                move || queue_toggle(pause.clone(), true)
            })
            .otherwise(move || queue_toggle(pause.clone(), false)),
            icon_action("重试失败项", Icon::Refresh)
                .enabled(state.map(|s| {
                    s.connected
                        && s.jobs.iter().any(|j| {
                            matches!(
                                j.status,
                                TransferStatus::Failed(_) | TransferStatus::Cancelled
                            )
                        })
                }))
                .on_press(move || retry.retry(None)),
            icon_action("清除已结束的传输", Icon::Clear).on_press(move || {
                clear.state.revise(|s| {
                    s.jobs.retain(|j| {
                        matches!(j.status, TransferStatus::Running | TransferStatus::Queued)
                    });
                    s.transfer_meters
                        .retain(|id, _| s.jobs.iter().any(|j| &j.id == id));
                })
            }),
        )),
        col().class("queue-list").fill(
            each(state.map(|s| {
                s.jobs
                    .iter()
                    .filter(|j| browser::queue_visible(&j.status, s.queue_filter))
                    .cloned()
                    .collect::<Vec<_>>()
            }))
            .key(|j| j.id.clone())
            .children({
                let ui = ui.clone();
                move |item| {
                    let job = item.value;
                    let cancel = ui.clone();
                    let retry = ui.clone();
                    row().class("queue-row").fill((
                        text().class("job-name").content(job.map(|j| {
                            format!(
                                "{} {}",
                                if j.direction == TransferDirection::Upload {
                                    "↑"
                                } else {
                                    "↓"
                                },
                                j.local_path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                            )
                        })),
                        col().class("job-transfer").fill((
                            row().class("job-progress-heading").fill((
                                text().class("job-status").content(i18n::bind(state.map(
                                    move |s| {
                                        let j = job.get();
                                        match &j.status {
                                            TransferStatus::Queued => "等待中".into(),
                                            TransferStatus::Running
                                                if s.transfer_conflict
                                                    .as_ref()
                                                    .is_some_and(|c| c.job_id == j.id) =>
                                            {
                                                "等待文件冲突处理".into()
                                            }
                                            TransferStatus::Running if j.total_bytes == 0 => {
                                                "正在准备传输…".into()
                                            }
                                            TransferStatus::Running
                                                if j.transferred_bytes >= j.total_bytes =>
                                            {
                                                "正在完成传输…".into()
                                            }
                                            TransferStatus::Running => format!(
                                                "{} / {}",
                                                transfer_progress::bytes(j.transferred_bytes),
                                                transfer_progress::bytes(j.total_bytes)
                                            ),
                                            TransferStatus::Completed => {
                                                match s.transfer_summaries.get(&j.id) {
                                                    Some(report)
                                                        if report.transferred_files == 0
                                                            && report.skipped_files > 0 =>
                                                    {
                                                        format!(
                                                            "已跳过 {} 个文件",
                                                            report.skipped_files
                                                        )
                                                    }
                                                    Some(report) if report.skipped_files > 0 => {
                                                        format!(
                                                            "已完成 · 跳过 {} 个文件",
                                                            report.skipped_files
                                                        )
                                                    }
                                                    _ => "已完成".into(),
                                                }
                                            }
                                            TransferStatus::Cancelled => "已取消".into(),
                                            TransferStatus::Failed(e) => format!("失败：{e}"),
                                        }
                                    },
                                ))),
                                text().class("job-percent").content(
                                    job.map(|j| format!("{:.1}%", transfer_progress::percent(j))),
                                ),
                            )),
                            col()
                                .class(job.map(|j| {
                                    match j.status {
                                        TransferStatus::Failed(_) => "job-progress failed",
                                        TransferStatus::Cancelled => "job-progress cancelled",
                                        _ => "job-progress",
                                    }
                                    .to_owned()
                                }))
                                .name(i18n::bind("传输进度"))
                                .role(glory::render::Role::ProgressBar)
                                .semantics(job.map(|j| {
                                    Some(Box::new(glory::render::Semantics {
                                        progress: Some(glory::render::ProgressValue {
                                            min: 0.,
                                            max: 100.,
                                            current: Some(transfer_progress::percent(j)),
                                            value_text: None,
                                        }),
                                        ..Default::default()
                                    }))
                                }))
                                .fill(
                                    col()
                                        .style(job.map(|j| {
                                            glory::style::Style::new().width(
                                                glory::style::Length::Percent(
                                                    transfer_progress::percent(j),
                                                ),
                                            )
                                        }))
                                        .class("job-progress-fill"),
                                ),
                            text().class("job-transfer-details").content(
                                state.map(move |s| transfer_progress::details(s, &job.get())),
                            ),
                        )),
                        close_action("取消传输")
                            .enabled(job.map(|j| {
                                matches!(j.status, TransferStatus::Running | TransferStatus::Queued)
                            }))
                            .on_press(move || {
                                cancel.state.revise(|s| {
                                    let id = job.get().id;
                                    if let Some(token) = s.cancels.get(&id) {
                                        token.store(true, Ordering::Relaxed);
                                    }
                                    if let Some(j) = s.jobs.iter_mut().find(|j| j.id == id)
                                        && j.status == TransferStatus::Queued
                                    {
                                        j.status = TransferStatus::Cancelled;
                                    }
                                });
                                cancel.schedule();
                            }),
                        icon_action("重试传输", Icon::Refresh)
                            .enabled(state.map(move |s| {
                                let j = job.get();
                                s.connected
                                    && Some(&j.server_id) == s.selected.as_ref()
                                    && !s.cancels.contains_key(&j.id)
                                    && matches!(
                                        j.status,
                                        TransferStatus::Failed(_) | TransferStatus::Cancelled
                                    )
                            }))
                            .on_press(move || retry.retry(Some(job.get().id))),
                    ))
                }
            }),
        ),
        text()
            .class(state.map(|s| {
                if s.jobs
                    .iter()
                    .any(|j| browser::queue_visible(&j.status, s.queue_filter))
                {
                    "hidden"
                } else {
                    "queue-empty"
                }
                .into()
            }))
            .content(i18n::bind("暂无传输任务")),
    ))
}
fn queue_filter(ui: Ui, index: u8, label: &'static str) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(state.map(move |s| {
            if s.queue_filter == index {
                "quiet-button active"
            } else {
                "quiet-button"
            }
            .to_string()
        }))
        .label(i18n::bind(label))
        .on_press(move || ui.state.revise(|s| s.queue_filter = index))
}
fn tab(ui: Ui, index: u8, label: &'static str) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(state.map(move |s| {
            if s.activity_tab == index && s.activity_open {
                "dock-tab active"
            } else {
                "dock-tab"
            }
            .into()
        }))
        .label(i18n::bind(state.map(move |s| {
            if index == 0 {
                format!("传输 ({})", s.jobs.len())
            } else {
                label.into()
            }
        })))
        .on_press(move || {
            ui.state.revise(|s| {
                s.activity_tab = index;
                s.activity_open = true;
            })
        })
}
pub(super) fn activity(ui: Ui) -> impl Widget {
    let state = ui.state;
    let toggle = ui.clone();
    let logs = ui.clone();
    let queue_ui = ui.clone();
    col()
        .class(state.map(|s| {
            if s.activity_open {
                "activity-dock"
            } else {
                "activity-dock collapsed"
            }
            .into()
        }))
        .fill((
            row().class("dock-header").fill((
                row()
                    .class("dock-tabs")
                    .fill((tab(ui.clone(), 0, "传输"), tab(ui.clone(), 1, "操作日志"))),
                when(state.map(|s| s.activity_open), {
                    let toggle = toggle.clone();
                    move || dock_toggle(toggle.clone(), true)
                })
                .otherwise(move || dock_toggle(toggle.clone(), false)),
            )),
            col()
                .class(state.map(|s| {
                    if s.activity_open {
                        "dock-content"
                    } else {
                        "hidden"
                    }
                    .into()
                }))
                .fill((
                    when(state.map(|s| s.activity_tab == 0), move || {
                        queue(queue_ui.clone())
                    }),
                    when(state.map(|s| s.activity_tab == 1), move || {
                        text()
                            .class("logs")
                            .selectable(true)
                            .content(logs.state.map(|s| s.logs.clone()))
                    }),
                )),
        ))
}
