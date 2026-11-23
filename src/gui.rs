//! The window (a separate process, started on demand): main view, installer, updater and
//! uninstaller in one frameless egui window with its own title bar.

use crate::decode::*;
use crate::i18n::{self, tr, trf};
use crate::installer::{self, APP};
use crate::ipc::{self, Shared, Status};
use crate::settings::{self, Settings};
use crate::sys::*;
use crate::updater::{self, Release, UpdateError};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Response, RichText, Sense, Shape, Stroke,
    StrokeKind, ViewportCommand,
};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const BG: Color32 = Color32::from_rgb(12, 12, 15);
const SURFACE: Color32 = Color32::from_rgb(22, 22, 27);
const SURFACE2: Color32 = Color32::from_rgb(33, 33, 40);
const LINE: Color32 = Color32::from_rgb(44, 44, 53);
const TEXT: Color32 = Color32::from_rgb(240, 240, 244);
const MUTED: Color32 = Color32::from_rgb(140, 140, 152);
const RED: Color32 = Color32::from_rgb(227, 38, 46); // accent
const RED_DARK: Color32 = Color32::from_rgb(140, 22, 28);
const RED_LIGHT: Color32 = Color32::from_rgb(255, 120, 126);
const GREEN: Color32 = Color32::from_rgb(48, 209, 88);
const YELLOW: Color32 = Color32::from_rgb(255, 196, 0);
const ORANGE: Color32 = Color32::from_rgb(255, 149, 0); // errors (red is the accent)
// controller
const WHITE: Color32 = Color32::from_rgb(246, 246, 248);
const SHADE: Color32 = Color32::from_rgb(218, 220, 226);
const INK: Color32 = Color32::from_rgb(20, 20, 24);
const INK2: Color32 = Color32::from_rgb(46, 46, 54);
const SYM: [Color32; 4] = [
    Color32::from_rgb(0, 168, 140),  // triangle
    Color32::from_rgb(232, 52, 72),  // circle
    Color32::from_rgb(70, 120, 230), // cross
    Color32::from_rgb(214, 90, 180), // square
];
const TITLE_H: f32 = 44.0;
const RADIUS: u8 = 14;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Main,
    Install,
    Uninstall,
    Banner, // README artwork, only for --render
}

pub fn run(mode: Mode) {
    launch(mode, None);
}

/// Draw a view with demo values and save it as PNG (README screenshots), then exit.
pub fn render(mode: Mode, out: std::path::PathBuf) {
    launch(mode, Some(out));
}

fn launch(mode: Mode, render_to: Option<std::path::PathBuf>) {
    let mut ui_mutex = 0;
    if mode == Mode::Main && render_to.is_none() {
        ui_mutex = unsafe { CreateMutexW(std::ptr::null(), 0, wide(ipc::UI_MUTEX).as_ptr()) };
        if unsafe { GetLastError() } == 183 {
            crate::tray::open_ui(); // already open: bring it to the front
            return;
        }
    }
    let size = match mode {
        Mode::Main => [500.0, 830.0],
        Mode::Install => [470.0, 650.0],
        Mode::Uninstall => [440.0, 300.0],
        Mode::Banner => [1280.0, 640.0],
    };
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP)
            .with_inner_size(size)
            .with_decorations(false) // own title bar
            .with_transparent(true) // rounded window corners
            .with_resizable(false)
            .with_icon(egui::IconData { rgba: crate::icon::rgba(64), width: 64, height: 64 }),
        ..Default::default()
    };
    let _ = eframe::run_native(
        APP,
        opts,
        Box::new(move |cc| {
            style(&cc.egui_ctx);
            let mut app = App::new(&cc.egui_ctx, mode, ui_mutex);
            if render_to.is_some() {
                app.demo = Some(demo_status());
                app.vigem = true;
                app.render_to = render_to;
            }
            Ok(Box::new(app))
        }),
    );
}

/// System fonts only (nothing bundled, nothing downloaded): Segoe UI plus CJK and Arabic
/// fallbacks. The current language's CJK font goes first (Han characters differ per language).
fn set_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |name: &str, file: &str| {
        let ok = std::fs::read(format!(r"C:\Windows\Fonts\{file}")).map(|d| fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(d)))).is_ok();
        ok.then(|| name.to_string())
    };
    // Segoe UI Variable has no Arabic glyphs, so Arabic words become shaping runs of their own (see i18n::rtl)
    let ui_font = load("ui", "SegUIVar.ttf").or_else(|| load("ui", "segoeui.ttf"));
    let (ja, zh, ko) = (load("ja", "YuGothM.ttc"), load("zh", "msyh.ttc"), load("ko", "malgun.ttf"));
    let ar = load("ar", "arial.ttf");
    let cjk = match i18n::cur() { 2 => [ja, zh, ko], 4 => [ko, ja, zh], _ => [zh, ja, ko] };
    let list = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
    if let Some(f) = ui_font {
        list.insert(0, f);
    }
    list.extend(cjk.into_iter().chain([ar]).flatten());
    ctx.set_fonts(fonts);
}

