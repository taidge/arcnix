use super::*;

#[derive(Clone, Default)]
pub(super) struct SettingsState {
    pub error: String,
}

impl Ui {
    fn update_preferences(&self, change: impl FnOnce(&mut AppSettings)) {
        let previous_language = self.state.with(|s| s.config.settings.language);
        let mut config = self.state.with(|s| s.config.clone());
        change(&mut config.settings);
        let result = if self.writable {
            self.store.save(&config)
        } else {
            Err(anyhow::anyhow!(
                "Configuration could not be loaded. Repair it before saving settings."
            ))
        };
        match result {
            Ok(()) => {
                let language = config.settings.language;
                self.state.revise(|s| {
                    s.config = config;
                    s.settings.error.clear();
                });
                if language != previous_language {
                    i18n::set_language(language);
                }
            }
            Err(error) => self.state.revise(|s| {
                s.settings.error = format!("Could not save settings: {error:#}");
            }),
        }
    }

    pub(super) fn change_language(&self, language: Language) {
        self.update_preferences(|settings| settings.language = language);
    }

    pub(super) fn change_theme(&self, theme: Theme) {
        self.update_preferences(|settings| settings.theme = theme);
    }

    pub(super) fn change_hidden_files(&self, show: bool) {
        self.update_preferences(|settings| settings.show_hidden_files = show);
        self.state.revise(|s| {
            file_browser::prune_selection(s, false);
            file_browser::prune_selection(s, true);
        });
    }

    pub(super) fn change_resource_interval(&self, seconds: u64) {
        if [3, 5, 10, 30].contains(&seconds) {
            self.update_preferences(|settings| settings.resource_interval_seconds = seconds);
        }
    }
}

fn choice(
    ui: &Ui,
    label: &'static str,
    active: impl Fn(&State) -> bool + 'static,
    action: impl FnMut() + 'static,
) -> sapid::Button {
    sapid::button()
        .class(
            ui.state
                .map(move |s| if active(s) { "choice active" } else { "choice" }.to_string()),
        )
        .label(i18n::bind(label))
        .on_press(action)
}

fn section(
    ui: &Ui,
    category: navigation::SettingsSection,
    title: &'static str,
    label: &'static str,
    description: &'static str,
    controls: impl IntoChildren + 'static,
) -> impl Widget {
    col()
        .class(ui.state.map(move |s| {
            if s.settings_section == category {
                "settings-section"
            } else {
                "settings-section hidden"
            }
            .to_owned()
        }))
        .fill((
            text()
                .class("settings-section-title")
                .content(i18n::bind(title)),
            col().class("settings-field").fill((
                text().class("field-label").content(i18n::bind(label)),
                text().class("subtle").content(i18n::bind(description)),
                row().class("choice-list settings-choices").fill(controls),
            )),
        ))
}

pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let english = ui.clone();
    let chinese = ui.clone();
    let system = ui.clone();
    let dark = ui.clone();
    let light = ui.clone();
    let show = ui.clone();
    let hide = ui.clone();
    let intervals = ui.clone();
    col().class("settings-page").fill((
        col().class("settings-heading").fill((
            text().class("page-title").content(i18n::bind("Application settings")),
            text().class("subtle").content(i18n::bind("Customize this application on your computer.")),
        )),
        text().class(state.map(|s| {
            if s.settings.error.is_empty() { "hidden" } else { "software-error settings-error" }.to_string()
        })).content(i18n::bind(state.map(|s| s.settings.error.clone()))),
        col().class("settings-sections").fill((
            section(&ui, navigation::SettingsSection::General, "General", "Language", "Choose the language used throughout the application.", (
                choice(&ui, "English", |s| s.config.settings.language == Language::English, move || english.change_language(Language::English)),
                choice(&ui, "简体中文", |s| s.config.settings.language == Language::SimplifiedChinese, move || chinese.change_language(Language::SimplifiedChinese)),
            )),
            section(&ui, navigation::SettingsSection::Appearance, "Appearance", "Theme", "Choose a color theme.", (
                choice(&ui, "Follow system", |s| s.config.settings.theme == Theme::System, move || system.change_theme(Theme::System)),
                choice(&ui, "Dark", |s| s.config.settings.theme == Theme::Dark, move || dark.change_theme(Theme::Dark)),
                choice(&ui, "Light", |s| s.config.settings.theme == Theme::Light, move || light.change_theme(Theme::Light)),
            )),
            section(&ui, navigation::SettingsSection::Files, "Files", "Hidden files", "Show or hide dotfiles in local and remote file lists.", (
                choice(&ui, "Show hidden files", |s| s.config.settings.show_hidden_files, move || show.change_hidden_files(true)),
                choice(&ui, "Hide hidden files", |s| !s.config.settings.show_hidden_files, move || hide.change_hidden_files(false)),
            )),
            section(&ui, navigation::SettingsSection::Monitoring, "Monitoring", "Resource refresh interval", "Refresh resource values only while the Resources page is open.",
                each(vec![3u64, 5, 10, 30]).key(|seconds| *seconds).children(move |item| {
                    let seconds = item.value.get();
                    let ui = intervals.clone();
                    sapid::button()
                        .class(state.map(move |s| {
                            if s.config.settings.resource_interval_seconds == seconds { "choice active" } else { "choice" }.to_string()
                        }))
                        .label(i18n::bind(format!("Every {seconds} seconds")))
                        .on_press(move || ui.change_resource_interval(seconds))
                }),
            ),
        )),
        col().class("settings-footer").fill((
            text().class("subtle").content(i18n::bind("Changes are saved automatically.")),
            text().class("subtle").content(i18n::bind("Preferences are saved on this computer. File pane sizes and conflict choices remain separate for each server.")),
        )),
    ))
}
