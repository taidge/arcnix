use arcnix_core::transfer::{ConflictAction, TransferConflict};
use worker::{ConflictDecision, ConflictScope};

use super::*;

#[derive(Clone)]
pub(super) struct PendingConflict {
    pub job_id: String,
    pub info: TransferConflict,
    pub reply: Sender<ConflictDecision>,
    pub action: ConflictAction,
    pub scope: ConflictScope,
}

pub(super) fn show(
    ui: &Ui,
    job_id: String,
    info: TransferConflict,
    reply: Sender<ConflictDecision>,
) {
    ui.state.revise(|s| {
        s.transfer_now = std::time::Instant::now();
        if let Some(m) = s.transfer_meters.get_mut(&job_id) {
            m.pause(s.transfer_now);
        }
        s.transfer_conflict = Some(PendingConflict {
            job_id,
            info,
            reply,
            action: ConflictAction::Skip,
            scope: ConflictScope::ThisFile,
        });
    });
    if ui.state.with(|s| s.dialog.is_none()) {
        ui.open_dialog(Dialog::TransferConflict);
    }
}

impl Ui {
    pub(super) fn resolve_conflict(&self) {
        let Some(pending) = self.state.revise(|s| s.transfer_conflict.take()) else {
            return;
        };
        let decision = ConflictDecision {
            action: pending.action,
            scope: pending.scope,
        };
        self.state.revise(|s| {
            s.transfer_now = std::time::Instant::now();
            if let Some(m) = s.transfer_meters.get_mut(&pending.job_id) {
                m.resume(s.transfer_now);
            }
        });
        self.apply_conflict_rule(&pending.job_id, pending.info.direction, decision);
        if pending.reply.send(decision).is_err() {
            self.log("文件冲突任务已结束，未继续传输。");
        }
        self.close_dialog();
    }
}

fn size(bytes: u64) -> String {
    let (unit, divisor) = if bytes >= 1024 * 1024 * 1024 {
        ("GiB", 1024.0 * 1024.0 * 1024.0)
    } else if bytes >= 1024 * 1024 {
        ("MiB", 1024.0 * 1024.0)
    } else if bytes >= 1024 {
        ("KiB", 1024.0)
    } else {
        return i18n::translate(&format!("{bytes} 字节"));
    };
    i18n::translate(&format!(
        "{:.2} {unit}（{bytes} 字节）",
        bytes as f64 / divisor
    ))
}

fn modified(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds.filter(|seconds| *seconds <= 253_402_300_799) else {
        return i18n::translate("修改时间未知");
    };
    let z = (seconds / 86400) as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    i18n::translate(&format!(
        "修改于 {year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    ))
}

fn action_choice(ui: Ui, action: ConflictAction, label: &'static str) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(state.map(move |s| {
            if s.transfer_conflict
                .as_ref()
                .is_some_and(|c| c.action == action)
            {
                "choice active"
            } else {
                "choice"
            }
            .to_string()
        }))
        .name(i18n::bind(format!("冲突处理：{label}")))
        .label(i18n::bind(label))
        .on_press(move || {
            state.revise(|s| {
                if let Some(conflict) = &mut s.transfer_conflict {
                    conflict.action = action;
                }
            })
        })
}

fn scope_choice(ui: Ui, scope: ConflictScope, label: &'static str) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(state.map(move |s| {
            if s.transfer_conflict
                .as_ref()
                .is_some_and(|c| c.scope == scope)
            {
                "choice active"
            } else {
                "choice"
            }
            .to_string()
        }))
        .name(i18n::bind(format!("冲突规则范围：{label}")))
        .label(i18n::bind(label))
        .on_press(move || {
            state.revise(|s| {
                if let Some(conflict) = &mut s.transfer_conflict {
                    conflict.scope = scope;
                }
            })
        })
}