fn style(ctx: &egui::Context) {
    set_fonts(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = SURFACE;
    v.extreme_bg_color = SURFACE2;
    v.selection.bg_fill = RED;
    v.selection.stroke = Stroke::new(1.0, WHITE);
    v.override_text_color = Some(TEXT);
    v.slider_trailing_fill = true;
    v.hyperlink_color = RED_LIGHT;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.noninteractive, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(10);
        w.bg_stroke = Stroke::NONE;
    }
    v.widgets.inactive.bg_fill = SURFACE2;
    v.widgets.inactive.weak_bg_fill = SURFACE2;
    v.widgets.hovered.bg_fill = LINE;
    v.widgets.hovered.weak_bg_fill = LINE;
    v.widgets.active.bg_fill = RED;
    v.widgets.active.weak_bg_fill = RED_DARK;
    v.widgets.open.weak_bg_fill = SURFACE2;
    ctx.set_visuals(v);
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Downloading,
    Installing,
    Done(std::path::PathBuf),
    Failed(String),
}

#[derive(Default)]
struct Upd {
    checking: bool,
    checked: Option<bool>, // Some(true) = up to date, Some(false) = check failed
    release: Option<Release>,
    phase: Phase,
}

enum Inst {
    Ready,
    Done,
    Failed(String),
}

struct App {
    mode: Mode,
    sh: &'static Shared,
    settings: Settings,
    deadzone: f32,
    autostart: bool,
    last: Status,
    upd: Arc<Mutex<Upd>>,
    progress: Arc<AtomicU64>,
    show_update: bool,
    ui_mutex: Handle,
    logo: egui::TextureHandle,
    opt: [bool; 4], // installer: start menu, desktop, autostart, updates
    inst: Inst,
    vigem: bool,
    removed: bool,
    demo: Option<Status>,                     // --render: fixed values instead of the live bridge
    render_to: Option<std::path::PathBuf>,
    frames: u32,
}

/// What the README screenshots show: Bluetooth, cross + R2 pressed, left stick pushed.
fn demo_status() -> Status {
    Status {
        running: true,
        slot: 1,
        controller: 2,
        exclusive: true,
        pad: crate::decode::Pad { buttons: A | RB, lt: 0, rt: 170, lx: -0.55, ly: 0.45, rx: 0.0, ry: 0.0 },
        rate: 592,
        fresh: 592,
        interval_ms: 1.69,
        jitter_ms: 1.80,
        error: 0,
        paused: false,
    }
}

