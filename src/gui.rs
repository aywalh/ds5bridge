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
    self, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Response, RichText, Sense, Shape,
    Stroke, StrokeKind, ViewportCommand,
};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// warm graphite, ivory text, one quiet accent the user picks
const BG: Color32 = Color32::from_rgb(15, 15, 17);
const SURFACE: Color32 = Color32::from_rgb(21, 21, 24);
const SURFACE2: Color32 = Color32::from_rgb(31, 31, 35);
const HOVER: Color32 = Color32::from_rgb(44, 44, 49);
const LINE: Color32 = Color32::from_rgb(38, 38, 43);
const TEXT: Color32 = Color32::from_rgb(232, 229, 223);
const MUTED: Color32 = Color32::from_rgb(130, 127, 121);
const OK: Color32 = Color32::from_rgb(140, 184, 146);
const WAIT: Color32 = Color32::from_rgb(212, 180, 118);
const ERR: Color32 = Color32::from_rgb(210, 124, 98);
const TITLE_H: f32 = 56.0;
const FOOT_H: f32 = 40.0;
const RADIUS: u8 = 14;

/// Accent presets: muted, warm tones (name, 0xRRGGBB).
pub const PRESETS: [(&str, u32); 6] = [
    ("Champagne", 0xC9B37E),
    ("Sage", 0x8FA98A),
    ("Slate", 0x8A9BB5),
    ("Rosé", 0xC49A9A),
    ("Bronze", 0xB08D6E),
    ("Ivory", 0xE2DDD2),
];

static ACCENT: AtomicU32 = AtomicU32::new(0xC9B37E);

