//! UI languages. The English text is the key; `lang/<code>.json` maps it to the translation and a
//! missing entry falls back to English, so a new text never breaks a language.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::OnceLock;

/// (code, name in its own language). The index is the language id; 0 = English.
pub const LANGS: [(&str, &str); 8] = [
    ("en", "English"), ("de", "Deutsch"), ("ja", "日本語"), ("zh", "简体中文"),
    ("ko", "한국어"), ("vi", "Tiếng Việt"), ("fil", "Filipino"), ("ar", "العربية"),
];

const FILES: [&str; 8] = [
    "{}",
    include_str!("../lang/de.json"),
    include_str!("../lang/ja.json"),
    include_str!("../lang/zh.json"),
    include_str!("../lang/ko.json"),
    include_str!("../lang/vi.json"),
    include_str!("../lang/fil.json"),
    include_str!("../lang/ar.json"),
];

pub const AR: usize = 7;

static CUR: AtomicUsize = AtomicUsize::new(0);
type Table = HashMap<String, String>;
static RAW: OnceLock<Vec<Table>> = OnceLock::new();
static DRAWN: OnceLock<Vec<Table>> = OnceLock::new();

fn raw() -> &'static [Table] {
    RAW.get_or_init(|| {
        FILES
            .iter()
            .map(|f| {
                crate::json::parse(f)
                    .map(|j| j.obj().iter().filter_map(|(k, v)| Some((k.clone(), v.str()?.to_string()))).collect())
                    .unwrap_or_default()
            })
            .collect()
    })
}

/// Same tables, Arabic put in visual order for egui (which has no bidi).
fn drawn() -> &'static [Table] {
    DRAWN.get_or_init(|| {
        raw().iter().enumerate().map(|(i, t)| if i == AR { t.iter().map(|(k, v)| (k.clone(), rtl(v))).collect() } else { t.clone() }).collect()
    })
}

pub fn cur() -> usize {
    CUR.load(Relaxed)
}

pub fn set(id: usize) {
    CUR.store(id.min(LANGS.len() - 1), Relaxed);
}

pub fn id_of(code: &str) -> Option<usize> {
    LANGS.iter().position(|l| l.0 == code)
}

/// The saved language, else English (the user picks another one in the app).
pub fn init(saved: Option<&str>) {
    set(saved.and_then(id_of).unwrap_or(0));
}

/// A language's own name, ready for egui.
pub fn native(id: usize) -> String {
    if id == AR { rtl(LANGS[id].1) } else { LANGS[id].1.into() }
}

/// Text for egui in the current language.
pub fn tr(en: &str) -> &str {
    drawn().get(cur()).and_then(|t| t.get(en)).map(|s| s.as_str()).unwrap_or(en)
}

/// Text for native Windows controls (tray menu, tooltips): Windows does its own right-to-left.
pub fn tr_native(en: &str) -> &str {
    raw().get(cur()).and_then(|t| t.get(en)).map(|s| s.as_str()).unwrap_or(en)
}

/// Translated text with each `{}` replaced by the next argument.
pub fn trf(en: &str, args: &[&dyn Display]) -> String {
    fill(tr(en), args)
}

pub fn trf_native(en: &str, args: &[&dyn Display]) -> String {
    fill(tr_native(en), args)
}

fn fill(t: &str, args: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut parts = t.split("{}");
    out.push_str(parts.next().unwrap_or(""));
    for (i, p) in parts.enumerate() {
        if let Some(a) = args.get(i) {
            out.push_str(&a.to_string());
        }
        out.push_str(p);
    }
    out
}

/// egui joins Arabic letters and mirrors each Arabic word itself, but lays a line out left to right
/// with no bidi. So: put the line in visual right-to-left order here, then turn each Arabic letter
/// run back to logical order for the shaper.
fn rtl(s: &str) -> String {
    // ponytail: simplified bidi (strong R/L, neutrals between two L stay L, else R), enough for UI labels
    let chars: Vec<char> = s.chars().collect();
    let arabic = |c: char| matches!(c as u32, 0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF);
    let strong: Vec<Option<bool>> = chars.iter().map(|&c| if arabic(c) { Some(true) } else if c.is_alphanumeric() { Some(false) } else { None }).collect();
    let rtl_at = |i: usize| -> bool {
        strong[i].unwrap_or_else(|| {
            let prev = strong[..i].iter().rev().flatten().next();
            let next = strong[i + 1..].iter().flatten().next();
            !(prev == Some(&false) && next == Some(&false))
        })
    };
    let mut runs: Vec<(bool, String)> = vec![];
    for (i, &c) in chars.iter().enumerate() {
        let r = rtl_at(i);
        match runs.last_mut() {
            Some(run) if run.0 == r => run.1.push(c),
            _ => runs.push((r, c.to_string())),
        }
    }
    let mirror = |c: char| match c { '(' => ')', ')' => '(', '[' => ']', ']' => '[', '«' => '»', '»' => '«', c => c };
    let visual: String = runs.iter().rev().map(|(r, t)| if *r { t.chars().rev().map(mirror).collect() } else { t.clone() }).collect();
    let visual = visual.replace("}{", "{}"); // a mirrored placeholder
    let mut out = String::new();
    let mut word = String::new();
    for c in visual.chars() {
        if arabic(c) {
            word.insert(0, c)
        } else {
            out.push_str(&std::mem::take(&mut word));
            out.push(c)
        }
    }
    out + &word
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_load_and_fall_back() {
        for (i, f) in FILES.iter().enumerate() {
            assert!(crate::json::parse(f).is_some(), "lang file {} is not valid JSON", LANGS[i].0);
        }
        set(0);
        assert_eq!(tr("no such text"), "no such text");
        assert_eq!(trf("{} of {}", &[&1, &"x"]), "1 of x");
        set(1);
        assert_eq!(tr("Quit"), "Beenden");
        set(0);
        assert_eq!(rtl("ملف {} ViGEm"), "ViGEm {} ملف");
        assert_eq!(rtl("كلمة أخرى"), "أخرى كلمة");
    }

    /// Every language file only uses keys the code knows, so a typo in a key is caught.
    #[test]
    fn translations_cover_the_same_keys() {
        let de: Vec<&String> = raw()[1].keys().collect();
        for (i, t) in raw().iter().enumerate().skip(2) {
            for k in &de {
                assert!(t.contains_key(*k), "{} misses \"{}\"", LANGS[i].0, k);
            }
        }
    }
}