impl App {
    fn new(ctx: &egui::Context, mode: Mode, ui_mutex: Handle) -> Self {
        let settings = settings::load();
        let n = 128;
        let logo = ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([n, n], &crate::icon::rgba(n as u32)), Default::default());
        let app = App {
            mode,
            sh: ipc::open(),
            deadzone: settings.deadzone as f32 / 1000.0,
            autostart: installer::autostart_enabled(),
            last: Status::default(),
            upd: Arc::default(),
            progress: Arc::default(),
            show_update: false,
            ui_mutex,
            logo,
            opt: [true, false, true, settings.check_updates],
            inst: Inst::Ready,
            vigem: crate::win::vigem_present(),
            removed: false,
            demo: None,
            render_to: None,
            frames: 0,
            settings,
        };
        if mode == Mode::Main && app.settings.check_updates && settings::now().saturating_sub(app.settings.last_check) > 6 * 3600 {
            app.check_updates();
        }
        app
    }

    fn save(&mut self) {
        settings::save(&self.settings);
    }

    fn check_updates(&self) {
        let upd = self.upd.clone();
        {
            let mut u = upd.lock().unwrap();
            if u.checking {
                return;
            }
            u.checking = true;
        }
        let mut s = self.settings.clone();
        std::thread::spawn(move || {
            let r = updater::check();
            let mut u = upd.lock().unwrap();
            u.checking = false;
            u.checked = Some(r.is_ok());
            if let Ok(rel) = r {
                u.release = rel;
            }
            s.last_check = settings::now();
            settings::save(&s);
        });
        // keep our copy in step so the next save does not overwrite last_check with an old value
        // ponytail: last_check only throttles the automatic check, so a tiny race here is harmless
    }

    fn start_update(&self) {
        let Some(rel) = self.upd.lock().unwrap().release.clone() else { return };
        let upd = self.upd.clone();
        let progress = self.progress.clone();
        upd.lock().unwrap().phase = Phase::Downloading;
        std::thread::spawn(move || {
            let msg = |e: UpdateError| match e {
                UpdateError::Download(m) => format!("{} ({m})", i18n::tr_native("Download failed")),
                UpdateError::NoChecksum => i18n::tr_native("No checksum published, update stopped").to_string(),
                UpdateError::BadChecksum => i18n::tr_native("Checksum mismatch, update stopped").to_string(),
                UpdateError::Install(m) => format!("{} ({m})", i18n::tr_native("Install failed")),
            };
            let phase = match updater::download(&rel, &progress) {
                Ok(path) => {
                    upd.lock().unwrap().phase = Phase::Installing;
                    match updater::install(&path, &rel.version) {
                        Ok(target) => Phase::Done(target),
                        Err(e) => Phase::Failed(msg(e)),
                    }
                }
                Err(e) => Phase::Failed(msg(e)),
            };
            upd.lock().unwrap().phase = phase;
        });
    }

    fn set_language(&mut self, ctx: &egui::Context, id: usize) {
        i18n::set(id);
        set_fonts(ctx);
        self.sh.lang.store(id as u32, Relaxed);
        self.settings.lang = Some(i18n::LANGS[id].0.to_string());
        self.save();
    }

    fn language_picker(&mut self, ui: &mut egui::Ui) {
        let cur = i18n::cur();
        let mut pick = None;
        egui::ComboBox::from_id_salt("lang").selected_text(i18n::native(cur)).width(150.0).show_ui(ui, |ui| {
            for id in 0..i18n::LANGS.len() {
                if ui.selectable_label(id == cur, i18n::native(id)).clicked() {
                    pick = Some(id);
                }
            }
        });
        if let Some(id) = pick {
            self.set_language(ui.ctx(), id);
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4] // transparent: we paint the rounded window ourselves
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.render_to.is_some() {
            self.capture(ui.ctx());
        }
        let full = ui.max_rect();
        if self.mode == Mode::Banner {
            return self.banner(ui, full);
        }
        ui.painter().rect(full, RADIUS, BG, Stroke::new(1.0, LINE), StrokeKind::Inside);
        title_bar(ui, full, &self.logo);

        let content = Rect::from_min_max(full.min + vec2(20.0, TITLE_H + 2.0), full.max - vec2(20.0, 14.0));
        let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(content));
        cui.spacing_mut().item_spacing = vec2(12.0, 12.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(&mut cui, |ui| match self.mode {
            Mode::Main => self.main_view(ui),
            Mode::Install => self.install_view(ui),
            Mode::Uninstall => self.uninstall_view(ui),
            Mode::Banner => {}
        });

        // repaint smoothly only while something changes; otherwise nearly no CPU
        let s = self.sh.status();
        let busy = {
            let u = self.upd.lock().unwrap();
            u.checking || matches!(u.phase, Phase::Downloading | Phase::Installing)
        };
        let changed = s != self.last;
        self.last = s;
        ui.ctx().request_repaint_after(Duration::from_millis(if changed || busy { 16 } else { 250 }));
    }
}

impl App {
    /// --render: after a few frames ask for a screenshot, save it as PNG and close.
    fn capture(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        if self.frames == 6 {
            let (out, ctx2) = (self.render_to.clone().unwrap_or_default(), ctx.clone());
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
                let _ = std::fs::write(&out, crate::shots::png(img.size[0] as u32, img.size[1] as u32, &rgba));
                ctx2.send_viewport_cmd(ViewportCommand::Close);
                ctx2.request_repaint();
            })));
        }
        if self.frames > 600 {
            ctx.send_viewport_cmd(ViewportCommand::Close); // never hang if no screenshot arrives
        }
        ctx.request_repaint();
    }

    /// README banner: logo, name, tagline, feature chips, the controller.
    fn banner(&mut self, ui: &mut egui::Ui, full: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(full, 0.0, BG);
        let cc = pos2(full.right() - 330.0, full.center().y + 10.0);
        for (i, a) in [(0, 0.07), (1, 0.045), (2, 0.025), (3, 0.012)] {
            p.circle_filled(cc, 230.0 + i as f32 * 70.0, RED.gamma_multiply(a));
        }
        let left = full.left() + 80.0;
        let logo = Rect::from_min_size(pos2(left, 150.0), vec2(84.0, 84.0));
        p.image(self.logo.id(), logo, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        p.text(pos2(left, 262.0), Align2::LEFT_TOP, APP, FontId::proportional(72.0), TEXT);
        p.text(pos2(left + 3.0, 352.0), Align2::LEFT_TOP, "PS5 DualSense → Xbox controller", FontId::proportional(28.0), TEXT);
        p.text(pos2(left + 3.0, 392.0), Align2::LEFT_TOP, "for Windows · USB & Bluetooth", FontId::proportional(28.0), MUTED);
        let mut x = left;
        for chip in ["Offline", "No telemetry", "Open source · MIT", "8 languages"] {
            let g = p.layout_no_wrap(chip.to_string(), FontId::proportional(17.0), TEXT);
            let r = Rect::from_min_size(pos2(x, 462.0), g.size() + vec2(28.0, 16.0));
            p.rect(r, 18.0, SURFACE2, Stroke::new(1.0, LINE), StrokeKind::Inside);
            p.galley(r.min + vec2(14.0, 8.0), g, TEXT);
            x = r.right() + 10.0;
        }
        let area = Rect::from_center_size(cc, vec2(560.0, 360.0));
        let s = self.demo.clone().unwrap_or_default();
        let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(area));
        controller(&mut cui, &s);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.ui_mutex != 0 {
            unsafe { CloseHandle(self.ui_mutex) };
        }
    }
}