fn accent() -> Color32 {
    let v = ACCENT.load(Relaxed);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Text colour that reads on top of the accent.
fn on_accent(c: Color32) -> Color32 {
    if 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32 > 140.0 { BG } else { TEXT }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Soft mechanical clicks for sliders and checkboxes, made in code and played from memory.
mod sound {
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    pub static ON: AtomicBool = AtomicBool::new(true);
    static WAVS: OnceLock<[Vec<u8>; 2]> = OnceLock::new();
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);

    /// A short 16-bit mono WAV: a sine with a touch of noise, fast attack, quick decay.
    pub fn wav(freq: f32, ms: f32, vol: f32) -> Vec<u8> {
        let rate = 44_100u32;
        let n = (rate as f32 * ms / 1000.0) as u32;
        let mut w = Vec::with_capacity(44 + n as usize * 2);
        w.extend(b"RIFF");
        w.extend((36 + n * 2).to_le_bytes());
        w.extend(b"WAVEfmt ");
        for v in [16u32] {
            w.extend(v.to_le_bytes());
        }
        for v in [1u16, 1] {
            w.extend(v.to_le_bytes()); // PCM, mono
        }
        w.extend(rate.to_le_bytes());
        w.extend((rate * 2).to_le_bytes());
        for v in [2u16, 16] {
            w.extend(v.to_le_bytes());
        }
        w.extend(b"data");
        w.extend((n * 2).to_le_bytes());
        let tau = ms / 1000.0 / 4.0;
        let mut seed = 0x1234_5678u32;
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let env = if t < 0.0004 { t / 0.0004 } else { (-(t - 0.0004) / tau).exp() };
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 16) as f32 / 32_768.0 - 1.0;
            let s = (0.8 * (std::f32::consts::TAU * freq * t).sin() + 0.2 * noise) * env * vol;
            w.extend(((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
        }
        w
    }

    fn play(i: usize, gap_ms: u64) {
        if !ON.load(Relaxed) {
            return;
        }
        let mut last = LAST.lock().unwrap();
        if last.is_some_and(|t| t.elapsed() < Duration::from_millis(gap_ms)) {
            return;
        }
        *last = Some(Instant::now());
        let w = &WAVS.get_or_init(|| [wav(2300.0, 9.0, 0.09), wav(1200.0, 16.0, 0.11)])[i];
        // SND_ASYNC | SND_NODEFAULT | SND_MEMORY; the buffer is static so it outlives playback
        unsafe { crate::sys::PlaySoundW(w.as_ptr(), 0, 0x0001 | 0x0002 | 0x0004) };
    }

    /// One detent of a slider.
    pub fn tick() {
        play(0, 28);
    }

    /// A checkbox or button.
    pub fn click() {
        play(1, 0);
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Main,
    Install,
    Uninstall,
    Banner, // README artwork, only for --render
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Controller,
    Settings,
    About,
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
        Mode::Main => [860.0, 560.0],
        Mode::Install => [560.0, 600.0],
        Mode::Uninstall => [460.0, 300.0],
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
            set_fonts(&cc.egui_ctx);
            let mut app = App::new(&cc.egui_ctx, mode, ui_mutex);
            if render_to.is_some() {
                app.demo = Some(demo_status());
                app.vigem = true;
                // a file name with "xbox" in it shows the Xbox view (README screenshot)
                if render_to.as_ref().is_some_and(|p| p.to_string_lossy().contains("xbox")) {
                    app.skin = Skin::Xbox;
                }
                app.render_to = render_to;
                sound::ON.store(false, Relaxed);
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

fn visuals(ctx: &egui::Context) {
    let a = accent();
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = SURFACE;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = SURFACE2;
    v.selection.bg_fill = a.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, a);
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = a;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.noninteractive, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(8);
        w.bg_stroke = Stroke::NONE;
    }
    v.widgets.inactive.bg_fill = SURFACE2;
    v.widgets.inactive.weak_bg_fill = SURFACE2;
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.active.bg_fill = HOVER;
    v.widgets.active.weak_bg_fill = HOVER;
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
    tab: Tab,
    skin: Skin,
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
    demo: Option<Status>, // --render: fixed values instead of the live bridge
    render_to: Option<std::path::PathBuf>,
    frames: u32,
}

/// What the README screenshots show: Bluetooth, cross + R1 pressed, R2 half way, left stick pushed.
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
        ACCENT.store(settings.accent, Relaxed);
        sound::ON.store(settings.sounds, Relaxed);
        visuals(ctx);
        let n = 128;
        let logo = ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([n, n], &crate::icon::rgba(n as u32)), Default::default());
        let app = App {
            mode,
            tab: Tab::Controller,
            skin: Skin::DualSense,
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

    fn set_accent(&mut self, ctx: &egui::Context, rgb: u32) {
        ACCENT.store(rgb, Relaxed);
        self.settings.accent = rgb;
        self.save();
        visuals(ctx);
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
        // ponytail: last_check only throttles the automatic check, so a tiny race with our own save is harmless
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
        egui::ComboBox::from_id_salt("lang").selected_text(i18n::native(cur)).width(170.0).show_ui(ui, |ui| {
            for id in 0..i18n::LANGS.len() {
                if ui.selectable_label(id == cur, i18n::native(id)).clicked() {
                    pick = Some(id);
                }
            }
        });
        if let Some(id) = pick {
            sound::click();
            self.set_language(ui.ctx(), id);
        }
    }

    fn status(&self) -> Status {
        self.demo.clone().unwrap_or_else(|| self.sh.status())
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
        let tab = if self.mode == Mode::Main { Some(&mut self.tab) } else { None };
        title_bar(ui, full, &self.logo, tab);

        if self.mode == Mode::Main {
            self.main_layout(ui, full);
        } else {
            let content = Rect::from_min_max(full.min + vec2(32.0, TITLE_H + 24.0), full.max - vec2(32.0, 24.0));
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(content));
            cui.spacing_mut().item_spacing = vec2(10.0, 10.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(&mut cui, |ui| match self.mode {
                Mode::Install => self.install_view(ui),
                _ => self.uninstall_view(ui),
            });
        }

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

    /// README banner: logo, name, tagline, feature chips, the live preview.
    fn banner(&mut self, ui: &mut egui::Ui, full: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(full, 0.0, BG);
        let left = full.left() + 80.0;
        let logo = Rect::from_min_size(pos2(left, 150.0), vec2(76.0, 76.0));
        p.image(self.logo.id(), logo, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        p.text(pos2(left, 256.0), Align2::LEFT_TOP, APP, FontId::proportional(68.0), TEXT);
        p.text(pos2(left + 3.0, 344.0), Align2::LEFT_TOP, "PS5 DualSense → Xbox controller", FontId::proportional(26.0), TEXT);
        p.text(pos2(left + 3.0, 382.0), Align2::LEFT_TOP, "for Windows · USB & Bluetooth", FontId::proportional(26.0), MUTED);
        let mut x = left;
        for chip in ["Offline", "No telemetry", "Open source · MIT", "8 languages"] {
            let g = p.layout_no_wrap(chip.to_string(), FontId::proportional(16.0), TEXT);
            let r = Rect::from_min_size(pos2(x, 452.0), g.size() + vec2(26.0, 14.0));
            p.rect(r, 8.0, SURFACE2, Stroke::new(1.0, LINE), StrokeKind::Inside);
            p.galley(r.min + vec2(13.0, 7.0), g, TEXT);
            x = r.right() + 10.0;
        }
        let panel = Rect::from_min_size(pos2(full.right() - 620.0, 110.0), vec2(540.0, 420.0));
        p.rect(panel, 14.0, SURFACE, Stroke::new(1.0, LINE), StrokeKind::Inside);
        p.text(panel.min + vec2(0.0, -26.0), Align2::LEFT_TOP, "LIVE PREVIEW", FontId::proportional(13.0), MUTED);
        controller(&p, panel.shrink(30.0), &self.demo.clone().unwrap_or_default(), Skin::DualSense);
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

/// Logo pill, tabs (main window only), minimise and close.
fn title_bar(ui: &mut egui::Ui, full: Rect, logo: &egui::TextureHandle, tab: Option<&mut Tab>) {
    let bar = Rect::from_min_size(full.min, vec2(full.width(), TITLE_H));
    let p = ui.painter().clone();
    let drag = ui.interact(bar, ui.scope_id().with("titlebar"), Sense::click_and_drag());
    if drag.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    p.line_segment([pos2(bar.left(), bar.bottom()), pos2(bar.right(), bar.bottom())], Stroke::new(1.0, LINE));

    let name = p.layout_no_wrap(APP.to_string(), FontId::proportional(13.5), TEXT);
    let pill = Rect::from_min_size(pos2(bar.left() + 14.0, bar.center().y - 17.0), vec2(name.size().x + 52.0, 34.0));
    p.rect(pill, 9.0, SURFACE2, Stroke::new(1.0, LINE), StrokeKind::Inside);
    let icon = Rect::from_center_size(pos2(pill.left() + 19.0, pill.center().y), vec2(18.0, 18.0));
    p.image(logo.id(), icon, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    p.galley(pos2(icon.right() + 9.0, pill.center().y - name.size().y / 2.0), name, TEXT);

    if let Some(tab) = tab {
        let mut x = pill.right() + 28.0;
        for (t, label) in [(Tab::Controller, tr("Controller")), (Tab::Settings, tr("Settings")), (Tab::About, tr("About"))] {
            let g = p.layout_no_wrap(label.to_string(), FontId::proportional(14.0), TEXT);
            let r = Rect::from_min_size(pos2(x, bar.top()), vec2(g.size().x, TITLE_H));
            let resp = ui.interact(r, ui.scope_id().with(("tab", label)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() && *tab != t {
                *tab = t;
                sound::click();
            }
            let active = *tab == t;
            let color = if active || resp.hovered() { TEXT } else { MUTED };
            p.galley(pos2(x, bar.center().y - g.size().y / 2.0), g.clone(), color);
            if active {
                let y = bar.bottom() - 10.0;
                p.line_segment([pos2(x, y), pos2(x + g.size().x, y)], Stroke::new(2.0, accent()));
            }
            x += g.size().x + 28.0;
        }
    }

    let w = 46.0;
    let close = Rect::from_min_size(pos2(bar.right() - w, bar.top()), vec2(w, TITLE_H - 1.0));
    let min = close.translate(vec2(-w, 0.0));
    for (rect, is_close) in [(min, false), (close, true)] {
        let r = ui.interact(rect, ui.scope_id().with(("winbtn", is_close)), Sense::click());
        if r.hovered() {
            let radius = if is_close { CornerRadius { ne: RADIUS, ..Default::default() } } else { CornerRadius::ZERO };
            p.rect_filled(rect, radius, if is_close { Color32::from_rgb(170, 52, 44) } else { SURFACE2 });
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

/// Small uppercase section label.
fn caption(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(11.5).color(MUTED));
}

/// Square checkbox with its label; plays a click.
fn check(ui: &mut egui::Ui, on: &mut bool, label: &str) -> Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(14.0), TEXT);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(32.0 + g.size().x, 26.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        sound::click();
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let b = Rect::from_center_size(pos2(rect.left() + 10.0, rect.center().y), vec2(20.0, 20.0));
    let p = ui.painter();
    let a = accent();
    p.rect_filled(b, 5.0, mix(if resp.hovered() { HOVER } else { SURFACE2 }, a, t));
    if t > 0.0 {
        let c = b.center();
        let pts = vec![c + vec2(-5.0, 0.5), c + vec2(-1.5, 4.0), c + vec2(5.0, -3.5)];
        p.add(Shape::line(pts, Stroke::new(2.0, on_accent(a).gamma_multiply(t))));
    }
    p.galley(pos2(b.right() + 12.0, rect.center().y - g.size().y / 2.0), g, TEXT);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Pill slider with the value in the middle and detents you can hear.
fn slider(ui: &mut egui::Ui, v: &mut f32, lo: f32, hi: f32, step: f32, text: impl Fn(f32) -> String) -> Response {
    let w = ui.available_width().min(240.0);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click_and_drag());
    let pad = 13.0;
    let span = rect.width() - 2.0 * pad;
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let t = ((pos.x - rect.left() - pad) / span).clamp(0.0, 1.0);
            let nv = (((lo + t * (hi - lo)) / step).round() * step).clamp(lo, hi);
            if (nv - *v).abs() >= step * 0.5 {
                *v = nv;
                sound::tick();
                resp.mark_changed();
            }
        }
    }
    let x = rect.left() + pad + (*v - lo) / (hi - lo) * span;
    let p = ui.painter();
    let a = accent();
    p.rect_filled(rect, 13.0, SURFACE2);
    p.rect_filled(Rect::from_min_max(rect.min, pos2(x + pad, rect.bottom())), 13.0, a.gamma_multiply(0.18));
    p.text(rect.center(), Align2::CENTER_CENTER, text(*v), FontId::proportional(12.5), TEXT);
    let grow = ui.ctx().animate_bool(resp.id.with("knob"), resp.hovered() || resp.dragged());
    p.circle_filled(pos2(x, rect.center().y), 8.0 + grow * 1.5, TEXT);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Label column + control, one line.
fn field(ui: &mut egui::Ui, label: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(140.0, 26.0), Sense::hover());
        ui.painter().text(pos2(r.left(), r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.5), MUTED);
        control(ui);
    });
}

fn primary(ui: &mut egui::Ui, text: &str) -> Response {
    let a = accent();
    let b = egui::Button::new(RichText::new(text).size(14.5).color(on_accent(a))).fill(a).corner_radius(9.0);
    let r = ui.add_sized([ui.available_width(), 40.0], b).on_hover_cursor(egui::CursorIcon::PointingHand);
    if r.clicked() {
        sound::click();
    }
    r
}

fn secondary(ui: &mut egui::Ui, text: &str) -> Response {
    let r = ui.add(egui::Button::new(RichText::new(text).size(13.0)).corner_radius(8.0).min_size(vec2(0.0, 28.0))).on_hover_cursor(egui::CursorIcon::PointingHand);
    if r.clicked() {
        sound::click();
    }
    r
}

fn link(ui: &mut egui::Ui, text: &str, url: &str) {
    if ui.add(egui::Label::new(RichText::new(text).size(12.5).color(accent())).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        open_url(url);
    }
}

fn dot(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(12.0, 20.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 7.0, color.gamma_multiply(0.18));
    ui.painter().circle_filled(r.center(), 3.5, color);
}

// ---------------------------------------------------------- main view ---

impl App {
    fn main_layout(&mut self, ui: &mut egui::Ui, full: Rect) {
        let s = self.status();
        let body = Rect::from_min_max(full.min + vec2(28.0, TITLE_H + 22.0), full.max - vec2(28.0, FOOT_H + 18.0));
        let right = Rect::from_min_max(pos2(body.right() - 390.0, body.top()), body.max);
        let left = Rect::from_min_max(body.min, pos2(right.left() - 32.0, body.bottom()));

        let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(left));
        lui.spacing_mut().item_spacing = vec2(10.0, 8.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(&mut lui, |ui| match self.tab {
            Tab::Controller => self.controller_tab(ui, &s),
            Tab::Settings => self.settings_tab(ui),
            Tab::About => self.about_tab(ui),
        });

        // live preview
        let p = ui.painter().clone();
        p.text(right.min + vec2(0.0, 3.0), Align2::LEFT_TOP, tr("LIVE PREVIEW"), FontId::proportional(11.5), MUTED);
        // DualSense = what you hold, Xbox = what games see
        let mut x = right.right();
        for (skin, name) in [(Skin::Xbox, "Xbox"), (Skin::DualSense, "DualSense")] {
            let g = p.layout_no_wrap(name.to_string(), FontId::proportional(12.0), TEXT);
            let r = Rect::from_min_max(pos2(x - g.size().x - 18.0, right.top()), pos2(x, right.top() + 20.0));
            let resp = ui.interact(r, ui.scope_id().with(("skin", name)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() && self.skin != skin {
                self.skin = skin;
                sound::click();
            }
            let active = self.skin == skin;
            if active {
                p.rect_filled(r, 6.0, SURFACE2);
            }
            p.galley(r.center() - g.size() / 2.0, g, if active || resp.hovered() { TEXT } else { MUTED });
            x = r.left() - 4.0;
        }
        let panel = Rect::from_min_max(right.min + vec2(0.0, 24.0), right.max - vec2(0.0, 26.0));
        p.rect(panel, 12.0, SURFACE, Stroke::new(1.0, LINE), StrokeKind::Inside);
        controller(&p, panel.shrink(10.0), &s, self.skin);
        let hint = if s.controller == 0 {
            tr("Connect a DualSense to see it here.")
        } else if self.skin == Skin::Xbox {
            tr("This is what games see.")
        } else {
            tr("Press any button to test it.")
        };
        p.text(pos2(right.left(), right.bottom() - 2.0), Align2::LEFT_BOTTOM, hint, FontId::proportional(12.5), MUTED);

        // footer
        let foot = Rect::from_min_max(pos2(full.left(), full.bottom() - FOOT_H), full.max);
        p.line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, LINE));
        let v = p.layout_no_wrap(format!("v{}", env!("CARGO_PKG_VERSION")), FontId::proportional(12.0), MUTED);
        let pill = Rect::from_min_size(pos2(foot.left() + 14.0, foot.center().y - 12.0), vec2(v.size().x + 22.0, 24.0));
        p.rect(pill, 7.0, SURFACE2, Stroke::new(1.0, LINE), StrokeKind::Inside);
        p.galley(pill.center() - v.size() / 2.0, v, MUTED);
        let mid = match s.controller {
            1 | 2 if s.rate > 0 => format!("{} · {} Hz", if s.controller == 1 { "USB" } else { "Bluetooth" }, s.rate),
            1 | 2 => (if s.controller == 1 { "USB" } else { "Bluetooth" }).to_string(),
            _ => tr("No controller").to_string(),
        };
        p.text(foot.center(), Align2::CENTER_CENTER, mid, FontId::proportional(12.5), MUTED);
        p.text(pos2(foot.right() - 18.0, foot.center().y), Align2::RIGHT_CENTER, format!("MIT · {}", tr("No telemetry")), FontId::proportional(12.5), MUTED);
    }

    fn status_line(&self, ui: &mut egui::Ui, s: &Status) {
        let bg_alive = self.demo.is_some() || mutex_exists(ipc::BG_MUTEX);
        let (text, color) = if !bg_alive {
            (tr("The background service is not running").to_string(), MUTED)
        } else if s.error != 0 {
            let e = match s.error {
                crate::win::ERR_NO_VIGEM => tr("ViGEmBus driver not found"),
                crate::win::ERR_NO_SLOT => tr("No free virtual controller slot"),
                _ => tr("Sending to the virtual controller failed"),
            };
            (e.to_string(), ERR)
        } else if s.paused {
            (tr("Paused").to_string(), MUTED)
        } else {
            match s.controller {
                1 => (trf("Connected via {}", &[&"USB"]), OK),
                2 => (trf("Connected via {}", &[&"Bluetooth"]), OK),
                _ => (tr("Waiting for DualSense · USB or Bluetooth").to_string(), WAIT),
            }
        };
        ui.horizontal(|ui| {
            dot(ui, color);
            ui.label(RichText::new(text).size(15.0));
            if !bg_alive && secondary(ui, tr("Start")).clicked() {
                if let Ok(me) = std::env::current_exe() {
                    let _ = std::process::Command::new(me).arg("--background").spawn();
                }
            }
        });
        if s.error == crate::win::ERR_NO_VIGEM {
            link(ui, tr("Get ViGEmBus (open source)"), "https://github.com/nefarius/ViGEmBus/releases");
        }
    }

    fn controller_tab(&mut self, ui: &mut egui::Ui, s: &Status) {
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
            ui.horizontal(|ui| {
                dot(ui, accent());
                ui.label(RichText::new(trf("Update available: {}", &[&r.version])).size(14.0));
                if secondary(ui, tr("Show")).clicked() {
                    self.show_update = true;
                }
            });
        }

        self.status_line(ui, s);
        caption(ui, tr("CONNECTION"));
        let live = s.rate > 0;
        let dash = || "–".to_string();
        let conn = match s.controller {
            1 => "USB".to_string(),
            2 => "Bluetooth".to_string(),
            _ => dash(),
        };
        let slot = if s.slot > 0 { trf("Xbox slot {}", &[&s.slot]) } else { dash() };
        let shared = if s.controller != 0 && !s.exclusive { format!(" · {}", tr("shared")) } else { String::new() };
        field(ui, tr("Connection"), |ui| {
            ui.label(RichText::new(format!("{conn}{shared}")).size(14.0));
        });
        field(ui, tr("Virtual controller"), |ui| {
            ui.label(RichText::new(slot).size(14.0));
        });
        field(ui, tr("Polling"), |ui| {
            ui.label(RichText::new(if live { format!("{} Hz", s.rate) } else { dash() }).size(14.0));
            if live {
                ui.label(RichText::new(format!("Ø {:.2} ms · ±{:.2}", s.interval_ms, s.jitter_ms)).size(12.5).color(MUTED));
            }
        });
        field(ui, tr("New data"), |ui| {
            ui.label(RichText::new(if live { format!("{} Hz", s.fresh) } else { dash() }).size(14.0));
            if live {
                ui.label(RichText::new(tr("real samples")).size(12.5).color(MUTED));
            }
        });

        caption(ui, tr("INPUT"));
        let mut active = !s.paused;
        if check(ui, &mut active, tr("Bridge active")).changed() {
            self.sh.paused.store(!active as u32, Relaxed);
        }
        field(ui, tr("Stick deadzone"), |ui| {
            let r = slider(ui, &mut self.deadzone, 0.0, 0.3, 0.01, |v| format!("{:.0} %", v * 100.0));
            if r.changed() {
                self.sh.deadzone.store((self.deadzone * 1000.0).round() as u32, Relaxed);
            }
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                self.settings.deadzone = (self.deadzone * 1000.0).round() as u32;
                self.save();
            }
        });
        if s.controller != 0 && !s.exclusive {
            ui.add_space(4.0);
            ui.label(RichText::new(tr("Another program (e.g. Steam) also reads the DualSense. Games may see double input.")).size(12.0).color(MUTED));
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        caption(ui, tr("GENERAL"));
        if installer::is_installed() {
            let mut a = self.autostart;
            if check(ui, &mut a, tr("Start with Windows")).changed() {
                installer::set_autostart(a);
                self.autostart = installer::autostart_enabled();
            }
        }
        let mut u = self.settings.check_updates;
        if check(ui, &mut u, tr("Check for updates")).changed() {
            self.settings.check_updates = u;
            self.save();
        }
        let mut snd = self.settings.sounds;
        if check(ui, &mut snd, tr("Sounds")).changed() {
            self.settings.sounds = snd;
            sound::ON.store(snd, Relaxed);
            sound::click();
            self.save();
        }
        ui.add_space(4.0);
        field(ui, tr("Language"), |ui| self.language_picker(ui));

        caption(ui, tr("APPEARANCE"));
        field(ui, tr("Accent"), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (name, rgb) in PRESETS {
                let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                let c = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
                let p = ui.painter();
                if self.settings.accent == rgb {
                    p.circle_stroke(r.center(), 11.5, Stroke::new(1.5, TEXT));
                } else if resp.hovered() {
                    p.circle_stroke(r.center(), 11.5, Stroke::new(1.0, MUTED));
                }
                p.circle_filled(r.center(), 8.5, c);
                if resp.on_hover_text(name).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    sound::click();
                    self.set_accent(ui.ctx(), rgb);
                }
            }
        });
        field(ui, tr("Custom colour"), |ui| {
            let v = self.settings.accent;
            let mut c = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
            if ui.color_edit_button_srgb(&mut c).changed() {
                self.set_accent(ui.ctx(), (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32);
            }
            ui.label(RichText::new(format!("#{:06X}", self.settings.accent)).size(12.5).color(MUTED));
        });
    }

    fn about_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            ui.painter().image(self.logo.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(APP).size(20.0));
                ui.label(RichText::new(trf("Version {}", &[&env!("CARGO_PKG_VERSION")])).size(12.5).color(MUTED));
            });
        });
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Use your PS5 DualSense as an Xbox controller on Windows, over USB or Bluetooth. Runs fully offline.")).size(13.0).color(MUTED));
        ui.label(RichText::new(tr("No account, no telemetry. Your data stays on this PC.")).size(13.0).color(MUTED));

        caption(ui, tr("UPDATES"));
        let (checking, checked, has) = {
            let u = self.upd.lock().unwrap();
            (u.checking, u.checked, u.release.is_some())
        };
        let state = if checking {
            tr("Checking…")
        } else {
            match (checked, has) {
                (_, true) => tr("Update available"),
                (Some(true), _) => tr("Up to date"),
                (Some(false), _) => tr("Update check failed"),
                _ => tr("Only asks GitHub for the newest version number"),
            }
        };
        ui.horizontal(|ui| {
            if secondary(ui, tr("Check now")).clicked() {
                self.check_updates();
            }
            if has && secondary(ui, tr("Show")).clicked() {
                self.tab = Tab::Controller;
                self.show_update = true;
            }
            ui.label(RichText::new(state).size(12.5).color(MUTED));
        });

        caption(ui, tr("PROJECT"));
        ui.horizontal(|ui| {
            link(ui, "GitHub", updater::REPO_URL);
            ui.label(RichText::new("· MIT").size(12.5).color(MUTED));
        });
        ui.add_space(10.0);
        if secondary(ui, tr("Quit DS5 Bridge")).on_hover_text(tr("Stops the background service and removes the virtual controller")).clicked() {
            self.sh.quit.store(1, Relaxed);
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn update_view(&mut self, ui: &mut egui::Ui, r: &Release) {
        ui.label(RichText::new(trf("Update to {}", &[&r.version])).size(20.0));
        ui.label(RichText::new(trf("Installed: {}", &[&env!("CARGO_PKG_VERSION")])).size(12.5).color(MUTED));
        caption(ui, tr("What's new"));
        egui::Frame::NONE.fill(SURFACE).stroke(Stroke::new(1.0, LINE)).corner_radius(10.0).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                ui.label(RichText::new(if r.notes.is_empty() { "–" } else { &r.notes }).size(13.0));
            });
        });
        ui.add_space(4.0);
        let phase_info = {
            let u = self.upd.lock().unwrap();
            match &u.phase {
                Phase::Idle => None,
                Phase::Downloading => Some((self.progress.load(Relaxed) as f32 / r.size.max(1) as f32, tr("Downloading…").to_string(), accent())),
                Phase::Installing => Some((1.0, tr("Installing…").to_string(), accent())),
                Phase::Done(_) => Some((1.0, tr("Restarting…").to_string(), OK)),
                Phase::Failed(m) => Some((0.0, m.clone(), ERR)),
            }
        };
        match phase_info {
            None => {
                ui.label(RichText::new(tr("The download is checked against GitHub's SHA-256 checksum before anything is replaced.")).size(12.0).color(MUTED));
                if primary(ui, tr("Update now")).clicked() {
                    self.start_update();
                }
            }
            Some((f, text, color)) => {
                if color != ERR {
                    ui.add(egui::ProgressBar::new(f.min(1.0)).fill(color).corner_radius(5.0).desired_height(8.0));
                }
                ui.label(RichText::new(text).color(color));
                if color == ERR && secondary(ui, tr("Try again")).clicked() {
                    self.upd.lock().unwrap().phase = Phase::Idle;
                }
            }
        }
        let idle = matches!(self.upd.lock().unwrap().phase, Phase::Idle | Phase::Failed(_));
        if idle && secondary(ui, tr("Back")).clicked() {
            self.show_update = false;
        }
    }

    // ------------------------------------------------------- installer ---

    fn install_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
            ui.painter().image(self.logo.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(tr("Welcome to DS5 Bridge")).size(22.0));
                ui.label(RichText::new(trf("Version {}", &[&env!("CARGO_PKG_VERSION")])).size(12.5).color(MUTED));
            });
        });
        ui.label(RichText::new(tr("Use your PS5 DualSense as an Xbox controller on Windows, over USB or Bluetooth. Runs fully offline.")).size(13.0).color(MUTED));

        match &self.inst {
            Inst::Done => {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    dot(ui, OK);
                    ui.label(RichText::new(tr("Installed")).size(16.0));
                });
                ui.label(RichText::new(tr("DS5 Bridge now lives in the tray (bottom right). Closing the window keeps it running.")).size(13.0).color(MUTED));
                ui.add_space(8.0);
                if primary(ui, tr("Start DS5 Bridge")).clicked() {
                    let _ = std::process::Command::new(installer::installed_exe()).spawn();
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
                return;
            }
            Inst::Failed(e) => {
                ui.label(RichText::new(trf("Installation failed: {}", &[e])).color(ERR));
            }
            Inst::Ready => {}
        }

        caption(ui, tr("OPTIONS"));
        let labels = [tr("Start menu shortcut"), tr("Desktop shortcut"), tr("Start with Windows"), tr("Check for updates")];
        for (i, t) in labels.into_iter().enumerate() {
            check(ui, &mut self.opt[i], t);
        }
        ui.add_space(4.0);
        field(ui, tr("Language"), |ui| self.language_picker(ui));

        caption(ui, tr("DRIVER"));
        ui.horizontal(|ui| {
            if self.vigem {
                dot(ui, OK);
                ui.label(RichText::new(tr("ViGEmBus driver found")).size(13.5));
            } else {
                dot(ui, ERR);
                ui.label(RichText::new(tr("ViGEmBus driver missing: needed for the virtual controller")).size(13.5).color(ERR));
            }
        });
        if !self.vigem {
            link(ui, tr("Get ViGEmBus (open source)"), "https://github.com/nefarius/ViGEmBus/releases");
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tr("No account, no telemetry. Your data stays on this PC.")).size(12.0).color(MUTED));
        // shown as %LOCALAPPDATA% so screenshots never contain a user name
        ui.label(RichText::new(trf("Installs to {}", &[&r"%LOCALAPPDATA%\Programs\DS5 Bridge"])).size(11.5).color(MUTED));
        ui.add_space(6.0);
        if primary(ui, tr("Install")).clicked() {
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
        if self.removed {
            ui.label(RichText::new(tr("DS5 Bridge was removed.")).size(20.0));
            ui.label(RichText::new(tr("Thanks for trying it.")).color(MUTED));
            ui.add_space(12.0);
            if secondary(ui, tr("Close")).clicked() {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            }
            return;
        }
        ui.label(RichText::new(tr("Remove DS5 Bridge?")).size(20.0));
        ui.label(RichText::new(tr("This removes the app, its shortcuts and its settings.")).color(MUTED));
        ui.add_space(12.0);
        if primary(ui, tr("Remove")).clicked() {
            installer::uninstall();
            self.removed = true;
        }
    }
}

