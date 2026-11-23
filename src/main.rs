// Never open a console window.
#![windows_subsystem = "windows"]
//! DS5 Bridge: use a PS5 DualSense (USB or Bluetooth) as a virtual Xbox 360 controller on
//! Windows. Offline, no telemetry, MIT licensed.
//!
//! One exe, several roles:
//!   (no args)       first run: installer; installed: start the tray service and open the window
//!   --background    tray service + bridge (no window; what autostart runs)
//!   --ui            the window
//!   --uninstall     the uninstaller (used by "Apps & features"); add --quiet for no window
//!   --install --quiet [--desktop] [--no-autostart] [--no-start]   unattended install
//!   --portable      run from where it is, without installing
//!   --render <view> <lang> <file.png>   README images with demo values (docs/render.ps1)

mod bridge;
mod decode;
mod gui;
mod i18n;
mod icon;
mod installer;
mod ipc;
mod json;
mod settings;
mod shots;
mod sys;
mod tray;
mod updater;
mod win;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    i18n::init(settings::load().lang.as_deref());

    // --render <main|install|uninstall|banner> <lang> <file.png>: README artwork with demo values
    if let [_, view, lang, out] = &args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        if args[0] == "--render" {
            i18n::set(i18n::id_of(lang).unwrap_or(0));
            let mode = match *view { "install" => gui::Mode::Install, "uninstall" => gui::Mode::Uninstall, "banner" => gui::Mode::Banner, _ => gui::Mode::Main };
            return gui::render(mode, out.into());
        }
    }
    if has("--background") {
        tray::run_background(has("--show"));
    } else if has("--ui") {
        gui::run(gui::Mode::Main);
    } else if has("--install") && has("--quiet") {
        // unattended install for scripts: Start menu shortcut + autostart, then start in the tray
        let o = installer::Options { start_menu: true, desktop: has("--desktop"), autostart: !has("--no-autostart") };
        if installer::install(&o).is_ok() && !has("--no-start") {
            let _ = std::process::Command::new(installer::installed_exe()).spawn();
        }
    } else if has("--uninstall") && has("--quiet") {
        installer::uninstall();
    } else if has("--uninstall") {
        gui::run(gui::Mode::Uninstall);
    } else if installer::running_installed() || has("--portable") {
        tray::start_and_show();
    } else {
        gui::run(gui::Mode::Install);
    }
}