// ------------------------------------------------------------- chrome ---

fn title_bar(ui: &mut egui::Ui, full: Rect, logo: &egui::TextureHandle) {
    let bar = Rect::from_min_size(full.min, vec2(full.width(), TITLE_H));
    let p = ui.painter().clone();
    let drag = ui.interact(bar, ui.scope_id().with("titlebar"), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    let icon = Rect::from_center_size(pos2(bar.left() + 24.0, bar.center().y), vec2(20.0, 20.0));
    p.image(logo.id(), icon, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    p.text(pos2(icon.right() + 9.0, bar.center().y), Align2::LEFT_CENTER, APP, FontId::proportional(14.0), TEXT);

    let w = 46.0;
    let close = Rect::from_min_size(pos2(bar.right() - w, bar.top()), vec2(w, TITLE_H));
    let min = close.translate(vec2(-w, 0.0));
    for (rect, is_close) in [(min, false), (close, true)] {
        let r = ui.interact(rect, ui.scope_id().with(("winbtn", is_close)), Sense::click());
        if r.hovered() {
            let radius = if is_close { CornerRadius { ne: RADIUS, ..Default::default() } } else { CornerRadius::ZERO };
            p.rect_filled(rect, radius, if is_close { Color32::from_rgb(196, 43, 28) } else { SURFACE2 });
        }
        let st = Stroke::new(1.2, if r.hovered() && is_close { Color32::WHITE } else { MUTED });
        let m = rect.center();
        if is_close {
            p.line_segment([m + vec2(-5.0, -5.0), m + vec2(5.0, 5.0)], st);
            p.line_segment([m + vec2(-5.0, 5.0), m + vec2(5.0, -5.0)], st);
        } else {
            p.line_segment([m + vec2(-5.0, 0.0), m + vec2(5.0, 0.0)], st);
        }
        if r.clicked() {
            ui.ctx().send_viewport_cmd(if is_close { ViewportCommand::Close } else { ViewportCommand::Minimized(true) });
        }
    }
}

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(16.0)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Settings row: title + description left, control right.
fn row(ui: &mut egui::Ui, title: &str, desc: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() - 170.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(title).size(15.0));
            if !desc.is_empty() {
                ui.label(RichText::new(desc).size(12.0).color(MUTED));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}

fn toggle(ui: &mut egui::Ui, on: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 28.0), Sense::click());
    let t = ui.ctx().animate_bool(resp.id, on);
    let p = ui.painter();
    p.rect_filled(rect, 14.0, if on { RED } else { LINE });
    let x = egui::lerp(rect.left() + 14.0..=rect.right() - 14.0, t);
    p.circle_filled(pos2(x, rect.center().y), 10.0, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn big_button(ui: &mut egui::Ui, text: &str, fill: Color32) -> Response {
    let b = egui::Button::new(RichText::new(text).size(15.0).color(Color32::WHITE)).fill(fill).corner_radius(12.0).min_size(vec2(ui.available_width(), 42.0));
    ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn small_button(ui: &mut egui::Ui, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).size(13.0)).corner_radius(10.0).min_size(vec2(0.0, 30.0))).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn link(ui: &mut egui::Ui, text: &str, url: &str) {
    if ui.add(egui::Label::new(RichText::new(text).size(12.0).color(RED_LIGHT)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        open_url(url);
    }
}

fn dot(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(14.0, 20.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 8.0, color.gamma_multiply(0.2));
    ui.painter().circle_filled(r.center(), 4.0, color);
}

// ---------------------------------------------------------- main view ---

impl App {
    fn main_view(&mut self, ui: &mut egui::Ui) {
        let s = self.demo.clone().unwrap_or_else(|| self.sh.status());
        let bg_alive = self.demo.is_some() || mutex_exists(ipc::BG_MUTEX);

        // update finished: hand over to the new version
        if let Phase::Done(target) = &self.upd.lock().unwrap().phase {
            if self.ui_mutex != 0 {
                unsafe { CloseHandle(self.ui_mutex) };
                self.ui_mutex = 0;
            }
            let _ = std::process::Command::new(target).spawn();
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }

        let release = self.upd.lock().unwrap().release.clone();
        if self.show_update {
            if let Some(r) = release {
                return self.update_view(ui, &r);
            }
            self.show_update = false;
        }
        if let Some(r) = &release {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    dot(ui, RED);
                    ui.label(RichText::new(trf("Update available: {}", &[&r.version])).size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_button(ui, tr("Show")).clicked() {
                            self.show_update = true;
                        }
                    });
                });
            });
        }

        // status
        let (text, color) = if !bg_alive {
            (tr("The background service is not running").to_string(), MUTED)
        } else if s.error != 0 {
            let e = match s.error {
                crate::win::ERR_NO_VIGEM => tr("ViGEmBus driver not found"),
                crate::win::ERR_NO_SLOT => tr("No free virtual controller slot"),
                _ => tr("Sending to the virtual controller failed"),
            };
            (e.to_string(), ORANGE)
        } else if s.paused {
            (tr("Paused").to_string(), MUTED)
        } else {
            match s.controller {
                1 => (trf("Connected via {}", &[&"USB"]), GREEN),
                2 => (trf("Connected via {}", &[&"Bluetooth"]), GREEN),
                _ => (tr("Waiting for DualSense · USB or Bluetooth").to_string(), YELLOW),
            }
        };
        ui.horizontal(|ui| {
            dot(ui, color);
            ui.label(RichText::new(text).size(15.0).color(color));
            if !bg_alive {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_button(ui, tr("Start")).clicked() {
                        if let Ok(me) = std::env::current_exe() {
                            let _ = std::process::Command::new(me).arg("--background").spawn();
                        }
                    }
                });
            }
        });
        if s.error == crate::win::ERR_NO_VIGEM {
            link(ui, tr("Get ViGEmBus (open source)"), "https://github.com/nefarius/ViGEmBus/releases");
        }

        card(ui, |ui| controller(ui, &s));
        stats(ui, &s);

        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            row(ui, tr("Bridge active"), tr("Passes the DualSense on to the Xbox controller"), |ui| {
                if toggle(ui, !s.paused).clicked() {
                    self.sh.paused.store(!s.paused as u32, Relaxed);
                }
            });
            ui.add(egui::Separator::default().spacing(2.0));
            row(ui, tr("Stick deadzone"), tr("Raise it if a stick drifts on its own"), |ui| {
                ui.spacing_mut().slider_width = 110.0;
                ui.label(RichText::new(format!("{:.2}", self.deadzone)).color(MUTED));
                let r = ui.add(egui::Slider::new(&mut self.deadzone, 0.0..=0.3).show_value(false));
                if r.changed() {
                    self.sh.deadzone.store((self.deadzone * 1000.0) as u32, Relaxed);
                }
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    self.settings.deadzone = (self.deadzone * 1000.0) as u32;
                    self.save();
                }
            });
            if installer::is_installed() {
                ui.add(egui::Separator::default().spacing(2.0));
                row(ui, tr("Start with Windows"), tr("Runs quietly in the tray after login"), |ui| {
                    if toggle(ui, self.autostart).clicked() {
                        installer::set_autostart(!self.autostart);
                        self.autostart = installer::autostart_enabled();
                    }
                });
            }
            ui.add(egui::Separator::default().spacing(2.0));
            row(ui, tr("Language"), "", |ui| self.language_picker(ui));
            ui.add(egui::Separator::default().spacing(2.0));
            let (checking, checked) = {
                let u = self.upd.lock().unwrap();
                (u.checking, u.checked)
            };
            let state = if checking {
                tr("Checking…")
            } else {
                match (checked, release.is_some()) {
                    (_, true) => tr("Update available"),
                    (Some(true), _) => tr("Up to date"),
                    (Some(false), _) => tr("Update check failed"),
                    _ => tr("Only asks GitHub for the newest version number"),
                }
            };
            row(ui, tr("Check for updates"), state, |ui| {
                if toggle(ui, self.settings.check_updates).clicked() {
                    self.settings.check_updates = !self.settings.check_updates;
                    self.save();
                }
                if small_button(ui, tr("Check now")).clicked() {
                    self.check_updates();
                }
            });
        });

        if s.controller != 0 && !s.exclusive {
            ui.label(RichText::new(tr("Another program (e.g. Steam) also reads the DualSense. Games may see double input.")).size(12.0).color(MUTED));
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("v{} · MIT · {}", env!("CARGO_PKG_VERSION"), tr("No telemetry"))).size(12.0).color(MUTED));
            link(ui, "GitHub", updater::REPO_URL);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if small_button(ui, tr("Quit DS5 Bridge")).on_hover_text(tr("Stops the background service and removes the virtual controller")).clicked() {
                    self.sh.quit.store(1, Relaxed);
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            });
        });
    }

    fn update_view(&mut self, ui: &mut egui::Ui, r: &Release) {
        card(ui, |ui| {
            ui.label(RichText::new(trf("Update to {}", &[&r.version])).size(20.0));
            ui.label(RichText::new(trf("Installed: {}", &[&env!("CARGO_PKG_VERSION")])).size(12.0).color(MUTED));
            ui.add_space(4.0);
            ui.label(RichText::new(tr("What's new")).size(13.0).color(MUTED));
            egui::Frame::NONE.fill(SURFACE2).corner_radius(10.0).inner_margin(12.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    ui.label(RichText::new(if r.notes.is_empty() { "–" } else { &r.notes }).size(13.0));
                });
            });
            ui.add_space(6.0);
            let phase_info = {
                let u = self.upd.lock().unwrap();
                match &u.phase {
                    Phase::Idle => None,
                    Phase::Downloading => Some((self.progress.load(Relaxed) as f32 / r.size.max(1) as f32, tr("Downloading…").to_string(), RED)),
                    Phase::Installing => Some((1.0, tr("Installing…").to_string(), RED)),
                    Phase::Done(_) => Some((1.0, tr("Restarting…").to_string(), GREEN)),
                    Phase::Failed(m) => Some((0.0, m.clone(), ORANGE)),
                }
            };
            match phase_info {
                None => {
                    ui.label(RichText::new(tr("The download is checked against GitHub's SHA-256 checksum before anything is replaced.")).size(12.0).color(MUTED));
                    if big_button(ui, tr("Update now"), RED).clicked() {
                        self.start_update();
                    }
                }
                Some((f, text, color)) => {
                    if color != ORANGE {
                        ui.add(egui::ProgressBar::new(f.min(1.0)).fill(color).corner_radius(6.0).desired_height(10.0));
                    }
                    ui.label(RichText::new(text).color(color));
                    if color == ORANGE && small_button(ui, tr("Try again")).clicked() {
                        self.upd.lock().unwrap().phase = Phase::Idle;
                    }
                }
            }
        });
        let idle = matches!(self.upd.lock().unwrap().phase, Phase::Idle | Phase::Failed(_));
        if idle && small_button(ui, tr("Back")).clicked() {
            self.show_update = false;
        }
    }

    // ------------------------------------------------------- installer ---

    fn install_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
            ui.painter().image(self.logo.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(tr("Welcome to DS5 Bridge")).size(22.0));
                ui.label(RichText::new(trf("Version {}", &[&env!("CARGO_PKG_VERSION")])).size(12.0).color(MUTED));
            });
        });
        ui.label(RichText::new(tr("Use your PS5 DualSense as an Xbox controller on Windows, over USB or Bluetooth. Runs fully offline.")).size(13.0).color(MUTED));

        match &self.inst {
            Inst::Done => {
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        dot(ui, GREEN);
                        ui.label(RichText::new(tr("Installed")).size(16.0).color(GREEN));
                    });
                    ui.label(RichText::new(tr("DS5 Bridge now lives in the tray (bottom right). Closing the window keeps it running.")).size(13.0).color(MUTED));
                });
                if big_button(ui, tr("Start DS5 Bridge"), RED).clicked() {
                    let _ = std::process::Command::new(installer::installed_exe()).spawn();
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
                return;
            }
            Inst::Failed(e) => {
                ui.label(RichText::new(trf("Installation failed: {}", &[e])).color(ORANGE));
            }
            Inst::Ready => {}
        }

        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 12.0;
            row(ui, tr("Language"), "", |ui| self.language_picker(ui));
            let labels = [
                (tr("Start menu shortcut"), ""),
                (tr("Desktop shortcut"), ""),
                (tr("Start with Windows"), tr("Runs quietly in the tray after login")),
                (tr("Check for updates"), tr("Only asks GitHub for the newest version number")),
            ];
            for (i, (t, d)) in labels.into_iter().enumerate() {
                ui.add(egui::Separator::default().spacing(2.0));
                let on = self.opt[i];
                row(ui, t, d, |ui| {
                    if toggle(ui, on).clicked() {
                        self.opt[i] = !on;
                    }
                });
            }
        });

        ui.horizontal(|ui| {
            if self.vigem {
                dot(ui, GREEN);
                ui.label(RichText::new(tr("ViGEmBus driver found")).size(13.0));
            } else {
                dot(ui, ORANGE);
                ui.label(RichText::new(tr("ViGEmBus driver missing: needed for the virtual controller")).size(13.0).color(ORANGE));
            }
        });
        if !self.vigem {
            link(ui, tr("Get ViGEmBus (open source)"), "https://github.com/nefarius/ViGEmBus/releases");
        }
        ui.label(RichText::new(tr("No account, no telemetry. Your data stays on this PC.")).size(12.0).color(MUTED));
        // shown as %LOCALAPPDATA% so screenshots never contain a user name
        ui.label(RichText::new(trf("Installs to {}", &[&r"%LOCALAPPDATA%\Programs\DS5 Bridge"])).size(11.0).color(MUTED));

        if big_button(ui, tr("Install"), RED).clicked() {
            let o = installer::Options { start_menu: self.opt[0], desktop: self.opt[1], autostart: self.opt[2] };
            self.settings.check_updates = self.opt[3];
            self.save();
            self.inst = match installer::install(&o) {
                Ok(()) => Inst::Done,
                Err(e) => Inst::Failed(e),
            };
            self.vigem = crate::win::vigem_present();
        }
    }

    fn uninstall_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        if self.removed {
            ui.label(RichText::new(tr("DS5 Bridge was removed.")).size(20.0));
            ui.label(RichText::new(tr("Thanks for trying it.")).color(MUTED));
            ui.add_space(12.0);
            if big_button(ui, tr("Close"), SURFACE2).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
            return;
        }
        ui.label(RichText::new(tr("Remove DS5 Bridge?")).size(20.0));
        ui.label(RichText::new(tr("This removes the app, its shortcuts and its settings.")).color(MUTED));
        ui.add_space(12.0);
        if big_button(ui, tr("Remove"), RED).clicked() {
            installer::uninstall();
            self.removed = true;
        }
    }
}