// --------------------------------------------------------- controller ---
//
// Both controllers are drawn in code on a 420 x 280 grid (no images from anyone else): the
// outline is a mirrored chain of curves, filled with a triangulated mesh, then traced with a thin line.

#[derive(Clone, Copy, PartialEq)]
pub enum Skin {
    DualSense,
    Xbox,
}

type Pt = (f32, f32);

/// Points along a cubic curve from `a` (excluded) to `d`.
fn cubic(out: &mut Vec<Pt>, a: Pt, b: Pt, c: Pt, d: Pt) {
    for i in 1..=12 {
        let t = i as f32 / 12.0;
        let u = 1.0 - t;
        let f = |a: f32, b: f32, c: f32, d: f32| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
        out.push((f(a.0, b.0, c.0, d.0), f(a.1, b.1, c.1, d.1)));
    }
}

/// Left half from the top centre down to the bottom centre as curves [control, control, end];
/// returns the whole outline, the right half mirrored.
fn mirrored(start: Pt, segs: &[[Pt; 3]]) -> Vec<Pt> {
    let mut l = vec![start];
    for s in segs {
        let cur = *l.last().unwrap();
        cubic(&mut l, cur, s[0], s[1], s[2]);
    }
    let n = l.len();
    let mut full = l.clone();
    full.extend(l[1..n - 1].iter().rev().map(|&(x, y)| (420.0 - x, y)));
    full
}

