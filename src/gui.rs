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
const BG: Color32 = Color32::from_rgb(5, 5, 7);
const SURFACE: Color32 = Color32::from_rgb(15, 16, 19);
const SURFACE2: Color32 = Color32::from_rgb(31, 31, 35);
const HOVER: Color32 = Color32::from_rgb(44, 44, 49);
const LINE: Color32 = Color32::from_rgb(32, 34, 40);
const TEXT: Color32 = Color32::from_rgb(242, 244, 248);
const MUTED: Color32 = Color32::from_rgb(137, 142, 153);
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
        Mode::Install => [880.0, 560.0],
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
                // a file name with "settings" in it shows the Settings tab (README screenshot)
                if render_to.as_ref().is_some_and(|p| p.to_string_lossy().contains("settings")) {
                    app.tab = Tab::Settings;
                }
                // "install-options" / "install-done": that installer page
                let name = render_to.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                app.step = if name.contains("options") { 1 } else if name.contains("done") { 4 } else { 0 };
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
    // serif titles (installer): Georgia, then everything above for other scripts
    let mut serif = list.clone();
    if load("serif", "georgia.ttf").is_some() {
        serif.insert(0, "serif".into());
    }
    fonts.families.insert(FontFamily::Name("serif".into()), serif);
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
    step: usize, // installer page
    step_at: std::time::Instant,
    vigem: bool,
    removed: bool,
    demo: Option<Status>, // --render: fixed values instead of the live bridge
    render_to: Option<std::path::PathBuf>,
    frames: u32,
    launched: std::time::Instant,
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
        if mode == Mode::Main {
            // redraw the moment the controller changes; no polling, sleeps while nothing happens
            let ctx = ctx.clone();
            let ev = ipc::changed_event();
            std::thread::spawn(move || loop {
                if unsafe { WaitForSingleObject(ev, 1000) } == 0 {
                    ctx.request_repaint();
                }
            });
        }
        let settings = settings::load();
        ACCENT.store(settings.accent, Relaxed);
        sound::ON.store(settings.sounds, Relaxed);
        visuals(ctx);
        let n = 128;
        let logo = ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([n, n], &crate::icon::rgba(n as u32)), Default::default());
        let app = App {
            mode,
            tab: Tab::Controller,
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
            step: 0,
            step_at: std::time::Instant::now(),
            vigem: crate::win::vigem_present(),
            removed: false,
            demo: None,
            render_to: None,
            frames: 0,
            launched: std::time::Instant::now(),
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
        if self.mode == Mode::Install {
            self.installer(ui, full);
        } else {
            self.window(ui, full);
        }
        self.after_frame(ui, full);
    }
}

impl App {
    fn window(&mut self, ui: &mut egui::Ui, full: Rect) {
        ui.painter().rect(full, RADIUS, BG, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let tab = if self.mode == Mode::Main { Some(&mut self.tab) } else { None };
        title_bar(ui, full, &self.logo, tab);

        if self.mode == Mode::Main {
            self.main_layout(ui, full);
        } else {
            let content = Rect::from_min_max(full.min + vec2(32.0, TITLE_H + 24.0), full.max - vec2(32.0, 24.0));
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(content));
            cui.spacing_mut().item_spacing = vec2(10.0, 10.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(&mut cui, |ui| self.uninstall_view(ui));
        }
    }

    fn after_frame(&mut self, ui: &mut egui::Ui, full: Rect) {

        // repaint smoothly only while something changes; otherwise nearly no CPU
        let s = self.sh.status();
        let busy = {
            let u = self.upd.lock().unwrap();
            u.checking || matches!(u.phase, Phase::Downloading | Phase::Installing)
        };
        let changed = s != self.last;
        self.last = s;
        // start screen: the mark draws itself, then the window fades in from behind it
        // (--render shows it frozen mid-way only when the file name asks for "splash")
        let shot = self.render_to.as_ref().map(|p| p.to_string_lossy().contains("splash"));
        let t = match shot { None => Some(self.launched.elapsed().as_secs_f32()), Some(true) => Some(0.40), Some(false) => None };
        if let Some(t) = t {
            let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("splash")));
            if splash(&p, full, t) && shot.is_none() {
                ui.ctx().request_repaint();
            }
        }
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
        crate::ps5::paint(&p, panel.shrink(30.0), &self.demo.clone().unwrap_or_default());
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.ui_mutex != 0 {
            unsafe { CloseHandle(self.ui_mutex) };
        }
    }
}