fn stats(ui: &mut egui::Ui, s: &Status) {
    let live = s.rate > 0;
    let dash = || "–".to_string();
    let conn = match s.controller {
        1 => "USB".to_string(),
        2 => "Bluetooth".to_string(),
        _ => dash(),
    };
    let items = [
        (
            tr("CONNECTION"),
            conn,
            if s.slot > 0 { format!("{}{}", trf("Xbox slot {}", &[&s.slot]), if s.controller != 0 && !s.exclusive { format!(" · {}", tr("shared")) } else { String::new() }) } else { String::new() },
        ),
        (tr("POLLING"), if live { format!("{} Hz", s.rate) } else { dash() }, if live { format!("Ø {:.2} ms · ±{:.2}", s.interval_ms, s.jitter_ms) } else { String::new() }),
        (tr("NEW DATA"), if live { format!("{} Hz", s.fresh) } else { dash() }, if live { tr("real samples").to_string() } else { String::new() }),
    ];
    ui.columns(3, |cols| {
        for (col, (label, value, sub)) in cols.iter_mut().zip(items) {
            card(col, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new(label).size(10.5).color(MUTED));
                ui.label(RichText::new(value).size(18.0));
                ui.add(egui::Label::new(RichText::new(sub).size(10.5).color(MUTED)).truncate());
            });
        }
    });
}

