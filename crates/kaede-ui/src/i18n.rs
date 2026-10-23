//! 极简 i18n（不再依赖 dioxus-i18n-json，改用 serde_json）。
//! 早期版本用 dioxus-i18n-json 的 parse_translation_json，但它依赖 dioxus 0.6，
//! 会把 winit 0.30 / tao 0.34 等 30+ crate 拉进 workspace 与 0.7 双版本共存。
//! 支持：扁平化键、变量插值 {name}、Plural（取 other）。
use dioxus::prelude::*;
use serde_json::Value;
use std::collections::HashMap;

type Bundle = HashMap<String, String>;

fn flatten(out: &mut Bundle, prefix: &str, v: &Value) {
    match v {
        Value::Object(map) => {
            if map.contains_key("other") && (map.contains_key("one") || map.contains_key("zero") || map.contains_key("few") || map.contains_key("many") || map.contains_key("two")) {
                if let Some(Value::String(s)) = map.get("other") {
                    out.insert(prefix.to_string(), s.clone());
                }
                return;
            }
            for (k, v) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(out, &key, v);
            }
        }
        Value::String(s) => { out.insert(prefix.to_string(), s.clone()); }
        Value::Number(n) => { out.insert(prefix.to_string(), n.to_string()); }
        Value::Bool(b)   => { out.insert(prefix.to_string(), b.to_string()); }
        _ => {}
    }
}

fn parse_bundle(src: &str, locale: &str) -> Bundle {
    let v: Value = serde_json::from_str(src).unwrap_or_else(|e| panic!("locale {locale}.json 解析失败: {e}"));
    let mut out = HashMap::new();
    flatten(&mut out, "", &v);
    out
}

fn bundles() -> HashMap<String, Bundle> {
    let mut m = HashMap::new();
    m.insert("zh-CN".to_string(), parse_bundle(include_str!("../locales/zh-CN.json"), "zh-CN"));
    m.insert("en".to_string(),    parse_bundle(include_str!("../locales/en.json"),    "en"));
    m
}

#[derive(Clone, Copy)]
pub struct I18n {
    pub locale: Signal<String>,
    pub bundles: Signal<&'static HashMap<String, Bundle>>,
}

impl I18n {
    pub fn new() -> Self {
        let b: &'static HashMap<String, Bundle> = Box::leak(Box::new(bundles()));
        Self { locale: Signal::new("zh-CN".to_string()), bundles: Signal::new(b) }
    }
    pub fn t(&self, key: &str) -> String {
        let b = *self.bundles.read();
        let l = self.locale.read().clone();
        b.get(&l).and_then(|x| x.get(key)).cloned().unwrap_or_else(|| key.to_string())
    }
    pub fn tf(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let mut s = self.t(key);
        for (k, v) in vars { s = s.replace(&format!("{{{k}}}"), v); }
        s
    }
    pub fn set_locale(&mut self, l: &str) { self.locale.set(l.to_string()); }
    pub fn locale(&self) -> String { self.locale.read().clone() }
}

impl Default for I18n { fn default() -> Self { Self::new() } }
pub fn use_i18n() -> I18n { use_context::<I18n>() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flatten_nested() {
        let v: Value = serde_json::from_str(r#"{"a":{"b":"x"},"c":"y"}"#).unwrap();
        let mut o = HashMap::new(); flatten(&mut o, "", &v);
        assert_eq!(o.get("a.b"), Some(&"x".into()));
        assert_eq!(o.get("c"), Some(&"y".into()));
    }
    #[test]
    fn flatten_plural() {
        let v: Value = serde_json::from_str(r#"{"k":{"one":"1","other":"many"}}"#).unwrap();
        let mut o = HashMap::new(); flatten(&mut o, "", &v);
        assert_eq!(o.get("k"), Some(&"many".into()));
    }
}
