//! Settings in %APPDATA%\DS5 Bridge\settings.ini (plain key=value). Stays on this PC.

use std::path::PathBuf;

#[derive(Clone, PartialEq)]
pub struct Settings {
    pub lang: Option<String>, // None = English
    pub deadzone: u32,        // per mille
    pub check_updates: bool,
    pub last_check: u64,      // unix seconds
    pub accent: u32,          // 0xRRGGBB
    pub sounds: bool,
    pub bridge_enabled: bool,
    pub rate_limit: u32, // Hz, 0 = auto
}

impl Default for Settings {
    fn default() -> Self {
        Settings { lang: None, deadzone: 80, check_updates: true, last_check: 0, accent: 0xC9B37E, sounds: true, bridge_enabled: false, rate_limit: 0 }
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
            "accent" => s.accent = u32::from_str_radix(v, 16).unwrap_or(s.accent) & 0xFF_FFFF,
            "sounds" => s.sounds = v != "0",
            "bridge_enabled" => s.bridge_enabled = v == "1",
            "rate" => s.rate_limit = v.parse().unwrap_or(0).min(1000),
            _ => {}
        }
    }
    s
}

pub fn save(s: &Settings) {
    let text = format!(
        "lang={}\ndeadzone={}\ncheck_updates={}\nlast_check={}\naccent={:06x}\nsounds={}\nrate={}\nbridge_enabled={}\n",
        s.lang.as_deref().unwrap_or(""),
        s.deadzone,
        s.check_updates as u8,
        s.last_check,
        s.accent,
        s.sounds as u8,
        s.rate_limit,
        s.bridge_enabled as u8
    );
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(file(), text);
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
