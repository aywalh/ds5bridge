//! Settings in %APPDATA%\DS5 Bridge\settings.ini (plain key=value). Stays on this PC.

use std::path::PathBuf;

#[derive(Clone, PartialEq)]
pub struct Settings {
    pub lang: Option<String>, // None = follow Windows
    pub deadzone: u32,        // per mille
    pub check_updates: bool,
    pub last_check: u64,      // unix seconds
}

impl Default for Settings {
    fn default() -> Self {
        Settings { lang: None, deadzone: 80, check_updates: true, last_check: 0 }
    }
}

pub fn dir() -> PathBuf {
    crate::installer::known_folder(crate::installer::FOLDER_ROAMING).join("DS5 Bridge")
}

fn file() -> PathBuf {
    dir().join("settings.ini")
}

pub fn load() -> Settings {
    let mut s = Settings::default();
    let text = std::fs::read_to_string(file()).unwrap_or_default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "lang" if !v.is_empty() => s.lang = Some(v.to_string()),
            "deadzone" => s.deadzone = v.parse().unwrap_or(s.deadzone).min(300),
            "check_updates" => s.check_updates = v != "0",
            "last_check" => s.last_check = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    s
}

pub fn save(s: &Settings) {
    let text = format!(
        "lang={}\ndeadzone={}\ncheck_updates={}\nlast_check={}\n",
        s.lang.as_deref().unwrap_or(""),
        s.deadzone,
        s.check_updates as u8,
        s.last_check
    );
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(file(), text);
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
