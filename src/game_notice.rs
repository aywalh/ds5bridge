//! Checks the foreground process only while Xbox output is active.
use crate::sys::*;
use std::collections::HashSet;

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    Restored,
    InGame(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppKind {
    Game,
    Other,
    Unknown,
}

#[derive(Default)]
pub struct GameNotice {
    restored: bool,
    foreground: Option<(u32, AppKind)>,
    warned: HashSet<u32>,
}

impl GameNotice {
    pub fn new(restored: bool) -> Self {
        Self {
            restored,
            ..Self::default()
        }
    }

    pub fn poll(&mut self, enabled: bool, output_active: bool) -> Option<Notice> {
        if !enabled {
            self.restored = false;
            self.foreground = None;
            self.warned.clear();
            return None;
        }

        let game = output_active.then(|| self.foreground_game()).flatten();
        self.pending(game)
    }

    fn pending(&self, game: Option<u32>) -> Option<Notice> {
        if let Some(pid) = game.filter(|pid| !self.warned.contains(pid)) {
            Some(Notice::InGame(pid))
        } else if self.restored {
            Some(Notice::Restored)
        } else {
            None
        }
    }

    pub fn delivered(&mut self, notice: Notice) {
        self.restored = false;
        if let Notice::InGame(pid) = notice {
            self.warned.insert(pid);
        }
    }

    fn foreground_game(&mut self) -> Option<u32> {
        let window = unsafe { GetForegroundWindow() };
        if window == 0 {
            return None;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        if pid == 0 || self.warned.contains(&pid) {
            return None;
        }
        let kind = match self.foreground {
            Some((current, kind)) if current == pid => kind,
            _ => {
                let kind = process_image(pid)
                    .as_deref()
                    .map_or(AppKind::Unknown, classify);
                self.foreground = Some((pid, kind));
                kind
            }
        };
        let game = match kind {
            AppKind::Game => true,
            AppKind::Other => false,
            AppKind::Unknown => {
                let mut state = 0;
                (unsafe { SHQueryUserNotificationState(&mut state) == 0 })
                    && state == QUNS_RUNNING_D3D_FULL_SCREEN
            }
        };
        game.then_some(pid)
    }
}

fn process_image(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process == 0 {
        return None;
    }
    let mut path = vec![0u16; 32_768];
    let mut length = path.len() as u32;
    let ok = unsafe {
        let result = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        CloseHandle(process);
        result != 0
    };
    ok.then(|| String::from_utf16_lossy(&path[..length as usize]))
}

fn classify(path: &str) -> AppKind {
    let path = path.to_ascii_lowercase().replace('/', "\\");
    let exe = path.rsplit('\\').next().unwrap_or_default();
    let shared = path.contains("\\steamworks shared\\")
        || path.contains("\\_commonredist\\")
        || path.contains("\\epic games\\launcher\\");
    let launcher = matches!(
        exe,
        "steam.exe"
            | "steamwebhelper.exe"
            | "epicgameslauncher.exe"
            | "epicwebhelper.exe"
            | "galaxyclient.exe"
            | "eadesktop.exe"
            | "ealauncher.exe"
            | "ubisoftconnect.exe"
            | "upc.exe"
            | "battle.net.exe"
            | "riotclientservices.exe"
            | "explorer.exe"
            | "chrome.exe"
            | "msedge.exe"
            | "firefox.exe"
            | "brave.exe"
            | "opera.exe"
            | "vivaldi.exe"
            | "gamebar.exe"
            | "ds5bridge.exe"
            | "padmimic.exe"
    );
    if shared || launcher || exe.contains("crashreport") || exe.ends_with("launcher.exe") {
        return AppKind::Other;
    }

    let library = [
        "\\steamapps\\common\\",
        "\\epic games\\",
        "\\xboxgames\\",
        "\\gog games\\",
        "\\ea games\\",
        "\\origin games\\",
        "\\ubisoft game launcher\\games\\",
    ]
    .iter()
    .any(|folder| path.contains(folder));
    let game_binary = exe.ends_with("-win64-shipping.exe") || exe.ends_with("-wingdk-shipping.exe");
    if exe.ends_with(".exe") && (library || game_binary) {
        AppKind::Game
    } else {
        AppKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_game_windows_and_excludes_the_launchers() {
        assert!(classify(r"X:/Games/Fortnite/FortniteClient-Win64-Shipping.exe") == AppKind::Game);
        assert!(classify(r"X:\Other Library\steamapps\common\Game\game.exe") == AppKind::Game);
        assert!(classify(r"X:\XboxGames\Game\Content\game.exe") == AppKind::Game);
        assert!(classify(r"X:\Epic Games\Launcher\Portal\EpicGamesLauncher.exe") == AppKind::Other);
        assert!(
            classify(r"X:\SteamLibrary\steamapps\common\Steamworks Shared\setup.exe")
                == AppKind::Other
        );
        assert!(classify(r"X:\Applications\editor.exe") == AppKind::Unknown);
    }

    #[test]
    fn a_game_notice_can_replace_the_startup_notice_and_does_not_repeat() {
        let mut tracker = GameNotice::new(true);
        assert_eq!(tracker.pending(None), Some(Notice::Restored));
        let game = Notice::InGame(10);
        assert_eq!(tracker.pending(Some(10)), Some(game));
        tracker.delivered(game);
        assert_eq!(tracker.pending(Some(10)), None);
        assert_eq!(tracker.pending(None), None);
        assert_eq!(tracker.pending(Some(20)), Some(Notice::InGame(20)));
        assert_eq!(tracker.poll(false, false), None);
        assert_eq!(tracker.pending(None), None);
        assert_eq!(tracker.pending(Some(10)), Some(game));
    }
}
