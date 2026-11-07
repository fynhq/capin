use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Zh,
    En,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::Zh, Self::En];

    pub const fn code(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "zh" => Some(Self::Zh),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Zh => t(self, "language.zh"),
            Self::En => t(self, "language.en"),
        }
    }

    fn strings(self) -> &'static HashMap<&'static str, &'static str> {
        static ZH: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
        static EN: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
        match self {
            Self::Zh => ZH.get_or_init(|| parse(include_str!("../../../locales/zh-CN.conf"))),
            Self::En => EN.get_or_init(|| parse(include_str!("../../../locales/en.conf"))),
        }
    }
}

/// Looks up `key` for `language`. Missing keys fall back to the key itself so
/// omissions are visible during development.
pub fn t(language: Language, key: &str) -> &str {
    language.strings().get(key).copied().unwrap_or(key)
}

fn parse(source: &'static str) -> HashMap<&'static str, &'static str> {
    let mut strings = HashMap::new();
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            strings.insert(key.trim(), value.trim());
        }
    }
    strings
}