// -------------------------------------------------------------- brand ---

/// How long the mark takes to draw itself.
const DRAW_SECONDS: f32 = 0.30;
/// Start screen length (the mark draws, then the screen fades out).
const SPLASH_SECONDS: f32 = 0.67;
const FADE_FROM: f32 = 0.42;

fn smooth(v: f32) -> f32 {
    let t = v.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The first `progress` (0..1) of a polyline, with round ends.
fn trace(p: &Painter, mut pts: Vec<Pos2>, stroke: Stroke, progress: f32) {
    if progress < 1.0 {
        let total: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
        let mut left = total * progress;
        for i in 1..pts.len() {
            let len = pts[i - 1].distance(pts[i]);
            if left < len {
                pts[i] = pts[i - 1].lerp(pts[i], left / len);
                pts.truncate(i + 1);
                break;
            }
            left -= len;
        }
    }
    if pts.len() > 1 {
        p.circle_filled(pts[0], stroke.width * 0.5, stroke.color);
        p.circle_filled(*pts.last().unwrap(), stroke.width * 0.5, stroke.color);
        p.add(Shape::line(pts, stroke));
    }
}

/// The DS5 Bridge mark: one line, up, over an arch and down (a controller's top edge), then two
/// dots for the sticks. 40 x 40 grid, `scale` 1.0 = 40 points; `seconds` since it began drawing.
fn mark(p: &Painter, center: Pos2, scale: f32, ink: Color32, dots: Color32, seconds: f32) {
    let draw = smooth(seconds / DRAW_SECONDS);
    if draw <= 0.0 {
        return;
    }
    let o = center - vec2(20.0, 20.0) * scale;
    let mut pts = vec![pos2(6.0, 34.0)];
    for i in 0..=32 {
        let a = std::f32::consts::PI * (1.0 + i as f32 / 32.0);
        pts.push(pos2(20.0 + 14.0 * a.cos(), 20.0 + 14.0 * a.sin()));
    }
    pts.push(pos2(34.0, 34.0));
    trace(p, pts.into_iter().map(|q| o + q.to_vec2() * scale).collect(), Stroke::new(5.8 * scale, ink), draw);
    let pop = smooth((seconds - DRAW_SECONDS * 0.8) / 0.12);
    for x in [15.0, 25.0] {
        p.circle_filled(o + vec2(x, 29.0) * scale, 2.7 * scale * pop, dots);
    }
}

/// Start screen over the whole window at `seconds` after launch; false once it is gone.
fn splash(p: &Painter, full: Rect, seconds: f32) -> bool {
    if seconds >= SPLASH_SECONDS {
        return false;
    }
    let fade = 1.0 - smooth((seconds - FADE_FROM) / (SPLASH_SECONDS - FADE_FROM));
    p.rect_filled(full, RADIUS, BG.gamma_multiply(fade));
    let pop = smooth(seconds / 0.2);
    mark(p, full.center(), 1.6 * (0.92 + 0.08 * pop), TEXT.gamma_multiply(pop * fade), accent().gamma_multiply(fade), seconds - 0.05);
    true
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
        let panel = Rect::from_min_max(right.min + vec2(0.0, 24.0), right.max - vec2(0.0, 26.0));
        p.rect(panel, 20.0, SURFACE, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let preview = panel.shrink(14.0);
        crate::ps5::paint(&p, preview, &s);
        let hint = if s.controller == 0 {
            tr("Connect a DualSense to see it here.")
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
        field(ui, tr("Polling rate"), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (hz, label) in [(0, tr("Auto").to_string()), (500, "500 Hz".into()), (250, "250 Hz".into()), (125, "125 Hz".into())] {
                let on = self.settings.rate_limit == hz;
                let b = egui::Button::new(RichText::new(label).size(12.5).color(if on { on_accent(accent()) } else { TEXT }))
                    .fill(if on { accent() } else { SURFACE2 })
                    .corner_radius(7.0)
                    .min_size(vec2(0.0, 26.0));
                if ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                    sound::click();
                    self.settings.rate_limit = hz;
                    self.sh.rate_limit.store(hz, Relaxed);
                    self.save();
                }
            }
        });
        ui.label(RichText::new(tr("Auto passes on every report: about 250 Hz over USB, about 600 Hz over Bluetooth. More over USB needs a driver.")).size(12.0).color(MUTED));
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

// ------------------------------------------------------------ installer ---
//
// Dark stage with facts about the app on top, a light sheet with an arched top edge below.

const PAPER: Color32 = Color32::from_rgb(250, 249, 246);
const PAPER_INK: Color32 = Color32::from_rgb(22, 21, 26);
const PAPER_MUTED: Color32 = Color32::from_rgb(132, 128, 138);
const PAPER_FIELD: Color32 = Color32::from_rgb(238, 236, 232);
const SCENE: Color32 = Color32::from_rgb(12, 13, 17);
const STEPS: usize = 5;
const CARD_R: u8 = 22;

fn serif(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("serif".into()))
}

/// Checkbox for the paper side.
fn paper_check(ui: &mut egui::Ui, on: &mut bool, label: &str) -> Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(14.0), PAPER_INK);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(30.0 + g.size().x, 28.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        sound::click();
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let b = Rect::from_center_size(pos2(rect.left() + 9.0, rect.center().y), vec2(18.0, 18.0));
    let p = ui.painter();
    p.rect_filled(b, 6.0, mix(PAPER, PAPER_INK, t));
    p.rect_stroke(b, 6.0, Stroke::new(1.5, mix(PAPER_MUTED, PAPER_INK, t)), StrokeKind::Inside);
    if t > 0.0 {
        let c = b.center();
        p.add(Shape::line(vec![c + vec2(-4.5, 0.5), c + vec2(-1.3, 3.5), c + vec2(4.5, -3.2)], Stroke::new(1.8, PAPER.gamma_multiply(t))));
    }
    p.galley(pos2(b.right() + 11.0, rect.center().y - g.size().y / 2.0), g, PAPER_INK);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Little true facts about the app for the installer's stage: (big value, caption).
fn facts() -> [(&'static str, &'static str); 6] {
    [
        ("~2 MB", tr("of memory while it runs in the background")),
        ("592 Hz", tr("reports per second over Bluetooth, measured")),
        ("0 bytes", tr("of your data ever leave your PC")),
        ("1 file", tr("is the whole app. No runtimes, no extra packages")),
        ("8", tr("languages, from Deutsch to 日本語")),
        ("Rust", tr("all the way down, talking straight to Windows")),
    ]
}

impl App {
    fn go(&mut self, step: usize) {
        self.step = step.min(STEPS - 1);
        self.step_at = std::time::Instant::now();
    }

    /// Seconds since the current step appeared (renders count as settled).
    fn step_time(&self) -> f32 {
        if self.render_to.is_some() { 5.0 } else { self.step_at.elapsed().as_secs_f32() }
    }

    /// Dark stage on top with rotating facts; a light sheet below whose top edge is an arch (the
    /// logo's shape). A line along the arch shows how far through the steps you are.
    fn installer(&mut self, ui: &mut egui::Ui, full: Rect) {
        let p = ui.painter().clone();
        let drag = ui.interact(full, ui.scope_id().with("drag"), Sense::click_and_drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let now = if self.render_to.is_some() { 0.6 } else { self.launched.elapsed().as_secs_f32() };
        let acc = accent();

        // ---- stage
        p.rect_filled(full, CARD_R, SCENE);
        // a soft glow: many faint discs blend into a smooth falloff
        for i in 0..14 {
            p.circle_filled(pos2(full.right() - 170.0, full.top() + 60.0), 300.0 - i as f32 * 20.0, acc.gamma_multiply(0.008));
        }
        // logo and name
        mark(&p, pos2(full.left() + 46.0, full.top() + 40.0), 0.55, TEXT, acc, 9.0);
        p.text(pos2(full.left() + 66.0, full.top() + 40.0), Align2::LEFT_CENTER, APP, FontId::proportional(14.0), TEXT);
        // minimise / close
        for (i, is_close) in [(1.0, false), (0.0, true)] {
            let c = pos2(full.right() - 32.0 - i * 32.0, full.top() + 32.0);
            let r = ui.interact(Rect::from_center_size(c, vec2(26.0, 26.0)), ui.scope_id().with(("w", is_close)), Sense::click());
            p.circle_filled(c, 13.0, Color32::from_white_alpha(if r.hovered() { 46 } else { 16 }));
            let st = Stroke::new(1.3, Color32::from_white_alpha(if r.hovered() { 255 } else { 180 }));
            if is_close {
                p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], st);
                p.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], st);
            } else {
                p.line_segment([c + vec2(-4.5, 0.0), c + vec2(4.5, 0.0)], st);
            }
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                ui.ctx().send_viewport_cmd(if is_close { ViewportCommand::Close } else { ViewportCommand::Minimized(true) });
            }
        }
        // the fact slides: a new one every 3.2 s, sliding in from the right
        let f = facts();
        let slot = 3.2;
        let idx = (now / slot) as usize % f.len();
        let k = smooth((now % slot) / 0.45);
        let (value, caption) = (tr(f[idx].0), f[idx].1);
        let x = full.left() + 46.0 + 30.0 * (1.0 - k);
        let ink = |c: Color32| c.gamma_multiply(k);
        p.text(pos2(x, full.top() + 84.0), Align2::LEFT_TOP, tr("DID YOU KNOW"), FontId::proportional(11.0), ink(MUTED));
        let big = p.layout_no_wrap(value.to_string(), serif(64.0), TEXT);
        let big_w = big.size().x;
        p.galley(pos2(x - 2.0, full.top() + 100.0), big, ink(TEXT));
        let cap = p.layout(caption.to_string(), FontId::proportional(16.0), TEXT, 300.0);
        let cap_h = cap.size().y;
        p.galley(pos2(x + big_w + 22.0, full.top() + 140.0 - cap_h / 2.0), cap, ink(Color32::from_rgb(190, 188, 182)));
        // slide markers on the right of the stage
        for i in 0..f.len() {
            let r = Rect::from_center_size(pos2(full.right() - 40.0, full.top() + 104.0 + i as f32 * 12.0), vec2(4.0, if i == idx { 12.0 } else { 4.0 }));
            p.rect_filled(r, 2.0, Color32::from_white_alpha(if i == idx { 220 } else { 60 }));
        }
        ui.ctx().request_repaint_after(Duration::from_millis(if self.render_to.is_some() { 1000 } else { 33 }));

        // ---- sheet with an arched top edge, drawn as thin convex strips
        let top = full.top() + 228.0;
        let rise = 26.0;
        let arch = |x: f32| top + rise * (1.0 - (std::f32::consts::PI * (x - full.left()) / full.width()).sin());
        let n = 88;
        let xs: Vec<f32> = (0..=n).map(|i| full.left() + full.width() * i as f32 / n as f32).collect();
        for w in xs.windows(2) {
            let (x0, x1) = (w[0], w[1]);
            let bottom = full.bottom() - if x0 < full.left() + CARD_R as f32 || x1 > full.right() - CARD_R as f32 { CARD_R as f32 } else { 0.0 };
            p.add(Shape::convex_polygon(vec![pos2(x0, arch(x0)), pos2(x1, arch(x1)), pos2(x1, bottom), pos2(x0, bottom)], PAPER, Stroke::NONE));
        }
        let base = Rect::from_min_max(pos2(full.left(), full.bottom() - CARD_R as f32 - 1.0), full.max);
        p.rect_filled(base, CornerRadius { sw: CARD_R, se: CARD_R, ..Default::default() }, PAPER);
        // progress along the arch: one fifth per step, the install step fills itself
        let done = match self.step {
            3 => (3.0 + smooth(self.step_time() / 1.1)) / STEPS as f32,
            s => (s + 1) as f32 / STEPS as f32,
        };
        let edge: Vec<Pos2> = xs.iter().map(|&x| pos2(x, arch(x))).collect();
        trace(&p, edge, Stroke::new(3.0, acc), done);

        // ---- the step's content
        let kk = smooth(self.step_time() / 0.35);
        let area = Rect::from_min_max(pos2(full.left() + 48.0, top + rise + 22.0 + 10.0 * (1.0 - kk)), pos2(full.right() - 48.0, full.bottom() - 26.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(area.min, pos2(area.right(), area.bottom() - 56.0))));
        c.set_opacity(kk);
        {
            let v = c.visuals_mut();
            v.override_text_color = Some(PAPER_INK);
            v.widgets.inactive.weak_bg_fill = PAPER_FIELD;
            v.widgets.inactive.bg_fill = PAPER_FIELD;
            v.widgets.hovered.weak_bg_fill = Color32::from_rgb(228, 225, 220);
            v.widgets.hovered.bg_fill = Color32::from_rgb(228, 225, 220);
            v.widgets.inactive.fg_stroke = Stroke::new(1.0, PAPER_INK);
            v.widgets.hovered.fg_stroke = Stroke::new(1.0, PAPER_INK);
        }
        c.spacing_mut().item_spacing = vec2(8.0, 6.0);
        let title = match self.step {
            0 => tr("Your DualSense, at home on Windows"),
            1 => tr("Make it yours"),
            2 => tr("One small driver"),
            3 => tr("Setting things up"),
            _ => tr("Ready to play"),
        };
        c.horizontal(|ui| {
            ui.label(RichText::new(title).font(serif(30.0)).color(PAPER_INK));
            ui.add_space(10.0);
            ui.label(RichText::new(trf("Step {} of {}", &[&(self.step + 1), &STEPS])).size(12.0).color(PAPER_MUTED));
        });
        c.add_space(10.0);

        let (mut action, mut note, mut back) = (None, String::new(), false);
        match self.step {
            0 => {
                c.columns(3, |cols| {
                    for (col, (t, d)) in cols.iter_mut().zip([
                        (tr("USB and Bluetooth"), tr("Cable or wireless, both just work.")),
                        (tr("Every Xbox game"), tr("Games see a normal Xbox controller.")),
                        (tr("Private"), tr("Offline. No account. No telemetry.")),
                    ]) {
                        egui::Frame::NONE.fill(PAPER_FIELD).corner_radius(14.0).inner_margin(14.0).show(col, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(t).size(14.5).color(PAPER_INK));
                            ui.label(RichText::new(d).size(12.5).color(PAPER_MUTED));
                        });
                    }
                });
                action = Some(tr("Get started"));
                note = tr("Free and open source. Nothing leaves your PC.").to_string();
            }
            1 => {
                c.columns(2, |cols| {
                    let labels = [tr("Start menu shortcut"), tr("Desktop shortcut"), tr("Start with Windows"), tr("Check for updates")];
                    for (i, t) in labels.into_iter().enumerate() {
                        paper_check(&mut cols[i / 2], &mut self.opt[i], t);
                    }
                });
                c.add_space(6.0);
                c.horizontal(|ui| {
                    ui.label(RichText::new(tr("Language")).size(14.0).color(PAPER_MUTED));
                    ui.add_space(8.0);
                    self.language_picker(ui);
                });
                action = Some(tr("Continue"));
                // shown as %LOCALAPPDATA% so screenshots never contain a user name
                note = trf("Installs to {}", &[&r"%LOCALAPPDATA%\Programs\DS5 Bridge"]);
                back = true;
            }
            2 => {
                c.horizontal(|ui| {
                    if self.vigem {
                        dot(ui, Color32::from_rgb(70, 150, 90));
                        ui.label(RichText::new(tr("ViGEmBus driver found")).size(14.5));
                    } else {
                        dot(ui, Color32::from_rgb(200, 110, 70));
                        ui.label(RichText::new(tr("ViGEmBus driver missing: needed for the virtual controller")).size(14.5));
                    }
                });
                c.label(RichText::new(tr("ViGEmBus creates the virtual Xbox controller that games see. Open source, by Nefarius.")).size(13.0).color(PAPER_MUTED));
                if !self.vigem {
                    c.add_space(4.0);
                    c.horizontal(|ui| {
                        if ui.add(egui::Label::new(RichText::new(tr("Get ViGEmBus (open source)")).size(13.5).underline()).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            open_url("https://github.com/nefarius/ViGEmBus/releases");
                        }
                        ui.add_space(10.0);
                        if ui.add(egui::Label::new(RichText::new(tr("Check again")).size(13.5).color(PAPER_MUTED).underline()).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.vigem = crate::win::vigem_present();
                        }
                    });
                }
                action = Some(tr("Install"));
                note = if self.vigem { tr("No admin rights needed.") } else { tr("You can also install it later. DS5 Bridge will remind you.") }.to_string();
                back = true;
            }
            3 => {
                let t = self.step_time();
                let failed = if let Inst::Failed(e) = &self.inst { Some(e.clone()) } else { None };
                match failed {
                    Some(e) => {
                        c.label(RichText::new(trf("Installation failed: {}", &[&e])).size(13.5).color(Color32::from_rgb(190, 80, 60)));
                        action = Some(tr("Try again"));
                    }
                    None => {
                        let msg = if t < 0.4 { tr("Copying DS5 Bridge…") } else if t < 0.8 { tr("Creating shortcuts…") } else { tr("Almost done…") };
                        c.label(RichText::new(msg).size(14.0).color(PAPER_MUTED));
                        if t > 1.25 && self.render_to.is_none() {
                            self.go(4);
                        }
                    }
                }
            }
            _ => {
                c.label(RichText::new(tr("Plug in your DualSense or pair it over Bluetooth. DS5 Bridge waits in the tray, bottom right.")).size(14.0));
                action = Some(tr("Open DS5 Bridge"));
                note = tr("No account, no telemetry. Your data stays on this PC.").to_string();
            }
        }

        // ---- bottom row: note on the left, back + one filled pill on the right
        let row = Rect::from_min_max(pos2(area.left(), area.bottom() - 40.0), area.max);
        let mut right_x = row.right();
        if let Some(label) = action {
            let g = p.layout_no_wrap(label.to_string(), FontId::proportional(14.5), PAPER);
            let r = Rect::from_min_max(pos2(row.right() - g.size().x - 58.0, row.top()), row.max);
            let resp = ui.interact(r, ui.scope_id().with("go"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let h = ui.ctx().animate_bool(resp.id, resp.hovered());
            p.rect_filled(r, 20.0, mix(PAPER_INK, Color32::from_rgb(52, 50, 58), h).gamma_multiply(kk));
            p.galley(pos2(r.left() + 22.0, r.center().y - g.size().y / 2.0), g, PAPER);
            let a = pos2(r.right() - 22.0 + h * 3.0, r.center().y);
            let st = Stroke::new(1.6, acc);
            p.line_segment([a - vec2(9.0, 0.0), a + vec2(3.0, 0.0)], st);
            p.line_segment([a + vec2(3.0, 0.0), a + vec2(-2.0, -5.0)], st);
            p.line_segment([a + vec2(3.0, 0.0), a + vec2(-2.0, 5.0)], st);
            right_x = r.left() - 18.0;
            if resp.clicked() {
                sound::click();
                match self.step {
                    0 | 1 => self.go(self.step + 1),
                    2 => {
                        let o = installer::Options { start_menu: self.opt[0], desktop: self.opt[1], autostart: self.opt[2] };
                        self.settings.check_updates = self.opt[3];
                        self.save();
                        self.inst = match installer::install(&o) {
                            Ok(()) => Inst::Done,
                            Err(e) => Inst::Failed(e),
                        };
                        self.go(3);
                    }
                    3 => {
                        self.inst = Inst::Ready;
                        self.go(2);
                    }
                    _ => {
                        let _ = std::process::Command::new(installer::installed_exe()).spawn();
                        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                    }
                }
            }
        }
        if back {
            let g = p.layout_no_wrap(tr("Back").to_string(), FontId::proportional(13.5), PAPER_MUTED);
            let r = Rect::from_min_max(pos2(right_x - g.size().x, row.top()), pos2(right_x, row.bottom()));
            let resp = ui.interact(r, ui.scope_id().with("back"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            p.galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, if resp.hovered() { PAPER_INK } else { PAPER_MUTED });
            right_x = r.left() - 18.0;
            if resp.clicked() {
                sound::click();
                self.go(self.step - 1);
            }
        }
        if !note.is_empty() {
            let g = p.layout(note, FontId::proportional(12.5), PAPER_MUTED, (right_x - row.left()).min(380.0));
            p.galley(pos2(row.left(), row.center().y - g.size().y / 2.0), g, PAPER_MUTED.gamma_multiply(kk));
        }
    }
}

#[cfg(test)]
mod tests {
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