/// Ear clipping: triangle indices for a simple (non-self-intersecting) polygon.
fn triangulate(pts: &[Pt]) -> Vec<u32> {
    let cross = |a: Pt, b: Pt, c: Pt| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let area: f32 = (0..pts.len()).map(|i| cross((0.0, 0.0), pts[i], pts[(i + 1) % pts.len()])).sum();
    let s = area.signum();
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    let mut tris = vec![];
    while idx.len() > 3 {
        let n = idx.len();
        let tri = |i: usize| (idx[(i + n - 1) % n], idx[i], idx[(i + 1) % n]);
        let ear = (0..n).find(|&i| {
            let (a, b, c) = tri(i);
            let (a, b, c) = (pts[a], pts[b], pts[c]);
            cross(a, b, c) * s > 1e-6
                && !idx.iter().any(|&j| {
                    let q = pts[j];
                    q != a && q != b && q != c && cross(a, b, q) * s >= 0.0 && cross(b, c, q) * s >= 0.0 && cross(c, a, q) * s >= 0.0
                })
        });
        match ear {
            Some(i) => {
                let (a, b, c) = tri(i);
                tris.extend([a as u32, b as u32, c as u32]);
                idx.remove(i);
            }
            // only straight (collinear) points left to drop
            None => match (0..n).find(|&i| cross(pts[tri(i).0], pts[tri(i).1], pts[tri(i).2]).abs() <= 1e-6) {
                Some(i) => {
                    idx.remove(i);
                }
                None => break,
            },
        }
    }
    if idx.len() == 3 {
        tris.extend(idx.iter().map(|&v| v as u32));
    }
    tris
}

