//! 极简 i18n 封装。
//!
//! 我们复用 `dioxus-i18n-json` 的 JSON 解析器（`parse_translation_json`），
//! 但不用它的 `I18nProvider` 组件（与 Dioxus 0.7.10 的宏不兼容）。
//! 自己提供 `I18n` context + `use_i18n()`。

use dioxus::prelude::*;
use dioxus_i18n_json::{TranslationValue, Translations, parse_translation_json};
use std::collections::HashMap;

/// 编译期嵌入的全部语言包：locale -> 扁平化后的键值表。
fn bundles() -> HashMap<String, Translations> {
    let mut m = HashMap::new();
    m.insert(
        "zh-CN".to_string(),
        parse_translation_json(include_str!("../locales/zh-CN.json")).expect("zh-CN.json 解析失败"),
    );
    m.insert(
        "en".to_string(),
        parse_translation_json(include_str!("../locales/en.json")).expect("en.json 解析失败"),
    );
    m
}

#[derive(Clone, Copy)]
pub struct I18n {
    pub locale: Signal<String>,
    pub bundles: Signal<&'static HashMap<String, Translations>>,
}

impl I18n {
    pub fn new() -> Self {
        let b: &'static HashMap<String, Translations> = Box::leak(Box::new(bundles()));
        Self {
            locale: Signal::new("zh-CN".to_string()),
            bundles: Signal::new(b),
        }
    }

    pub fn t(&self, key: &str) -> String {
        let b = *self.bundles.read();
        let l = self.locale.read().clone();
        resolve(b, &l, key).unwrap_or_else(|| key.to_string())
    }

    pub fn tf(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let mut s = self.t(key);
        for (k, v) in vars {
            s = s.replace(&format!("{{{}}}", k), v);
        }
        s
    }

    pub fn set_locale(&mut self, l: &str) {
        self.locale.set(l.to_string());
    }

    pub fn locale(&self) -> String {
        self.locale.read().clone()
    }
}

impl Default for I18n {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve(bundles: &HashMap<String, Translations>, locale: &str, key: &str) -> Option<String> {
    let table = bundles.get(locale)?;
    match table.get(key)? {
        TranslationValue::String(s) => Some(s.clone()),
        TranslationValue::Plural(p) => p.other.clone(),
    }
}

pub fn use_i18n() -> I18n {
    use_context::<I18n>()
}
