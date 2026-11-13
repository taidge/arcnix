//! Sapid action primitives with decorative SVG masks for the Wry host.
//! Wry does not render Glory Graphic yet; CSS masks retain currentColor.
use glory::prelude::*;

use crate::i18n;

#[derive(Clone, Copy)]
pub enum Icon {
    Search,
    ChevronRight,
    Server,
    Bell,
    Paste,
    Folder,
    File,
    Grid,
    List,
    Copy,
    Edit,
    Settings,
    Close,
    More,
    Up,
    Go,
    Refresh,
    Filter,
    Collapse,
    Expand,
    Pause,
    Play,
    Clear,
    Plus,
    Upload,
    Download,
}

impl Icon {
    fn name(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::ChevronRight => "chevron-right",
            Self::Server => "server",
            Self::Bell => "bell",
            Self::Paste => "paste",
            Self::Folder => "folder",
            Self::File => "file",
            Self::Grid => "grid",
            Self::List => "list",
            Self::Copy => "copy",
            Self::Edit => "edit",
            Self::Settings => "settings",
            Self::Close => "close",
            Self::More => "more",
            Self::Up => "up",
            Self::Go => "go",
            Self::Refresh => "refresh",
            Self::Filter => "filter",
            Self::Collapse => "collapse",
            Self::Expand => "expand",
            Self::Pause => "pause",
            Self::Play => "play",
            Self::Clear => "clear",
            Self::Plus => "plus",
            Self::Upload => "upload",
            Self::Download => "download",
        }
    }
    fn svg(self) -> &'static str {
        match self {
            Self::Search => include_str!("../assets/icons/search.svg"),
            Self::ChevronRight => include_str!("../assets/icons/chevron-right.svg"),
            Self::Server => include_str!("../assets/icons/server.svg"),
            Self::Bell => include_str!("../assets/icons/bell.svg"),
            Self::Paste => include_str!("../assets/icons/paste.svg"),
            Self::Folder => include_str!("../assets/icons/folder.svg"),
            Self::File => include_str!("../assets/icons/file.svg"),
            Self::Grid => include_str!("../assets/icons/grid.svg"),
            Self::List => include_str!("../assets/icons/list.svg"),
            Self::Copy => include_str!("../assets/icons/copy.svg"),
            Self::Edit => include_str!("../assets/icons/edit.svg"),
            Self::Settings => include_str!("../assets/icons/settings.svg"),
            Self::Close => include_str!("../assets/icons/close.svg"),
            Self::More => include_str!("../assets/icons/more.svg"),
            Self::Up => include_str!("../assets/icons/arrow-up.svg"),
            Self::Go => include_str!("../assets/icons/arrow-right.svg"),
            Self::Refresh => include_str!("../assets/icons/refresh.svg"),
            Self::Filter => include_str!("../assets/icons/filter.svg"),
            Self::Collapse => include_str!("../assets/icons/chevron-down.svg"),
            Self::Expand => include_str!("../assets/icons/chevron-up.svg"),
            Self::Pause => include_str!("../assets/icons/pause.svg"),
            Self::Play => include_str!("../assets/icons/play.svg"),
            Self::Clear => include_str!("../assets/icons/trash.svg"),
            Self::Plus => include_str!("../assets/icons/plus.svg"),
            Self::Upload => include_str!("../assets/icons/upload.svg"),
            Self::Download => include_str!("../assets/icons/download.svg"),
        }
    }
}

pub(super) fn glyph(icon: Icon) -> impl Widget {
    text()
        .class(format!("action-glyph glyph-{}", icon.name()))
        .content("")
}

pub fn icon_action(name: impl Into<String>, icon: Icon) -> sapid::ActionIcon {
    let name = name.into();
    sapid::action_icon(i18n::translate(&name))
        .icon(glyph(icon))
        .variant(sapid::ActionIconVariant::Subtle)
        .size(sapid::Size::Small)
        .class("icon-button")
}

pub fn close_action(name: impl Into<String>) -> sapid::CloseButton {
    sapid::close_button()
        .name(i18n::translate(&name.into()))
        .icon(glyph(Icon::Close))
        .size(sapid::Size::Small)
        .class("icon-button")
}

pub fn text_action(name: impl Into<String>, icon: Icon) -> sapid::Button {
    let name = name.into();
    sapid::button()
        .name(i18n::bind(name.clone()))
        .class("button with-icon")
        .fill((glyph(icon), text().content(i18n::bind(name))))
}

pub fn css() -> String {
    let mut css = String::new();
    for (selector, svg) in [
        (".brand-icon", include_str!("../assets/arcnix-dark.svg")),
        (
            ".theme-light .brand-icon",
            include_str!("../assets/arcnix-light.svg"),
        ),
    ] {
        let encoded = svg
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        css.push_str(&format!(
            "{selector}{{background-image:url(\"data:image/svg+xml,{encoded}\")}}\n"
        ));
    }
    for icon in [
        Icon::Search,
        Icon::ChevronRight,
        Icon::Server,
        Icon::Bell,
        Icon::Paste,
        Icon::Folder,
        Icon::File,
        Icon::Grid,
        Icon::List,
        Icon::Copy,
        Icon::Edit,
        Icon::Settings,
        Icon::Close,
        Icon::More,
        Icon::Up,
        Icon::Go,
        Icon::Refresh,
        Icon::Filter,
        Icon::Collapse,
        Icon::Expand,
        Icon::Pause,
        Icon::Play,
        Icon::Clear,
        Icon::Plus,
        Icon::Upload,
        Icon::Download,
    ] {
        let encoded = icon
            .svg()
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        css.push_str(&format!(
            ".glyph-{}{{mask-image:url(\"data:image/svg+xml,{encoded}\")}}\n",
            icon.name()
        ));
    }
    css
}
