//! The words rustv shows, in every language it speaks.
//!
//! One catalog per language (`en.toml`, `tr.toml` next to this crate), keyed
//! by stable ids such as `setting.sleep.after_minutes.label`. tvd, the TV shell
//! and the web remote (which gets them from tvd) all read the same ones, so a
//! label is translated once. English is the fallback for a key another
//! language lacks, and the tests below keep every language complete.
//!
//! Adding a language: copy `en.toml`, translate it, and add it to [`LANGUAGES`]
//! and [`SOURCES`].

use std::collections::BTreeMap;
use std::sync::LazyLock;

/// Language codes, English first.
pub const LANGUAGES: &[&str] = &["en", "tr"];

/// The catalogs, built into every binary that uses them.
const SOURCES: &[(&str, &str)] = &[("en", include_str!("../en.toml")), ("tr", include_str!("../tr.toml"))];

type Catalog = BTreeMap<String, String>;

static CATALOGS: LazyLock<BTreeMap<&'static str, Catalog>> = LazyLock::new(|| {
    SOURCES.iter().map(|(lang, text)| (*lang, parse(text).unwrap_or_else(|err| panic!("{lang}.toml: {err}")))).collect()
});

/// `a.b = "x"` and `[a] b = "x"` both become `"a.b" -> "x"`.
fn parse(text: &str) -> Result<Catalog, String> {
    let table: toml::Table = text.parse().map_err(|err: toml::de::Error| err.to_string())?;
    let mut out = Catalog::new();
    flatten("", &toml::Value::Table(table), &mut out)?;
    Ok(out)
}

fn flatten(prefix: &str, value: &toml::Value, out: &mut Catalog) -> Result<(), String> {
    match value {
        toml::Value::String(text) => {
            out.insert(prefix.to_string(), text.clone());
            Ok(())
        }
        toml::Value::Table(table) => {
            for (key, value) in table {
                let key = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
                flatten(&key, value, out)?;
            }
            Ok(())
        }
        _ => Err(format!("{prefix}: only text is allowed")),
    }
}

fn catalog(lang: &str) -> &'static Catalog {
    CATALOGS.get(lang).unwrap_or(&CATALOGS["en"])
}

/// The text for `key`: in `lang`, else in English, else the key itself (so a
/// missing one shows up instead of vanishing).
pub fn text<'a>(lang: &str, key: &'a str) -> &'a str
where
    'static: 'a,
{
    catalog(lang).get(key).or_else(|| CATALOGS["en"].get(key)).map_or(key, String::as_str)
}

/// The text for `key` with `{name}` placeholders filled in. A numeric `n`
/// picks a plural form when the key has them (`key.one`, `key.other`).
pub fn format(lang: &str, key: &str, args: &[(&str, &str)]) -> String {
    let count = args.iter().find(|(name, _)| *name == "n").and_then(|(_, value)| value.parse::<f64>().ok());
    let plural = count.map(|n| format!("{key}.{}", plural_form(lang, n)));
    let template = match &plural {
        Some(form) if has(lang, form) || has("en", form) => text(lang, form),
        _ => text(lang, key),
    };
    let mut out = template.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn has(lang: &str, key: &str) -> bool {
    CATALOGS.get(lang).is_some_and(|catalog| catalog.contains_key(key))
}

/// The CLDR plural category for `n`: English says "1 day" but "2 days";
/// Turkish uses one form after a number.
pub fn plural_form(lang: &str, n: f64) -> &'static str {
    match lang {
        "tr" => "other",
        _ if n == 1.0 => "one",
        _ => "other",
    }
}

/// Every catalog as JSON, `{"en": {...}, "tr": {...}}`, for the web remote.
pub fn catalogs_json() -> String {
    serde_json::to_string(&*CATALOGS).expect("catalogs are plain text")
}