struct Shell {
    body: Vec<Pt>,
    body_t: Vec<u32>,
    plate: Vec<Pt>, // darker centre part (DualSense two-tone), may be empty
    plate_t: Vec<u32>,
}

fn ds5_body() -> Vec<Pt> {
    mirrored(
        (210.0, 56.0),
        &[
            [(190.0, 56.0), (170.0, 56.0), (152.0, 56.0)],
            [(128.0, 56.0), (116.0, 46.0), (92.0, 46.0)],
            [(64.0, 46.0), (50.0, 58.0), (44.0, 80.0)],
            [(34.0, 118.0), (22.0, 170.0), (20.0, 206.0)],
            [(18.0, 248.0), (40.0, 270.0), (68.0, 270.0)],
            [(96.0, 270.0), (108.0, 250.0), (118.0, 228.0)],
            [(132.0, 200.0), (160.0, 196.0), (210.0, 196.0)],
        ],
    )
}

fn shell(skin: Skin) -> &'static Shell {
    static DS: std::sync::OnceLock<Shell> = std::sync::OnceLock::new();
    static XB: std::sync::OnceLock<Shell> = std::sync::OnceLock::new();
    let make = |body: Vec<Pt>, plate: Vec<Pt>| Shell { body_t: triangulate(&body), plate_t: triangulate(&plate), body, plate };
    match skin {
        Skin::DualSense => DS.get_or_init(|| {
            // the black centre: from beside the touchpad down around the sticks to the arch
            let plate = mirrored(
                (210.0, 56.0),
                &[
                    [(190.0, 56.0), (166.0, 56.0), (148.0, 56.0)],
                    [(140.0, 120.0), (114.0, 160.0), (118.0, 228.0)],
                    [(132.0, 200.0), (160.0, 196.0), (210.0, 196.0)],
                ],
            );
            make(ds5_body(), plate)
        }),
        Skin::Xbox => XB.get_or_init(|| {
            let body = mirrored(
                (210.0, 64.0),
                &[
                    [(184.0, 64.0), (164.0, 50.0), (124.0, 48.0)],
                    [(84.0, 46.0), (58.0, 56.0), (46.0, 82.0)],
                    [(32.0, 120.0), (16.0, 182.0), (22.0, 222.0)],
                    [(28.0, 262.0), (64.0, 272.0), (86.0, 254.0)],
                    [(108.0, 236.0), (128.0, 210.0), (162.0, 206.0)],
                    [(184.0, 204.0), (196.0, 206.0), (210.0, 206.0)],
                ],
            );
            make(body, vec![])
        }),
    }
}

