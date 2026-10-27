//! Errors a person may see, in whatever language their screen speaks.
//!
//! tvd itself only logs in English. An error the remote or the TV shows is a
//! [`Localized`]: a catalog key with its values. Its plain text is the English
//! one (for logs and older remotes); the key and values travel next to it, so
//! each screen words it in its own language.

use std::fmt;

/// A message from the catalog (`crates/locales`), with its `{placeholders}` filled in.
#[derive(Debug, Clone)]
pub struct Localized {
    pub key: &'static str,
    pub args: Vec<(&'static str, String)>,
}

impl fmt::Display for Localized {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(name, value)| (*name, value.as_str())).collect();
        f.write_str(&rustv_locales::format("en", self.key, &args))
    }
}

impl std::error::Error for Localized {}

/// A text tvd hands to the screens to word themselves, such as an audio output's
/// generic name: `{"key": "...", "args": {...}}` next to the English text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct Label {
    pub key: &'static str,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub args: std::collections::BTreeMap<&'static str, String>,
}

impl Label {
    pub fn new(key: &'static str, args: &[(&'static str, String)]) -> Self {
        Self { key, args: args.iter().cloned().collect() }
    }

    /// The English text, for logs and older clients.
    pub fn english(&self) -> String {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(name, value)| (*name, value.as_str())).collect();
        rustv_locales::format("en", self.key, &args)
    }
}

/// An error with this key and these values, for `?` and `bail!`-like returns.
pub fn error(key: &'static str, args: &[(&'static str, String)]) -> anyhow::Error {
    Localized { key, args: args.to_vec() }.into()
}

/// What a client gets for an error: the English text, and the key and values
/// when it is one the catalog knows.
pub fn describe(err: &anyhow::Error) -> serde_json::Value {
    let message = format!("{err:#}");
    match err.chain().find_map(|cause| cause.downcast_ref::<Localized>()) {
        Some(localized) => {
            let args: serde_json::Map<String, serde_json::Value> =
                localized.args.iter().map(|(name, value)| (name.to_string(), value.clone().into())).collect();
            serde_json::json!({ "message": message, "key": localized.key, "args": args })
        }
        None => serde_json::json!({ "message": message }),
    }
}

/// The catalog's code for a configured language.
pub fn code(language: crate::config::Language) -> &'static str {
    match language {
        crate::config::Language::En => "en",
        crate::config::Language::Tr => "tr",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_carry_their_key_and_read_in_english() {
        let err = error("error.pin.wrong_blocked", &[("seconds", "30".into())]);
        assert_eq!(err.to_string(), "Wrong PIN. Entry is blocked for 30 s");
        let body = describe(&err.context("unlocking"));
        assert_eq!(body["key"], "error.pin.wrong_blocked");
        assert_eq!(body["args"]["seconds"], "30");
        assert!(describe(&anyhow::anyhow!("plain")).get("key").is_none());
    }
}