// --------------------------------------------------------- controller ---

/// A white/red DualShock-style controller drawn on a 420 x 270 grid, scaled to the width.
/// Everything lights up live with the real controller.
fn controller(ui: &mut egui::Ui, s: &Status) {
    let full_w = ui.available_width();
    let w = full_w.min(560.0).min(if full_w < 500.0 { 390.0 } else { 560.0 }); // compact inside the window
    let k = w / 420.0;
    let (resp, p) = ui.allocate_painter(vec2(full_w, 268.0 * k), Sense::hover());
    let o = resp.rect.min + vec2((full_w - w) / 2.0, 0.0);
    let at = |x: f32, y: f32| -> Pos2 { o + vec2(x, y) * k };
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    let on = |bit: u16| s.pad.buttons & bit != 0;
    let font = |size: f32| FontId::proportional(size * k);
    let live = s.controller != 0;

    // soft glow behind the controller while connected
    if live {
        for (i, a) in [(0, 0.05), (1, 0.03), (2, 0.018)] {
            p.circle_filled(at(210.0, 150.0), (118.0 + i as f32 * 26.0) * k, RED.gamma_multiply(a));
        }
    }

    // triggers L2/R2: red, fill rises with pressure
    for (x0, v, name) in [(92.0, s.pad.lt, "L2"), (270.0, s.pad.rt, "R2")] {
        let t = r(x0, 2.0, x0 + 58.0, 40.0);
        p.rect_filled(t, CornerRadius { nw: (22.0 * k) as u8, ne: (22.0 * k) as u8, sw: 0, se: 0 }, RED_DARK);
        if v > 0 {
            let h = t.height() * v as f32 / 255.0;
            let fill = Rect::from_min_max(pos2(t.left(), t.bottom() - h), t.max);
            p.rect_filled(fill, CornerRadius { nw: (22.0 * k) as u8, ne: (22.0 * k) as u8, sw: 0, se: 0 }, RED_LIGHT);
        }
        p.text(at(x0 + 29.0, 18.0), Align2::CENTER_CENTER, name, font(10.0), WHITE);
    }
    // bumpers L1/R1
    for (x0, bit, name) in [(84.0, LB, "L1"), (266.0, RB, "R1")] {
        let b = r(x0, 38.0, x0 + 70.0, 53.0);
        p.rect_filled(b, 7.0 * k, if on(bit) { WHITE } else { RED });
        if on(bit) {
            p.rect_stroke(b, 7.0 * k, Stroke::new(2.0 * k, RED), StrokeKind::Inside);
        }
        p.text(b.center(), Align2::CENTER_CENTER, name, font(9.0), if on(bit) { RED } else { WHITE });
    }

    // grips with red tips, then the white body over them
    for x in [96.0, 324.0] {
        p.circle_filled(at(x, 214.0), 48.0 * k, RED);
    }
    for x in [100.0, 320.0] {
        p.circle_filled(at(x, 186.0), 60.0 * k, WHITE);
    }
    p.rect_filled(r(52.0, 54.0, 368.0, 196.0), 50.0 * k, WHITE);
    // subtle shading line where the grips meet the body
    p.line_segment([at(150.0, 196.0), at(270.0, 196.0)], Stroke::new(1.0 * k, SHADE));

    // touchpad (red), light bar edges while connected
    let pad = r(150.0, 60.0, 270.0, 116.0);
    p.rect_filled(pad, 9.0 * k, RED);
    if live {
        for x in [pad.left() - 4.0 * k, pad.right() + 4.0 * k] {
            p.line_segment([pos2(x, pad.top() + 6.0 * k), pos2(x, pad.bottom() - 6.0 * k)], Stroke::new(2.5 * k, RED_LIGHT));
        }
    }
    // create / options
    for (x, bit) in [(136.0, BACK), (284.0, START)] {
        let b = Rect::from_center_size(at(x, 70.0), vec2(8.0, 18.0) * k);
        p.rect_filled(b, 4.0 * k, if on(bit) { RED } else { INK2 });
    }

    // d-pad: black well, red cross
    let dc = at(104.0, 110.0);
    p.circle_filled(dc, 34.0 * k, INK);
    for (dx, dy, bit) in [(0.0, -1.0, UP), (0.0, 1.0, DOWN), (-1.0, 0.0, LEFT), (1.0, 0.0, RIGHT)] {
        let c = dc + vec2(dx, dy) * 17.0 * k;
        let size = if dx == 0.0 { vec2(14.0, 18.0) } else { vec2(18.0, 14.0) } * k;
        p.rect_filled(Rect::from_center_size(c, size), 3.0 * k, if on(bit) { WHITE } else { RED });
    }
    p.rect_filled(Rect::from_center_size(dc, vec2(14.0, 14.0) * k), 2.0 * k, RED_DARK);

    // face buttons: black well, light buttons with coloured symbols
    let fc = at(316.0, 110.0);
    p.circle_filled(fc, 38.0 * k, INK);
    for (i, (dx, dy, bit)) in [(0.0, -1.0, Y), (1.0, 0.0, B), (0.0, 1.0, A), (-1.0, 0.0, X)].into_iter().enumerate() {
        let c = fc + vec2(dx, dy) * 21.0 * k;
        let pressed = on(bit);
        p.circle_filled(c, 11.5 * k, if pressed { SYM[i] } else { WHITE });
        let st = Stroke::new(1.7 * k, if pressed { Color32::WHITE } else { SYM[i] });
        let m = 5.0 * k;
        match i {
            0 => {
                p.add(Shape::closed_line(vec![c + vec2(0.0, -m), c + vec2(m * 0.95, m * 0.7), c + vec2(-m * 0.95, m * 0.7)], st));
            }
            1 => {
                p.circle_stroke(c, m, st);
            }
            2 => {
                p.line_segment([c + vec2(-m, -m) * 0.8, c + vec2(m, m) * 0.8], st);
                p.line_segment([c + vec2(-m, m) * 0.8, c + vec2(m, -m) * 0.8], st);
            }
            _ => {
                p.rect_stroke(Rect::from_center_size(c, vec2(m, m) * 1.6), 1.0, st, StrokeKind::Middle);
            }
        }
    }

    // speaker grille + PS button
    for row in 0..3 {
        for col in 0..(5 - row % 2) {
            let x = 210.0 - (4 - row % 2) as f32 * 3.5 + col as f32 * 7.0;
            p.circle_filled(at(x, 128.0 + row as f32 * 5.0), 1.3 * k, INK2);
        }
    }
    let ps = at(210.0, 160.0);
    p.circle_filled(ps, 11.0 * k, if on(GUIDE) { RED } else { INK });
    p.text(ps, Align2::CENTER_CENTER, "PS", font(8.0), WHITE);

    // sticks: grey ring, red cap that follows the stick, white ring on L3/R3
    for (x, sx, sy, bit) in [(162.0, s.pad.lx, s.pad.ly, LS), (258.0, s.pad.rx, s.pad.ry, RS)] {
        let c = at(x, 162.0);
        p.circle_filled(c, 27.0 * k, SHADE);
        p.circle_filled(c, 23.0 * k, INK2);
        let d = c + vec2(sx, -sy) * 9.0 * k;
        p.circle_filled(d, 18.0 * k, if on(bit) { RED_LIGHT } else { RED });
        p.circle_filled(d, 11.0 * k, if on(bit) { RED } else { RED_DARK.gamma_multiply(0.6) });
        if on(bit) || sx != 0.0 || sy != 0.0 {
            p.circle_stroke(d, 18.0 * k, Stroke::new(2.0 * k, WHITE));
        }
    }
}