const WING: Color32 = Color32::from_rgb(36, 36, 40);
const PLATE: Color32 = Color32::from_rgb(23, 23, 26);
const WELL: Color32 = Color32::from_rgb(15, 15, 17);

/// Everything a part needs to draw itself.
struct Pen<'a> {
    p: &'a Painter,
    o: Pos2,
    k: f32,
    s: &'a Status,
    acc: Color32,
    ink: Color32,
    line: Color32,
    st: Stroke,
    hot: Stroke,
    live: bool,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.o + vec2(x, y) * self.k
    }
    fn on(&self, bit: u16) -> bool {
        self.s.pad.buttons & bit != 0
    }
    fn pick(&self, on: bool) -> (Color32, Stroke) {
        if on { (self.acc, self.hot) } else { (WELL, self.st) }
    }
    fn font(&self, size: f32) -> FontId {
        FontId::proportional(size * self.k)
    }
    fn fill(&self, pts: &[Pt], tris: &[u32], color: Color32) {
        if pts.is_empty() {
            return;
        }
        let mut m = egui::Mesh::default();
        for &(x, y) in pts {
            m.colored_vertex(self.at(x, y), color);
        }
        m.indices = tris.to_vec();
        self.p.add(Shape::mesh(m));
    }
    fn trace(&self, pts: &[Pt], stroke: Stroke) {
        if !pts.is_empty() {
            self.p.add(Shape::closed_line(pts.iter().map(|&(x, y)| self.at(x, y)).collect(), stroke));
        }
    }
    /// Convex shape relative to a centre, filled when pressed.
    fn key(&self, c: Pt, pts: &[Pt], on: bool) {
        let (fill, stroke) = self.pick(on);
        self.p.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| self.at(c.0 + x, c.1 + y)).collect(), fill, stroke));
    }
    fn trigger(&self, x0: f32, y0: f32, w: f32, v: u8, name: &str) {
        let t = Rect::from_min_max(self.at(x0, y0), self.at(x0 + w, y0 + 30.0));
        let cr = CornerRadius { nw: (14.0 * self.k) as u8, ne: (14.0 * self.k) as u8, sw: (3.0 * self.k) as u8, se: (3.0 * self.k) as u8 };
        self.p.rect_filled(t, cr, WELL);
        if v > 0 {
            let h = t.height() * v as f32 / 255.0;
            self.p.rect_filled(Rect::from_min_max(pos2(t.left(), t.bottom() - h), t.max), if h > 14.0 * self.k { cr } else { CornerRadius::ZERO }, self.acc);
        }
        self.p.rect_stroke(t, cr, if v > 0 { self.hot } else { self.st }, StrokeKind::Inside);
        self.p.text(t.center(), Align2::CENTER_CENTER, name, self.font(9.0), if v > 150 { self.ink } else { MUTED });
    }
    fn bumper(&self, x0: f32, y0: f32, w: f32, bit: u16, name: &str) {
        let b = Rect::from_min_max(self.at(x0, y0), self.at(x0 + w, y0 + 9.0));
        let (fill, stroke) = self.pick(self.on(bit));
        self.p.rect_filled(b, 4.5 * self.k, fill);
        self.p.rect_stroke(b, 4.5 * self.k, stroke, StrokeKind::Inside);
        self.p.text(b.center(), Align2::CENTER_CENTER, name, self.font(7.0), if self.on(bit) { self.ink } else { MUTED });
    }
    fn round(&self, c: Pt, r: f32, on: bool) {
        let (fill, stroke) = self.pick(on);
        self.p.circle_filled(self.at(c.0, c.1), r * self.k, fill);
        self.p.circle_stroke(self.at(c.0, c.1), r * self.k, stroke);
    }
    /// Well, cap that follows the stick, a line from the centre while it is pushed.
    fn stick(&self, c: Pt, x: f32, y: f32, bit: u16) {
        let k = self.k;
        let cc = self.at(c.0, c.1);
        self.p.circle_filled(cc, 23.0 * k, WELL);
        self.p.circle_stroke(cc, 23.0 * k, self.st);
        let d = cc + vec2(x, -y) * 9.0 * k;
        let moved = x != 0.0 || y != 0.0;
        let pressed = self.on(bit);
        self.p.circle_filled(d, 15.0 * k, if pressed { self.acc } else { WING });
        self.p.circle_stroke(d, 15.0 * k, if moved || pressed { self.hot } else { self.st });
        self.p.circle_stroke(d, 9.5 * k, Stroke::new(self.st.width * 0.8, if pressed { self.ink.gamma_multiply(0.5) } else { self.line.gamma_multiply(0.5) }));
        if moved {
            self.p.line_segment([cc, d], Stroke::new(1.4 * k, self.acc));
            self.p.circle_filled(cc, 2.0 * k, self.acc);
        }
    }
    fn symbol_stroke(&self, pressed: bool) -> Stroke {
        Stroke::new((1.0 * self.k).max(0.9), if pressed { self.ink } else { TEXT.gamma_multiply(if self.live { 0.7 } else { 0.4 }) })
    }
}

