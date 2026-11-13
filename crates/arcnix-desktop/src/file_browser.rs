use std::path::Component;

use glory::style::{Edges, Style};
use serde::Deserialize;

use super::*;

#[derive(Clone, Default)]
pub(super) struct PaneState {
    root: String,
    anchor: Option<String>,
    expanded: HashSet<String>,
    children: HashMap<String, Vec<FileEntry>>,
    pending: HashMap<String, u64>,
    pub editing: bool,
    draft: String,
    copied: bool,
}

#[derive(Clone, Default)]
pub(super) struct BrowserState {
    local: PaneState,
    remote: PaneState,
    local_icons: bool,
    remote_icons: bool,
}
impl BrowserState {
    fn pane(&self, remote: bool) -> &PaneState {
        if remote { &self.remote } else { &self.local }
    }
    fn pane_mut(&mut self, remote: bool) -> &mut PaneState {
        if remote {
            &mut self.remote
        } else {
            &mut self.local
        }
    }
    pub fn reset(&mut self, remote: bool) {
        *self.pane_mut(remote) = PaneState::default();
    }
}

#[derive(Clone)]
pub(super) struct FileRow {
    pub entry: FileEntry,
    depth: usize,
    expanded: bool,
    loading: bool,
}
pub(super) fn icons(s: &State, remote: bool) -> bool {
    if remote {
        s.file_browser.remote_icons
    } else {
        s.file_browser.local_icons
    }
}
pub(super) fn prune_selection(s: &mut State, remote: bool) {
    let visible: HashSet<_> = entries(s, remote).into_iter().map(|f| f.path).collect();
    selection(s, remote).retain(|p| visible.contains(p));
    if s.file_browser
        .pane(remote)
        .anchor
        .as_ref()
        .is_some_and(|p| !visible.contains(p))
    {
        s.file_browser.pane_mut(remote).anchor = None;
    }
}
pub(super) fn view_controls(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let list = ui.clone();
    let grid = ui;
    row().class("view-switch").fill((
        icon_action("详细列表", Icon::List)
            .class(state.map(move |s| {
                if icons(s, remote) {
                    "icon-button view-list"
                } else {
                    "icon-button view-list active"
                }
                .to_owned()
            }))
            .on_press(move || {
                list.state.revise(|s| {
                    if remote {
                        s.file_browser.remote_icons = false;
                    } else {
                        s.file_browser.local_icons = false;
                    }
                })
            }),
        icon_action("平铺图标", Icon::Grid)
            .class(state.map(move |s| {
                if icons(s, remote) {
                    "icon-button view-grid active"
                } else {
                    "icon-button view-grid"
                }
                .to_owned()
            }))
            .on_press(move || {
                grid.state.revise(|s| {
                    if remote {
                        s.file_browser.remote_icons = true;
                    } else {
                        s.file_browser.local_icons = true;
                    }
                    prune_selection(s, remote);
                })
            }),
    ))
}
fn file_kind(entry: &FileEntry) -> String {
    if entry.is_symlink {
        i18n::translate("链接")
    } else if entry.is_dir {
        i18n::translate("文件夹")
    } else if let Some(extension) = Path::new(&entry.name).extension().and_then(|e| e.to_str()) {
        format!("{} {}", extension.to_uppercase(), i18n::translate("文件"))
    } else {
        i18n::translate("文件")
    }
}
fn timestamp(value: Option<u64>) -> String {
    value
        .and_then(|v| i64::try_from(v).ok())
        .and_then(|v| time::OffsetDateTime::from_unix_timestamp(v).ok())
        .map(|date| {
            date.to_offset(time::UtcOffset::local_offset_at(date).unwrap_or(time::UtcOffset::UTC))
        })
        .and_then(|date| {
            date.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .map(|value| value.replace('T', " "))
        .unwrap_or_else(|| "—".into())
}
pub(super) fn information(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    col().class("file-information").fill((
        text()
            .class("file-information-title")
            .content(state.map(move |s| {
                let all = entries(s, remote);
                let selected = if remote {
                    &s.remote_selection
                } else {
                    &s.local_selection
                };
                let chosen: Vec<_> = all.iter().filter(|f| selected.contains(&f.path)).collect();
                if chosen.len() == 1 {
                    let file = chosen[0];
                    if file.is_dir {
                        format!("{} · {}", file.name, file_kind(file))
                    } else {
                        format!(
                            "{} · {} · {} ({} {})",
                            file.name,
                            file_kind(file),
                            size(file.size),
                            file.size,
                            i18n::translate("字节")
                        )
                    }
                } else if !chosen.is_empty() {
                    let bytes = chosen
                        .iter()
                        .filter(|f| !f.is_dir)
                        .fold(0u64, |total, f| total.saturating_add(f.size));
                    format!(
                        "{} · {}",
                        i18n::translate(&format!("已选 {} 项", chosen.len())),
                        size(bytes)
                    )
                } else {
                    i18n::translate(&format!("{} 个项目", all.len()))
                }
            })),
        text()
            .class("file-information-metadata")
            .content(state.map(move |s| {
                let selected = if remote {
                    &s.remote_selection
                } else {
                    &s.local_selection
                };
                let chosen: Vec<_> = entries(s, remote)
                    .into_iter()
                    .filter(|f| selected.contains(&f.path))
                    .collect();
                if chosen.len() != 1 {
                    return String::new();
                }
                let file = &chosen[0];
                let mut value = format!(
                    "{}: {}",
                    i18n::translate("修改时间"),
                    timestamp(file.modified)
                );
                if let Some(mode) = file.permissions {
                    value.push_str(&format!(
                        " · {}: {:04o}",
                        i18n::translate("权限"),
                        mode & 0o7777
                    ));
                }
                value
            })),
        text()
            .class("file-information-path")
            .content(state.map(move |s| {
                let selected = if remote {
                    &s.remote_selection
                } else {
                    &s.local_selection
                };
                let chosen: Vec<_> = entries(s, remote)
                    .into_iter()
                    .filter(|f| selected.contains(&f.path))
                    .collect();
                if chosen.len() == 1 {
                    if remote {
                        chosen[0].path.clone()
                    } else {
                        local_display(Path::new(&chosen[0].path))
                    }
                } else {
                    String::new()
                }
            })),
    ))
}
fn path(s: &State, remote: bool) -> &str {
    if remote {
        &s.remote_path
    } else {
        &s.local_path
    }
}
fn selection(s: &mut State, remote: bool) -> &mut HashSet<String> {
    if remote {
        &mut s.remote_selection
    } else {
        &mut s.local_selection
    }
}
fn sorted(s: &State, remote: bool, entries: &[FileEntry]) -> Vec<FileEntry> {
    let (query, sort, descending) = if remote {
        (&s.remote_filter, s.remote_sort, s.remote_desc)
    } else {
        (&s.local_filter, s.local_sort, s.local_desc)
    };
    visible(entries, query, sort, descending)
        .into_iter()
        .filter(|f| s.config.settings.show_hidden_files || !f.name.starts_with('.'))
        .collect()
}
pub(super) fn rows(s: &State, remote: bool) -> Vec<FileRow> {
    fn append(
        s: &State,
        remote: bool,
        entries: &[FileEntry],
        depth: usize,
        out: &mut Vec<FileRow>,
    ) {
        if depth > 32 || out.len() >= 20_000 {
            return;
        }
        let pane = s.file_browser.pane(remote);
        for entry in sorted(s, remote, entries) {
            if out.len() >= 20_000 {
                break;
            }
            let expanded = pane.root == path(s, remote) && pane.expanded.contains(&entry.path);
            out.push(FileRow {
                depth,
                expanded,
                loading: pane.pending.contains_key(&entry.path),
                entry: entry.clone(),
            });
            if expanded
                && !icons(s, remote)
                && let Some(children) = pane.children.get(&entry.path)
            {
                append(s, remote, children, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    append(
        s,
        remote,
        if remote { &s.remote } else { &s.local },
        0,
        &mut out,
    );
    out
}
pub(super) fn entries(s: &State, remote: bool) -> Vec<FileEntry> {
    rows(s, remote).into_iter().map(|row| row.entry).collect()
}
pub(super) fn selected_entries(s: &State, remote: bool) -> Vec<FileEntry> {
    let selected = if remote {
        &s.remote_selection
    } else {
        &s.local_selection
    };
    let all = entries(s, remote);
    transfer_roots(&all, selected, remote)
}
fn transfer_roots(all: &[FileEntry], selected: &HashSet<String>, remote: bool) -> Vec<FileEntry> {
    let directories: HashSet<_> = all
        .iter()
        .filter(|f| f.is_dir && !f.is_symlink && selected.contains(&f.path))
        .map(|f| f.path.as_str())
        .collect();
    all.iter()
        .filter(|entry| selected.contains(&entry.path) && !entry.is_symlink)
        .filter(|entry| {
            if remote {
                !entry
                    .path
                    .match_indices('/')
                    .any(|(index, _)| directories.contains(&entry.path[..index]))
            } else {
                !Path::new(&entry.path)
                    .ancestors()
                    .skip(1)
                    .filter_map(Path::to_str)
                    .any(|parent| directories.contains(parent))
            }
        })
        .cloned()
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum GestureKind {
    Select,
    Activate,
    Expand,
    Collapse,
    All,
    Clear,
}
#[derive(Deserialize)]
struct Gesture {
    kind: GestureKind,
    #[serde(default)]
    path: String,
    #[serde(default)]
    toggle: bool,
    #[serde(default)]
    range: bool,
}
pub(super) fn select(ui: &Ui, remote: bool, target: &str, toggle: bool, range: bool) {
    ui.state.revise(|s| {
        let visible = entries(s, remote);
        let Some(index) = visible.iter().position(|f| f.path == target) else {
            return;
        };
        let anchor = s.file_browser.pane(remote).anchor.clone();
        let anchor_index = anchor
            .as_ref()
            .and_then(|anchor| visible.iter().position(|f| &f.path == anchor));
        let start = anchor_index.unwrap_or(index);
        let selected = selection(s, remote);
        if range {
            if !toggle {
                selected.clear();
            }
            selected.extend(
                visible[start.min(index)..=start.max(index)]
                    .iter()
                    .map(|f| f.path.clone()),
            );
        } else if toggle {
            if !selected.remove(target) {
                selected.insert(target.to_owned());
            }
        } else {
            selected.clear();
            selected.insert(target.to_owned());
        }
        if !range || anchor_index.is_none() {
            s.file_browser.pane_mut(remote).anchor = Some(target.to_owned());
        }
    });
}
fn navigate(ui: &Ui, remote: bool, target: String) {
    ui.state.revise(|s| {
        if remote {
            s.remote_path = target;
        } else {
            s.local_path = target;
        }
    });
    ui.refresh(remote);
}
fn activate(ui: &Ui, remote: bool, target: &str) {
    let entry = ui.state.with(|s| {
        entries(s, remote)
            .into_iter()
            .find(|f| f.path == target && !f.is_symlink)
    });
    if let Some(entry) = entry {
        if entry.is_dir {
            navigate(ui, remote, entry.path);
        } else {
            select(ui, remote, &entry.path, false, false);
        }
    }
}
fn toggle_tree(ui: &Ui, remote: bool, target: &str, force: Option<bool>) {
    let request = ui.state.revise(|s| {
        let visible = rows(s, remote);
        let row = visible
            .iter()
            .find(|row| row.entry.path == target && row.entry.is_dir && !row.entry.is_symlink)?;
        if row.depth >= 32 {
            return None;
        }
        let root = path(s, remote).to_owned();
        let pane = s.file_browser.pane_mut(remote);
        if pane.root != root {
            *pane = PaneState {
                root,
                ..Default::default()
            };
        }
        let expand = force.unwrap_or(!pane.expanded.contains(target));
        if !expand {
            pane.expanded.remove(target);
            pane.pending.remove(target);
        } else {
            pane.expanded.insert(target.to_owned());
        }
        // A collapsed branch must not retain invisible selected descendants.
        let visible: HashSet<_> = entries(s, remote).into_iter().map(|f| f.path).collect();
        selection(s, remote).retain(|p| visible.contains(p));
        if s.file_browser
            .pane(remote)
            .anchor
            .as_ref()
            .is_some_and(|p| !visible.contains(p))
        {
            s.file_browser.pane_mut(remote).anchor = None;
        }
        let pane = s.file_browser.pane(remote);
        (expand && !pane.children.contains_key(target) && !pane.pending.contains_key(target))
            .then(|| target.to_owned())
    });
    if let Some(target) = request {
        let serial = ui.send(Action::Children {
            remote,
            path: target.clone(),
        });
        ui.state.revise(|s| {
            s.file_browser
                .pane_mut(remote)
                .pending
                .insert(target, serial);
        });
    }
}
pub(super) fn accept_children(
    ui: &Ui,
    remote: bool,
    parent: String,
    serial: u64,
    result: std::result::Result<Vec<FileEntry>, String>,
) {
    let error = ui.state.revise(|s| {
        let root = path(s, remote).to_owned();
        let pane = s.file_browser.pane_mut(remote);
        if pane.root != root || pane.pending.get(&parent) != Some(&serial) {
            return None;
        }
        pane.pending.remove(&parent);
        match result {
            Ok(entries) => {
                pane.children.insert(parent, entries);
                None
            }
            Err(error) => {
                pane.expanded.remove(&parent);
                Some(error)
            }
        }
    });
    if let Some(error) = error {
        ui.error(format!("读取目录失败：{error}"));
    }
}
fn gesture(ui: &Ui, remote: bool, value: String) {
    if value.len() > 32_768 {
        return;
    }
    let Ok(event) = serde_json::from_str::<Gesture>(&value) else {
        return;
    };
    match event.kind {
        GestureKind::Select => select(ui, remote, &event.path, event.toggle, event.range),
        GestureKind::Activate => activate(ui, remote, &event.path),
        GestureKind::Expand => toggle_tree(ui, remote, &event.path, Some(true)),
        GestureKind::Collapse => toggle_tree(ui, remote, &event.path, Some(false)),
        GestureKind::All => ui.state.revise(|s| {
            *selection(s, remote) = entries(s, remote).into_iter().map(|f| f.path).collect();
        }),
        GestureKind::Clear => ui.state.revise(|s| {
            selection(s, remote).clear();
        }),
    }
}

pub(super) fn listing(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let input_ui = ui.clone();
    col()
        .class(state.map(move |s| {
            if rows(s, remote).is_empty() {
                "file-list-container file-list-empty"
            } else {
                "file-list-container"
            }
            .to_owned()
        }))
        .fill((
            // Native pointer modifiers are absent from Glory 0.7's Activate event.
            // This private bridge carries a finite gesture; Rust validates the path
            // against the current visible rows before changing selection/navigation.
            input()
                .class("file-gesture-input")
                .value("")
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        gesture(&input_ui, remote, value);
                    }
                }),
            col().class("file-list").fill(
                each(state.map(move |s| rows(s, remote)))
                    .key(|row| row.entry.path.clone())
                    .children(move |item| {
                        let entry = item.value;
                        let open = ui.clone();
                        let expand = ui.clone();
                        row()
                            .name(entry.map(|r| r.entry.path.clone()))
                            .class(state.map(move |s| {
                                let selected = if remote {
                                    &s.remote_selection
                                } else {
                                    &s.local_selection
                                };
                                if selected.contains(&entry.get().entry.path) {
                                    "file-row row-selected"
                                } else {
                                    "file-row"
                                }
                                .to_owned()
                            }))
                            .style(entry.map(|r| {
                                Style::new().padding(Edges {
                                    left: 10.0 + r.depth as f64 * 18.0,
                                    right: 10.0,
                                    top: 0.0,
                                    bottom: 0.0,
                                })
                            }))
                            .fill((
                                sapid::button()
                                    .class(entry.map(|r| {
                                        if r.entry.is_dir {
                                            "file-expander"
                                        } else {
                                            "file-expander file-expander-empty"
                                        }
                                        .to_owned()
                                    }))
                                    .name(i18n::bind(entry.map(|r| {
                                        format!(
                                            "{} {}",
                                            if r.expanded { "收起" } else { "展开" },
                                            r.entry.name
                                        )
                                    })))
                                    .label(entry.map(|r| {
                                        if r.loading {
                                            "…"
                                        } else if r.expanded {
                                            "⌄"
                                        } else {
                                            "›"
                                        }
                                        .to_owned()
                                    }))
                                    .enabled(entry.map(|r| r.entry.is_dir && !r.entry.is_symlink))
                                    .on_press(move || {
                                        toggle_tree(&expand, remote, &entry.get().entry.path, None)
                                    }),
                                sapid::button()
                                    .class("file-name")
                                    .name(entry.map(|r| r.entry.name.clone()))
                                    .label("")
                                    .fill((
                                        text()
                                            .class(entry.map(|r| {
                                                if r.entry.is_dir {
                                                    "action-glyph file-entry-icon glyph-folder"
                                                } else {
                                                    "action-glyph file-entry-icon glyph-file"
                                                }
                                                .to_owned()
                                            }))
                                            .content(""),
                                        text()
                                            .class("file-label")
                                            .content(entry.map(|r| r.entry.name.clone())),
                                    ))
                                    .on_press(move || {
                                        activate(&open, remote, &entry.get().entry.path)
                                    }),
                                text().class("file-size").content(i18n::bind(entry.map(|r| {
                                    if r.entry.is_dir {
                                        "文件夹".into()
                                    } else if r.entry.is_symlink {
                                        "链接".into()
                                    } else {
                                        size(r.entry.size)
                                    }
                                }))),
                                text()
                                    .class("file-modified")
                                    .content(entry.map(|r| modified(r.entry.modified))),
                            ))
                    }),
            ),
        ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Crumb {
    label: String,
    path: String,
}
fn breadcrumbs(path: &str, remote: bool) -> Vec<Crumb> {
    let mut result: Vec<Crumb> = Vec::new();
    if remote {
        let mut current = if path.starts_with('/') {
            result.push(Crumb {
                label: "/".into(),
                path: "/".into(),
            });
            "/".to_owned()
        } else {
            String::new()
        };
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if !current.is_empty() && current != "/" {
                current.push('/');
            }
            current.push_str(part);
            result.push(Crumb {
                label: part.into(),
                path: current.clone(),
            });
        }
    } else {
        let mut current = PathBuf::new();
        for component in Path::new(path).components() {
            current.push(component.as_os_str());
            if let Component::RootDir = component
                && let Some(previous) = result.last_mut()
            {
                previous.path = local_display(&current);
                continue;
            }
            let target = local_display(&current);
            let label = match component {
                Component::Prefix(_) | Component::RootDir => target.clone(),
                _ => component.as_os_str().to_string_lossy().into_owned(),
            };
            result.push(Crumb {
                label,
                path: target,
            });
        }
    }
    if result.is_empty() {
        result.push(Crumb {
            label: ".".into(),
            path: ".".into(),
        });
    }
    result
}
pub(super) fn pathbar(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let edit_ui = ui.clone();
    let enter_ui = ui.clone();
    let cancel_ui = ui.clone();
    let up_ui = ui.clone();
    let refresh_ui = ui.clone();
    let copy_ui = ui.clone();
    let crumb_ui = ui.clone();
    let enabled = state.map(move |s| !remote || s.connected && s.sftp_available);
    row().class("pathbar").fill((
        icon_action(
            if remote {
                "远程上级目录"
            } else {
                "本地上级目录"
            },
            Icon::Up,
        )
        .enabled(enabled)
        .on_press(move || {
            let target = up_ui.state.with(|s| {
                if remote {
                    let p = s.remote_path.trim_end_matches('/');
                    p.rsplit_once('/')
                        .map(|(parent, _)| {
                            if parent.is_empty() {
                                "/".into()
                            } else {
                                parent.to_owned()
                            }
                        })
                        .unwrap_or_else(|| ".".into())
                } else {
                    Path::new(&s.local_path)
                        .parent()
                        .map(local_display)
                        .unwrap_or_else(|| s.local_path.clone())
                }
            });
            navigate(&up_ui, remote, target);
        }),
        col().class("path-location").fill((
            row()
                .class(state.map(move |s| {
                    if s.file_browser.pane(remote).editing {
                        "breadcrumbs hidden"
                    } else {
                        "breadcrumbs"
                    }
                    .to_owned()
                }))
                .name(i18n::bind(if remote {
                    "远程路径"
                } else {
                    "本地路径"
                }))
                .fill(
                    each(state.map(move |s| breadcrumbs(path(s, remote), remote)))
                        .key(|c| c.path.clone())
                        .children(move |item| {
                            let crumb = item.value;
                            let click = crumb_ui.clone();
                            sapid::button()
                                .class(state.map(move |s| {
                                    if path(s, remote) == crumb.get().path {
                                        "breadcrumb current"
                                    } else {
                                        "breadcrumb"
                                    }
                                    .to_owned()
                                }))
                                .label(crumb.map(|c| c.label.clone()))
                                .name(crumb.map(|c| c.path.clone()))
                                .enabled(enabled)
                                .on_press(move || navigate(&click, remote, crumb.get().path))
                        }),
                ),
            input()
                .class(state.map(move |s| {
                    if s.file_browser.pane(remote).editing {
                        "path-input"
                    } else {
                        "path-input hidden"
                    }
                    .to_owned()
                }))
                .name(i18n::bind(if remote {
                    "远程路径"
                } else {
                    "本地路径"
                }))
                .value(state.map(move |s| s.file_browser.pane(remote).draft.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| s.file_browser.pane_mut(remote).draft = value);
                    }
                })
                .on(commit, move |_| {
                    let target = enter_ui
                        .state
                        .with(|s| s.file_browser.pane(remote).draft.clone());
                    navigate(&enter_ui, remote, target);
                })
                .on(keyboard, move |event| {
                    if let Event::Keyboard {
                        command: glory::render::KeyboardCommand::Dismiss,
                        ..
                    } = event
                    {
                        cancel_ui
                            .state
                            .revise(|s| s.file_browser.pane_mut(remote).editing = false);
                    }
                }),
        )),
        icon_action(
            if remote {
                "编辑远程路径"
            } else {
                "编辑本地路径"
            },
            Icon::Edit,
        )
        .class("icon-button path-edit")
        .on_press(move || {
            edit_ui.state.revise(|s| {
                let current = path(s, remote).to_owned();
                let pane = s.file_browser.pane_mut(remote);
                pane.draft = current;
                pane.editing = !pane.editing;
            });
        }),
        icon_action(
            if remote {
                "复制远程路径"
            } else {
                "复制本地路径"
            },
            Icon::Copy,
        )
        .class(state.map(move |s| {
            if s.file_browser.pane(remote).copied {
                "icon-button path-copy copied"
            } else {
                "icon-button path-copy"
            }
            .to_owned()
        }))
        .on_press(move || {
            let value = copy_ui.state.with(|s| path(s, remote).to_owned());
            match crate::clipboard::copy(&value) {
                Ok(()) => copy_ui
                    .state
                    .revise(|s| s.file_browser.pane_mut(remote).copied = true),
                Err(error) => copy_ui.error(format!("复制路径失败：{error}")),
            }
        }),
        icon_action(
            if remote {
                "刷新远程目录"
            } else {
                "刷新本地目录"
            },
            Icon::Refresh,
        )
        .enabled(state.map(move |s| !remote || s.connected && s.sftp_available))
        .on_press(move || refresh_ui.refresh(remote)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_selection_transfers_parent_once_without_confusing_sibling_prefixes() {
        let entries: Vec<_> = [
            ("/a", true),
            ("/a/child", false),
            ("/ab/file", false),
            ("/a\\b/file", false),
        ]
        .into_iter()
        .map(|(path, is_dir)| FileEntry {
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            is_dir,
            is_symlink: false,
            size: 4,
            modified: None,
            permissions: None,
        })
        .collect();
        let selected = entries.iter().map(|f| f.path.clone()).collect();
        let result = transfer_roots(&entries, &selected, true);
        assert_eq!(
            result.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["/a", "/ab/file", "/a\\b/file"]
        );
    }
    #[test]
    fn posix_breadcrumbs_keep_ancestor_targets() {
        let parts = breadcrumbs("/srv/my data/中文/", true);
        assert_eq!(
            parts.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            ["/", "/srv", "/srv/my data", "/srv/my data/中文"]
        );
        assert_eq!(breadcrumbs("/", true)[0].path, "/");
        assert_eq!(breadcrumbs(".", true)[0].path, ".");
    }
    #[cfg(windows)]
    #[test]
    fn windows_breadcrumbs_preserve_drive_and_share_roots() {
        let parts = breadcrumbs(r"D:\Works\中文", false);
        assert_eq!(
            parts.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            [r"D:\", r"D:\Works", r"D:\Works\中文"]
        );
        let parts = breadcrumbs(r"\\server\share\folder", false);
        assert_eq!(parts[0].path, r"\\server\share\");
        assert_eq!(parts.last().unwrap().path, r"\\server\share\folder");
    }
}
