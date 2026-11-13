//! Rust-side reactive localization; server output and user data are not rewritten.
use std::cell::Cell;

use arcnix_core::Language;
use glory::prelude::*;
thread_local! { static LANGUAGE: Cell<Option<Cage<Language>>> = const { Cell::new(None) }; }
pub fn init(scope: &Scope, language: Language) {
    LANGUAGE.with(|slot| slot.set(Some(scope.cage(language))));
}
pub fn set_language(language: Language) {
    LANGUAGE.with(|slot| {
        if let Some(state) = slot.get() {
            state.revise(|current| *current = language);
        }
    });
}
pub fn bind(value: impl Into<Vary<String>>) -> Vary<String> {
    let value = value.into();
    LANGUAGE.with(|slot| match slot.get() {
        Some(language) => language
            .map(move |language| translate_for(&value.get(), *language))
            .into(),
        None => Vary::Value(translate_for(&value.get(), Language::English)),
    })
}
pub fn translate(value: &str) -> String {
    let language = LANGUAGE.with(|slot| slot.get().map(|l| l.get()).unwrap_or_default());
    translate_for(value, language)
}
fn catalog() -> impl Iterator<Item = (&'static str, &'static str)> {
    include_str!("../assets/translations.tsv")
        .lines()
        .filter_map(|line| line.split_once('\t'))
}
fn parts(pattern: &str) -> (Vec<String>, Vec<String>) {
    let mut literals = Vec::new();
    let mut keys = Vec::new();
    let mut rest = pattern;
    let mut automatic = 0;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start + 1..].find('}') else {
            break;
        };
        let end = start + 1 + end;
        literals.push(rest[..start].to_string());
        let raw = &rest[start + 1..end];
        let key = raw.split(':').next().unwrap_or("");
        keys.push(if key.is_empty() {
            let key = automatic.to_string();
            automatic += 1;
            key
        } else {
            key.to_string()
        });
        rest = &rest[end + 1..];
    }
    literals.push(rest.to_string());
    (literals, keys)
}
fn capture<'a>(text: &'a str, literals: &[String]) -> Option<Vec<&'a str>> {
    let mut rest = text.strip_prefix(literals.first()?.as_str())?;
    let mut captures = Vec::new();
    for (index, delimiter) in literals.iter().enumerate().skip(1) {
        if index == literals.len() - 1 {
            captures.push(rest.strip_suffix(delimiter.as_str())?);
            rest = "";
        } else {
            if delimiter.is_empty() {
                captures.push("");
                continue;
            }
            let end = rest.find(delimiter)?;
            captures.push(&rest[..end]);
            rest = &rest[end + delimiter.len()..];
        }
    }
    rest.is_empty().then_some(captures)
}
fn render(
    pattern: &str,
    keys: &[String],
    values: &[&str],
    language: Language,
    depth: u8,
) -> String {
    let (literals, target_keys) = parts(pattern);
    let mut result = literals[0].clone();
    for (index, key) in target_keys.iter().enumerate() {
        if let Some(at) = keys.iter().position(|source| source == key) {
            if matches!(
                key.as_str(),
                "label" | "direction" | "location" | "e" | "error" | "message" | "warning"
            ) || pattern.contains("logical processors")
                || pattern.contains("Last sampled")
                || pattern.contains("Last read")
                || pattern == "{} · {} items"
            {
                result.push_str(&translate_inner(values[at], language, depth + 1));
            } else {
                result.push_str(values[at]);
            }
        }
        result.push_str(&literals[index + 1]);
    }
    result
}
struct Pattern {
    literals: Vec<String>,
    keys: Vec<String>,
    target: &'static str,
}
struct Catalog {
    exact: std::collections::HashMap<&'static str, &'static str>,
    patterns: Vec<Pattern>,
}
fn selected_catalog(language: Language) -> &'static Catalog {
    static CATALOG: std::sync::OnceLock<[Catalog; 2]> = std::sync::OnceLock::new();
    let catalogs = CATALOG.get_or_init(|| {
        std::array::from_fn(|index| {
            let mut exact = std::collections::HashMap::new();
            let mut patterns = Vec::new();
            for (zh, en) in catalog() {
                let (source, target) = if index == 0 { (zh, en) } else { (en, zh) };
                if source.contains('{') {
                    let (literals, keys) = parts(source);
                    patterns.push(Pattern {
                        literals,
                        keys,
                        target,
                    });
                } else {
                    exact.entry(source).or_insert(target);
                }
            }
            patterns.sort_by_key(|pattern| {
                std::cmp::Reverse(pattern.literals.iter().map(String::len).sum::<usize>())
            });
            Catalog { exact, patterns }
        })
    });
    &catalogs[if language == Language::English { 0 } else { 1 }]
}
pub fn translate_for(value: &str, language: Language) -> String {
    translate_inner(value, language, 0)
}
fn translate_inner(value: &str, language: Language, depth: u8) -> String {
    if depth > 4 || value.is_empty() {
        return value.to_owned();
    }
    if language == Language::English
        && !value
            .chars()
            .any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
    {
        return value.to_owned();
    }
    let catalog = selected_catalog(language);
    if let Some(target) = catalog.exact.get(value) {
        return (*target).to_owned();
    }
    for pattern in &catalog.patterns {
        if let Some(values) = capture(value, &pattern.literals) {
            return render(pattern.target, &pattern.keys, &values, language, depth);
        }
    }
    let trimmed = value.trim();
    if trimmed.len() != value.len() {
        let start = value.find(trimmed).unwrap_or(0);
        return format!(
            "{}{}{}",
            &value[..start],
            translate_inner(trimmed, language, depth + 1),
            &value[start + trimmed.len()..]
        );
    }
    if value.contains('\n') {
        return value
            .split_inclusive('\n')
            .map(|line| translate_inner(line, language, depth + 1))
            .collect();
    }
    value.to_owned()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translates_templates_preserving_values_and_reordering() {
        assert_eq!(
            translate_for("SFTP · 12 个项目", Language::English),
            "SFTP · 12 items"
        );
        assert_eq!(
            translate_for("生产主机的服务器操作", Language::English),
            "Actions for 生产主机"
        );
        assert_eq!(
            translate_for("Actions for 生产主机", Language::SimplifiedChinese),
            "生产主机的服务器操作"
        );
        assert_eq!(
            translate_for("服务器的服务器操作", Language::English),
            "Actions for 服务器"
        );
    }
    #[test]
    fn bound_text_tracks_both_language_and_source_changes() {
        let app = glory::App::memory();
        let scope = app.scope();
        init(scope, Language::English);
        let source = scope.cage("上传".to_string());
        let bound = bind(source);
        assert_eq!(bound.get(), "Upload");
        set_language(Language::SimplifiedChinese);
        assert_eq!(bound.get(), "上传");
        source.revise(|value| *value = "下载".into());
        assert_eq!(bound.get(), "下载");
        set_language(Language::English);
        assert_eq!(bound.get(), "Download");
        LANGUAGE.with(|slot| slot.set(None));
    }
    #[test]
    fn leaves_external_content_and_paths_untouched() {
        for value in ["/var/log/中文.txt", "Accepted key for user", "hello 世界"] {
            assert_eq!(translate_for(value, Language::English), value);
        }
    }
}