/// The live controller, fitted into `area`.
fn controller(p: &Painter, area: Rect, s: &Status, skin: Skin) {
    let k = (area.width() / 420.0).min(area.height() / 280.0);
    let live = s.controller != 0;
    let acc = accent();
    let line = TEXT.gamma_multiply(if live { 0.42 } else { 0.25 });
    let sw = k.max(1.0);
    let pen = Pen {
        p,
        o: area.center() - vec2(210.0, 140.0) * k,
        k,
        s,
        acc,
        ink: on_accent(acc),
        line,
        st: Stroke::new(sw, line),
        hot: Stroke::new(sw, acc),
        live,
    };
    match skin {
        Skin::DualSense => dualsense(&pen),
        Skin::Xbox => xbox(&pen),
    }
}

fn dualsense(pen: &Pen) {
    let (s, k) = (pen.s, pen.k);
    pen.trigger(70.0, 0.0, 56.0, s.pad.lt, "L2");
    pen.trigger(294.0, 0.0, 56.0, s.pad.rt, "R2");
    pen.bumper(62.0, 32.0, 72.0, LB, "L1");
    pen.bumper(286.0, 32.0, 72.0, RB, "R1");

    let sh = shell(Skin::DualSense);
    pen.fill(&sh.body, &sh.body_t, WING);
    pen.fill(&sh.plate, &sh.plate_t, PLATE);
    pen.trace(&sh.plate, Stroke::new(pen.st.width, pen.line.gamma_multiply(0.55)));
    pen.trace(&sh.body, pen.st);

    // touchpad: flat top, bottom edge curving down in the middle
    let mut pad = vec![(152.0, 60.0), (268.0, 60.0), (268.0, 106.0)];
    cubic(&mut pad, (268.0, 106.0), (250.0, 122.0), (226.0, 124.0), (210.0, 124.0));
    cubic(&mut pad, (210.0, 124.0), (194.0, 124.0), (170.0, 122.0), (152.0, 106.0));
    pen.p.add(Shape::convex_polygon(pad.iter().map(|&(x, y)| pen.at(x, y)).collect(), WELL, pen.st));
    // light bars beside the touchpad
    if pen.live {
        for (a, b) in [((146.0, 66.0), (143.0, 108.0)), ((274.0, 66.0), (277.0, 108.0))] {
            pen.p.line_segment([pen.at(a.0, a.1), pen.at(b.0, b.1)], Stroke::new(2.0 * k, pen.acc.gamma_multiply(0.75)));
        }
    }
    // create / options
    for (x, bit) in [(134.0, BACK), (286.0, START)] {
        let (fill, stroke) = pen.pick(pen.on(bit));
        let r = Rect::from_center_size(pen.at(x, 70.0), vec2(5.0, 13.0) * k);
        pen.p.rect_filled(r, 2.5 * k, fill);
        pen.p.rect_stroke(r, 2.5 * k, stroke, StrokeKind::Inside);
    }
    // d-pad: four arrow keys pointing at the centre
    let arrow = [(-6.5, -21.0), (6.5, -21.0), (6.5, -9.0), (0.0, -3.5), (-6.5, -9.0)];
    for (rot, bit) in [(0, UP), (1, RIGHT), (2, DOWN), (3, LEFT)] {
        let pts: Vec<Pt> = arrow.iter().map(|&(x, y)| match rot { 0 => (x, y), 1 => (-y, x), 2 => (-x, -y), _ => (y, -x) }).collect();
        pen.key((96.0, 112.0), &pts, pen.on(bit));
    }
    // face buttons with thin symbols
    let fc = (324.0, 112.0);
    for (i, (dx, dy, bit)) in [(0.0, -1.0, Y), (1.0, 0.0, B), (0.0, 1.0, A), (-1.0, 0.0, X)].into_iter().enumerate() {
        let c = (fc.0 + dx * 21.0, fc.1 + dy * 21.0);
        let pressed = pen.on(bit);
        pen.round(c, 10.0, pressed);
        let (cc, m, st) = (pen.at(c.0, c.1), 4.4 * k, pen.symbol_stroke(pressed));
        match i {
            0 => {
                pen.p.add(Shape::closed_line(vec![cc + vec2(0.0, -m), cc + vec2(m * 0.95, m * 0.7), cc + vec2(-m * 0.95, m * 0.7)], st));
            }
            1 => {
                pen.p.circle_stroke(cc, m, st);
            }
            2 => {
                pen.p.line_segment([cc + vec2(-m, -m) * 0.8, cc + vec2(m, m) * 0.8], st);
                pen.p.line_segment([cc + vec2(-m, m) * 0.8, cc + vec2(m, -m) * 0.8], st);
            }
            _ => {
                pen.p.rect_stroke(Rect::from_center_size(cc, vec2(m, m) * 1.6), 1.0, st, StrokeKind::Middle);
            }
        }
    }
    // speaker grille, PS button, mute button
    for row in 0..3 {
        for col in 0..(7 - row % 2) {
            let x = 210.0 - (6 - row % 2) as f32 * 3.0 + col as f32 * 6.0;
            pen.p.circle_filled(pen.at(x, 134.0 + row as f32 * 5.0), 1.0 * k, pen.line);
        }
    }
    pen.round((210.0, 162.0), 8.0, pen.on(GUIDE));
    let mic = Rect::from_center_size(pen.at(210.0, 180.0), vec2(12.0, 4.0) * k);
    pen.p.rect_stroke(mic, 2.0 * k, pen.st, StrokeKind::Inside);

    pen.stick((156.0, 164.0), s.pad.lx, s.pad.ly, LS);
    pen.stick((264.0, 164.0), s.pad.rx, s.pad.ry, RS);
}

