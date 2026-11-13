use std::collections::VecDeque;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use super::*;
use crate::terminal_worker::{Action as TerminalAction, Event as TerminalEvent};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Idle,
    Starting,
    Running,
    Exited,
    Failed,
}
#[derive(Clone, Serialize)]
pub struct Packet {
    id: u64,
    data: String,
    #[serde(skip)]
    bytes: usize,
}
#[derive(Clone, Default)]
pub struct TerminalState {
    pub id: u64,
    pub status: Status,
    pub error: String,
    pub exit_code: Option<i32>,
    packets: VecDeque<Packet>,
    sequence: u64,
    clear: u64,
    paste: String,
    paste_revision: u64,
    selected: bool,
    input_ack: u64,
    copied: bool,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Input {
    Ready { columns: u32, rows: u32 },
    Data { id: u64, data: String },
    Binary { id: u64, data: String },
    Resize { id: u64, columns: u32, rows: u32 },
    Consumed { id: u64, packet: u64 },
    Selection { id: u64, selected: bool },
    Copy { id: u64, text: String },
    Paste { id: u64 },
    Pasted { id: u64, revision: u64 },
}
#[derive(Serialize)]
struct Metadata<'a> {
    epoch: u64,
    id: u64,
    status: Status,
    clear: u64,
    paste: &'a str,
    paste_revision: u64,
    input_ack: u64,
}
impl Ui {
    pub(super) fn terminal_enter(&self) {
        if self.state.with(|s| {
            navigation::workspace_visible(s)
                && s.workspace_page == WorkspacePage::Terminal
                && s.terminal.status == Status::Idle
        }) {
            self.terminal_start(100, 30);
        }
    }
    fn terminal_start(&self, columns: u32, rows: u32) {
        if !self.state.with(|s| s.connected) {
            return;
        }
        let id = self.state.revise(|s| {
            let id = s.terminal.id + 1;
            s.terminal = TerminalState {
                id,
                status: Status::Starting,
                ..Default::default()
            };
            id
        });
        self.send(Action::Terminal(TerminalAction::Open {
            id,
            columns: columns.clamp(2, 500),
            rows: rows.clamp(1, 300),
        }));
    }
}
pub(super) fn disconnected(ui: &Ui) {
    ui.state.revise(|s| s.terminal = TerminalState::default());
    ui.send(Action::Terminal(TerminalAction::Detach));
}
pub(super) fn update(ui: &Ui, event: TerminalEvent) {
    match event {
        TerminalEvent::Started { id } => ui.state.revise(|s| {
            if s.terminal.id == id && s.terminal.status == Status::Starting {
                s.terminal.status = Status::Running;
            }
        }),
        TerminalEvent::Output { id, bytes } => ui.state.revise(|s| {
            if s.terminal.id != id {
                return;
            }
            s.terminal.sequence += 1;
            s.terminal.packets.push_back(Packet {
                id: s.terminal.sequence,
                data: STANDARD.encode(&bytes),
                bytes: bytes.len(),
            });
        }),
        TerminalEvent::Exited { id, code } => ui.state.revise(|s| {
            if s.terminal.id == id {
                s.terminal.status = Status::Exited;
                s.terminal.exit_code = code;
            }
        }),
        TerminalEvent::Failed { id, message } => ui.state.revise(|s| {
            if s.terminal.id == id {
                s.terminal.status = Status::Failed;
                s.terminal.error = message;
            }
        }),
    }
}
fn handle_input(ui: &Ui, json: String) {
    if json.len() > 400_000 || !ui.state.with(|s| s.connected) {
        return;
    }
    let Ok(envelope) = serde_json::from_str::<serde_json::Value>(&json) else {
        return;
    };
    let token = envelope["token"].as_u64().unwrap_or(0);
    if !ui.state.with(|s| {
        envelope["epoch"].as_u64() == Some(s.epoch)
            && envelope["session"].as_u64() == Some(s.terminal.id)
            && token > s.terminal.input_ack
    }) {
        return;
    }
    let Ok(event) = serde_json::from_value::<Input>(envelope) else {
        return;
    };
    let (current, running) = ui
        .state
        .with(|s| (s.terminal.id, s.terminal.status == Status::Running));
    match event {
        Input::Ready { columns, rows } => {
            if ui.state.with(|s| s.terminal.status == Status::Idle) {
                ui.terminal_start(columns, rows);
            } else if running {
                ui.send(Action::Terminal(TerminalAction::Resize {
                    id: current,
                    columns: columns.clamp(2, 500),
                    rows: rows.clamp(1, 300),
                }));
            }
        }
        Input::Data { id, data } if id == current && running && data.len() <= 16_384 => {
            ui.send(Action::Terminal(TerminalAction::Input {
                id,
                bytes: data.into_bytes(),
            }));
        }
        Input::Binary { id, data } if id == current && running && data.len() <= 16_384 => {
            if let Ok(bytes) = STANDARD.decode(data) {
                ui.send(Action::Terminal(TerminalAction::Input { id, bytes }));
            }
        }
        Input::Resize { id, columns, rows } if id == current && running => {
            ui.send(Action::Terminal(TerminalAction::Resize {
                id,
                columns: columns.clamp(2, 500),
                rows: rows.clamp(1, 300),
            }));
        }
        Input::Consumed { id, packet } if id == current => {
            let bytes = ui.state.revise(|s| {
                let mut bytes = 0;
                while s.terminal.packets.front().is_some_and(|p| p.id <= packet) {
                    bytes += s.terminal.packets.pop_front().unwrap().bytes;
                }
                bytes
            });
            if bytes != 0 {
                ui.send(Action::Terminal(TerminalAction::Consumed { id, bytes }));
            }
        }
        Input::Selection { id, selected } if id == current => ui.state.revise(|s| {
            s.terminal.selected = selected;
            s.terminal.copied = false;
        }),
        Input::Copy { id, text } if id == current && text.len() <= 65_536 => {
            match clipboard::copy(&text) {
                Ok(()) => ui.state.revise(|s| s.terminal.copied = true),
                Err(error) => ui.error(format!("Cannot copy terminal selection: {error:#}")),
            }
        }
        Input::Copy { id, .. } if id == current => {
            ui.error("Terminal selection is too large to copy (64 KiB maximum)")
        }
        Input::Paste { id } if id == current && running => match clipboard::read(256 * 1024) {
            Ok(text) => ui.state.revise(|s| {
                s.terminal.paste_revision += 1;
                s.terminal.paste = text;
            }),
            Err(error) => ui.error(format!("Cannot paste into terminal: {error:#}")),
        },
        Input::Pasted { id, revision } if id == current => ui.state.revise(|s| {
            if s.terminal.paste_revision == revision {
                s.terminal.paste.clear();
            }
        }),
        _ => {}
    }
    ui.state.revise(|s| s.terminal.input_ack = token);
}
pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let bridge = ui.clone();
    let start = ui.clone();
    let close = ui.clone();
    let clear = ui;
    col()
        .class(state.map(|s| {
            if navigation::workspace_visible(s) && s.workspace_page == WorkspacePage::Terminal {
                "terminal-module"
            } else {
                "terminal-module hidden"
            }
            .to_owned()
        }))
        .fill((
            row().class("terminal-toolbar").fill((
                text().class("terminal-heading").content(i18n::bind("终端")),
                text()
                    .class("terminal-status")
                    .content(i18n::bind(state.map(|s| {
                        if !s.connected {
                            return "连接服务器后使用终端".into();
                        }
                        match s.terminal.status {
                            Status::Idle => "尚未启动".into(),
                            Status::Starting => "正在启动终端…".into(),
                            Status::Running => "交互式 SSH".into(),
                            Status::Exited => s
                                .terminal
                                .exit_code
                                .map(|code| format!("终端已退出（{code}）"))
                                .unwrap_or_else(|| "终端已关闭".into()),
                            Status::Failed => "终端连接失败".into(),
                        }
                    }))),
                icon_action("复制选中文本", Icon::Copy)
                    .class(state.map(|s| {
                        if s.terminal.copied {
                            "icon-button terminal-copy copied"
                        } else {
                            "icon-button terminal-copy"
                        }
                        .to_owned()
                    }))
                    .enabled(state.map(|s| s.terminal.selected)),
                icon_action("粘贴到终端", Icon::Paste)
                    .class("icon-button terminal-paste")
                    .enabled(state.map(|s| s.terminal.status == Status::Running)),
                icon_action("清空终端显示", Icon::Clear)
                    .enabled(state.map(|s| s.terminal.id > 0))
                    .on_press(move || clear.state.revise(|s| s.terminal.clear += 1)),
                icon_action("重新启动终端", Icon::Refresh)
                    .class("icon-button terminal-restart")
                    .enabled(state.map(|s| s.connected && s.terminal.status != Status::Starting))
                    .on_press(move || start.terminal_start(100, 30)),
                close_action("关闭终端")
                    .class("icon-button terminal-close")
                    .enabled(
                        state.map(|s| {
                            matches!(s.terminal.status, Status::Starting | Status::Running)
                        }),
                    )
                    .on_press(move || {
                        let id = close.state.revise(|s| {
                            s.terminal.status = Status::Exited;
                            s.terminal.id
                        });
                        close.send(Action::Terminal(TerminalAction::Close { id }));
                    }),
            )),
            text()
                .class(state.map(|s| {
                    if s.terminal.error.is_empty() {
                        "hidden"
                    } else {
                        "terminal-error"
                    }
                    .to_owned()
                }))
                .content(state.map(|s| s.terminal.error.clone())),
            input()
                .class("terminal-input-bridge")
                .value("")
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        handle_input(&bridge, value);
                    }
                }),
            text().class("terminal-state").content(state.map(|s| {
                serde_json::to_string(&Metadata {
                    epoch: s.epoch,
                    id: s.terminal.id,
                    status: s.terminal.status,
                    clear: s.terminal.clear,
                    paste: &s.terminal.paste,
                    paste_revision: s.terminal.paste_revision,
                    input_ack: s.terminal.input_ack,
                })
                .unwrap_or_default()
            })),
            col().class("terminal-packets").fill(
                each(state.map(|s| s.terminal.packets.iter().cloned().collect::<Vec<_>>()))
                    .key(|p| p.id)
                    .children(|item| {
                        text().class("terminal-packet").content(
                            item.value
                                .map(|packet| serde_json::to_string(packet).unwrap_or_default()),
                        )
                    }),
            ),
            view()
                .class("terminal-surface")
                .name(i18n::bind("交互式 SSH 终端")),
            text().class("terminal-hint").content(i18n::bind(
                "Ctrl+C 中断 · Ctrl+Shift+C 复制 · Ctrl+Shift+V 粘贴",
            )),
        ))
}
