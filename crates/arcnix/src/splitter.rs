//! Captured pointer resizing supported by the Wry host; all state stays in Rust.
use std::cell::RefCell;

use glory::render::{DragCapture, DragPhase, Navigation};
use glory::style::{Length, Style};

use super::*;

const HANDLE: f64 = 10.0;
const MIN_PANE: f64 = 260.0;

fn slot_style(ratio: f64) -> Style {
    Style {
        grow: Some(ratio),
        shrink: Some(1.0),
        ..Style::new().width(Length::Px(0.0))
    }
}

fn bounded_ratio(ratio: f64, width: f64) -> f64 {
    let minimum = if width.is_finite() && width > HANDLE {
        (MIN_PANE / (width - HANDLE)).clamp(0.2, 0.5)
    } else {
        0.2
    };
    if ratio.is_finite() {
        ratio.clamp(minimum, 1.0 - minimum)
    } else {
        0.5
    }
}

struct DragOrigin {
    x: f64,
    ratio: f64,
    original_ratio: f64,
    width: f64,
    server: Option<String>,
}

pub(super) fn panes(
    ui: Ui,
    left: impl Widget + 'static,
    right: impl Widget + 'static,
) -> impl Widget {
    let state = ui.state;
    let width = ui.scope.cage(0.0);
    let origin = Rc::new(RefCell::new(None::<DragOrigin>));
    let resize_origin = origin.clone();
    let dragging = ui.scope.cage(false);
    let drag_ui = ui.clone();
    let resize_ui = ui.clone();
    let reset_ui = ui;
    row()
        .class("panes")
        .on(resize, move |event| {
            if let Event::Resize { size, .. } = event {
                width.revise(|value| *value = size.width);
                // A resized container invalidates the captured geometry. End
                // this gesture instead of allowing a jump on its next move.
                if let Some(initial) = resize_origin.borrow_mut().take()
                    && resize_ui.state.with(|s| s.selected == initial.server)
                {
                    resize_ui
                        .state
                        .revise(|s| s.file_split = initial.original_ratio);
                }
                dragging.revise(|value| *value = false);
            }
        })
        .fill((
            col()
                .class("pane-slot local-slot")
                .style(state.map(move |s| slot_style(bounded_ratio(s.file_split, width.get()))))
                .fill(left),
            view()
                .class(dragging.map(|active| {
                    if *active {
                        "pane-splitter dragging"
                    } else {
                        "pane-splitter"
                    }
                    .to_string()
                }))
                .role(Role::Group)
                .name(i18n::bind("调整文件栏宽度，上下方向键调整，Home 恢复均分"))
                .focus(FocusMode::TabStop)
                .semantics(Some(Box::new(Semantics {
                    drag_capture: Some(DragCapture::Press),
                    ..Default::default()
                })))
                .on(drag, move |event| {
                    let Event::Drag { phase, x, .. } = event else {
                        return;
                    };
                    match phase {
                        DragPhase::Start => {
                            let measured = width.get();
                            if measured <= HANDLE {
                                return;
                            }
                            let (ratio, original_ratio, server) = drag_ui.state.with(|s| {
                                (
                                    bounded_ratio(s.file_split, measured),
                                    s.file_split,
                                    s.selected.clone(),
                                )
                            });
                            *origin.borrow_mut() = Some(DragOrigin {
                                x,
                                ratio,
                                original_ratio,
                                width: measured,
                                server,
                            });
                            dragging.revise(|value| *value = true);
                        }
                        DragPhase::Move | DragPhase::Drop => {
                            let mut start = origin.borrow_mut();
                            let Some(initial) = start.as_ref() else {
                                return;
                            };
                            if drag_ui.state.with(|s| s.selected != initial.server) {
                                start.take();
                                dragging.revise(|value| *value = false);
                                return;
                            }
                            let ratio = bounded_ratio(
                                initial.ratio + (x - initial.x) / (initial.width - HANDLE),
                                initial.width,
                            );
                            drag_ui.state.revise(|s| s.file_split = ratio);
                            if phase == DragPhase::Drop {
                                start.take();
                                dragging.revise(|value| *value = false);
                                drag_ui.save_file_split(ratio);
                            }
                        }
                        DragPhase::Cancel => {
                            if let Some(initial) = origin.borrow_mut().take()
                                && drag_ui.state.with(|s| s.selected == initial.server)
                            {
                                drag_ui
                                    .state
                                    .revise(|s| s.file_split = initial.original_ratio);
                            }
                            dragging.revise(|value| *value = false);
                        }
                    }
                })
                .on(navigate, move |event| {
                    if let Event::Navigate { command, .. } = event {
                        let ratio = reset_ui.state.with(|s| s.file_split);
                        let ratio = match command {
                            Navigation::Previous => ratio - 0.05,
                            Navigation::Next => ratio + 0.05,
                            Navigation::First => 0.5,
                            _ => return,
                        };
                        reset_ui.save_file_split(bounded_ratio(ratio, width.get()));
                    }
                }),
            col()
                .class("pane-slot remote-slot")
                .style(
                    state.map(move |s| slot_style(1.0 - bounded_ratio(s.file_split, width.get()))),
                )
                .fill(right),
        ))
}