fn xbox(pen: &Pen) {
    let (s, k) = (pen.s, pen.k);
    pen.trigger(78.0, 2.0, 54.0, s.pad.lt, "LT");
    pen.trigger(288.0, 2.0, 54.0, s.pad.rt, "RT");
    pen.bumper(66.0, 34.0, 76.0, LB, "LB");
    pen.bumper(278.0, 34.0, 76.0, RB, "RB");

    let sh = shell(Skin::Xbox);
    pen.fill(&sh.body, &sh.body_t, WING);
    pen.trace(&sh.body, pen.st);

    // guide, view, menu
    pen.round((210.0, 84.0), 12.0, pen.on(GUIDE));
    let x = pen.at(210.0, 84.0);
    let st = pen.symbol_stroke(pen.on(GUIDE));
    pen.p.line_segment([x + vec2(-4.5, -4.5) * k, x + vec2(4.5, 4.5) * k], st);
    pen.p.line_segment([x + vec2(-4.5, 4.5) * k, x + vec2(4.5, -4.5) * k], st);
    for (cx, bit) in [(180.0, BACK), (240.0, START)] {
        pen.round((cx, 112.0), 6.5, pen.on(bit));
    }
    // left stick high, d-pad low, right stick low, buttons high
    pen.stick((110.0, 110.0), s.pad.lx, s.pad.ly, LS);
    let arm = [(-6.0, -20.0), (6.0, -20.0), (6.0, -6.0), (-6.0, -6.0)];
    for (rot, bit) in [(0, UP), (1, RIGHT), (2, DOWN), (3, LEFT)] {
        let pts: Vec<Pt> = arm.iter().map(|&(x, y)| match rot { 0 => (x, y), 1 => (-y, x), 2 => (-x, -y), _ => (y, -x) }).collect();
        pen.key((164.0, 164.0), &pts, pen.on(bit));
    }
    pen.stick((258.0, 164.0), s.pad.rx, s.pad.ry, RS);
    let fc = (314.0, 110.0);
    for (dx, dy, bit, name) in [(0.0, -1.0, Y, "Y"), (1.0, 0.0, B, "B"), (0.0, 1.0, A, "A"), (-1.0, 0.0, X, "X")] {
        let c = (fc.0 + dx * 21.0, fc.1 + dy * 21.0);
        let pressed = pen.on(bit);
        pen.round(c, 10.0, pressed);
        pen.p.text(pen.at(c.0, c.1), Align2::CENTER_CENTER, name, pen.font(10.0), pen.symbol_stroke(pressed).color);
    }
}

#[cfg(test)]
mod tests {
    /// The outlines must triangulate completely, or the controller would have holes.
    #[test]
    fn outlines_fill_completely() {
        use super::*;
        let area = |pts: &[Pt]| (0..pts.len()).map(|i| { let (a, b) = (pts[i], pts[(i + 1) % pts.len()]); a.0 * b.1 - b.0 * a.1 }).sum::<f32>().abs() / 2.0;
        let tri_area = |pts: &[Pt], t: &[u32]| t.chunks(3).map(|c| area(&[pts[c[0] as usize], pts[c[1] as usize], pts[c[2] as usize]])).sum::<f32>();
        for skin in [Skin::DualSense, Skin::Xbox] {
            let sh = shell(skin);
            assert!((area(&sh.body) - tri_area(&sh.body, &sh.body_t)).abs() < 1.0, "body");
            assert!((area(&sh.plate) - tri_area(&sh.plate, &sh.plate_t)).abs() < 1.0, "plate");
        }
        // an L shape, the simplest non-convex case
        let l = [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0), (1.0, 2.0), (0.0, 2.0)];
        assert_eq!(triangulate(&l).len(), 12);
    }

    #[test]
    fn click_wav_is_valid() {
        let w = super::sound::wav(2000.0, 10.0, 0.1);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        let n = 441; // 10 ms at 44.1 kHz
        assert_eq!(w.len(), 44 + n * 2);
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, 36 + n * 2);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize, n * 2);
        // quiet: no sample anywhere near full scale
        let peak = w[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        assert!(peak > 100 && peak < 6000, "peak {peak}");
    }
}