/// The `{placeholders}` a text uses, sorted.
pub fn placeholders(text: &str) -> Vec<&str> {
    let mut found: Vec<&str> = text
        .split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name))
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn every_language_has_every_key_with_the_same_placeholders() {
        let en = catalog("en");
        for lang in LANGUAGES {
            let other = &CATALOGS[lang];
            let missing: Vec<&String> = en.keys().filter(|key| !other.contains_key(*key)).collect();
            let extra: Vec<&String> = other.keys().filter(|key| !en.contains_key(*key)).collect();
            assert!(missing.is_empty(), "{lang}.toml lacks {missing:?}");
            assert!(extra.is_empty(), "{lang}.toml has keys en.toml does not: {extra:?}");
            for (key, text) in en {
                assert_eq!(placeholders(text), placeholders(&other[key]), "{lang}: {key}");
                assert!(!other[key].trim().is_empty(), "{lang}: {key} is empty");
            }
        }
    }

    #[test]
    fn fills_in_falls_back_and_picks_plurals() {
        assert_eq!(format("tr", "unit.minutes", &[("n", "15")]), "15 dk");
        assert_eq!(format("en", "unit.days", &[("n", "1")]), "1 day");
        assert_eq!(format("en", "unit.days", &[("n", "2")]), "2 days");
        assert_eq!(format("tr", "unit.days", &[("n", "2")]), "2 gün");
        assert_eq!(text("xx", "common.close"), "Close");
        assert_eq!(text("tr", "no.such.key"), "no.such.key");
        assert!(catalogs_json().contains("\"tr\""));
    }

    /// Every key the code spells out must exist: catches a typo or a key
    /// removed from the catalog but still used.
    #[test]
    fn every_key_used_in_the_code_exists() {
        // Distinct from config sections (`audio.step`, `keyboard.enabled`), which look alike.
        const GROUPS: &[&str] = &[
            "common", "error", "network", "option", "osk", "output", "power", "remote", "section", "setting", "stats",
            "tv", "unit",
        ];
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        collect(&root, &mut files);
        let en = catalog("en");
        let mut unknown = Vec::new();
        let mut used = 0;
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            for literal in text.split('"').skip(1).step_by(2) {
                let Some((group, rest)) = literal.split_once('.') else { continue };
                if !GROUPS.contains(&group)
                    || rest.is_empty()
                    || !literal.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_')
                {
                    continue;
                }
                // A prefix the code completes at run time, such as `option.theme.preset.`.
                if literal.ends_with('.') {
                    assert!(
                        en.keys().any(|key| key.starts_with(literal)),
                        "{}: no keys under {literal}",
                        file.display()
                    );
                    continue;
                }
                used += 1;
                if !en.contains_key(literal) && !has("en", &format!("{literal}.one")) {
                    unknown.push(format!("{}: {literal}", file.display()));
                }
            }
        }
        // The remote builds setting keys with label("sleep.enabled") and hint("sleep.enabled").
        for file in files.iter().filter(|file| file.extension().is_some_and(|ext| ext == "js")) {
            let text = std::fs::read_to_string(file).unwrap();
            for (call, suffix) in [("label(\"", "label"), ("hint(\"", "hint")] {
                for rest in text.split(call).skip(1) {
                    let Some((setting, _)) = rest.split_once('"') else { continue };
                    let key = format!("setting.{setting}.{suffix}");
                    used += 1;
                    if !en.contains_key(&key) {
                        unknown.push(format!("{}: {key}", file.display()));
                    }
                }
            }
        }
        assert!(used > 100, "only {used} keys found; is the scan looking in the right place?");
        assert!(unknown.is_empty(), "keys missing from en.toml:\n{}", unknown.join("\n"));
    }

    fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy();
            if path.is_dir() && name != "target" && name != "node_modules" && name != "locales" {
                collect(&path, out);
            } else if ["rs", "js", "html", "slint"].iter().any(|ext| name.ends_with(&format!(".{ext}"))) {
                out.push(path);
            }
        }
    }
}