fn file_details(ui: &Ui, source: bool) -> impl Widget + use<> {
    let state = ui.state;
    col().class("conflict-file").fill((
        text().class("field-label").content(i18n::bind(if source {
            "传入文件"
        } else {
            "目标已有文件"
        })),
        text()
            .class("conflict-path")
            .selectable(true)
            .content(state.map(move |s| {
                s.transfer_conflict
                    .as_ref()
                    .map(|c| {
                        let path = if source {
                            &c.info.source_path
                        } else {
                            &c.info.destination_path
                        };
                        let local = source == (c.info.direction == TransferDirection::Upload);
                        if local {
                            local_display(Path::new(path))
                        } else {
                            path.clone()
                        }
                    })
                    .unwrap_or_default()
            })),
        text().class("subtle").content(state.map(move |s| {
            s.transfer_conflict
                .as_ref()
                .map(|c| {
                    let (bytes, time) = if source {
                        (c.info.source_size, c.info.source_modified)
                    } else {
                        (c.info.destination_size, c.info.destination_modified)
                    };
                    format!("{} · {}", size(bytes), modified(time))
                })
                .unwrap_or_default()
        })),
    ))
}

pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::TransferConflict)),
        move || {
            let submit = ui.clone();
            let cancel = ui.clone();
            modal(
            ui.clone(),
            "目标文件已存在",
            col().class("form-stack").fill((
                text().class("dialog-description").content(i18n::bind(state.map(|s| {
                    s.transfer_conflict.as_ref().map(|c| {
                        let location = if c.info.direction == TransferDirection::Upload { "服务器" } else { "本地" };
                        format!("{location}中已有同名文件，请选择如何处理。覆盖会替换目标文件的原有内容。")
                    }).unwrap_or_default()
                }))),
                file_details(&ui, true),
                file_details(&ui, false),
                col().class("field").fill((
                    text().class("field-label").content(i18n::bind("处理方式")),
                    row().class("choice-list").fill((
                        action_choice(ui.clone(), ConflictAction::Overwrite, "覆盖"),
                        action_choice(ui.clone(), ConflictAction::Skip, "跳过"),
                    )),
                )),
                col().class("field").fill((
                    text().class("field-label").content(i18n::bind("应用范围")),
                    row().class("choice-list").fill((
                        scope_choice(ui.clone(), ConflictScope::ThisFile, "仅此文件"),
                        scope_choice(ui.clone(), ConflictScope::ThisQueue, "本次批量传输"),
                        scope_choice(ui.clone(), ConflictScope::ThisSession, "当前连接"),
                        scope_choice(ui.clone(), ConflictScope::FutureConnections, "以后连接此服务器"),
                    )),
                )),
                text().class("subtle").content(i18n::bind(state.map(|s| {
                    s.transfer_conflict.as_ref().map(|c| {
                        let direction = if c.info.direction == TransferDirection::Upload { "上传" } else { "下载" };
                        match c.scope {
                            ConflictScope::ThisFile => "下一次文件冲突仍会询问。".into(),
                            ConflictScope::ThisQueue => format!("应用于本次加入队列的这批{direction}，不影响其他批次。"),
                            ConflictScope::ThisSession => format!("应用于当前服务器连接内的后续{direction}；断开连接后失效。"),
                            ConflictScope::FutureConnections => format!("保存为此服务器的{direction}规则，后续连接也会使用。可在服务器的更多菜单中重置。"),
                        }
                    }).unwrap_or_default()
                }))),
            )),
            (
                btn("取消队列", move || {
                    cancel.cancel_all();
                    cancel.close_dialog();
                }),
                sapid::button()
                    .class(state.map(|s| {
                        if s.transfer_conflict.as_ref().is_some_and(|c| c.action == ConflictAction::Overwrite) {
                            "button danger"
                        } else {
                            "button primary"
                        }.to_string()
                    }))
                    .label(i18n::bind(state.map(|s| {
                        if s.transfer_conflict.as_ref().is_some_and(|c| c.action == ConflictAction::Overwrite) {
                            "覆盖文件"
                        } else {
                            "跳过文件"
                        }.to_string()
                    })))
                    .enabled(state.map(|s| s.transfer_conflict.is_some()))
                    .on_press(move || submit.resolve_conflict()),
            ),
        )
        },
    )
}
