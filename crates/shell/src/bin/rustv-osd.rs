//! `rustv-osd`: what rustv shows over everything on the TV, apps included.
//!
//! Only a renderer: tvd starts it, writes one JSON line on stdin per change and
//! closes stdin to end it. It draws on sway's overlay layer, so it shows over
//! full screen apps, and never takes input or focus. The first argument picks
//! what it is:
//!
//! - `keyboard`: the on-screen keyboard (the keys, the focus, what was typed).
//!   tvd moves the focus and types into the app.
//! - `level`: a volume or brightness bar, drawn like the home screen's.
//! - `prompt`: the "keep this resolution?" prompt over an app, drawn like the
//!   home screen's, with its countdown. tvd takes OK and Back for it.
//! - `gamma`: no picture; holds sway's gamma tables to dim the screen. When it
//!   ends, by design or by a crash, sway puts full brightness back.
//! - `type`: no picture; types text into whatever has focus through the Wayland
//!   virtual keyboard, with a keymap made for that text, so any character works
//!   (ğ, ş, ı, é, ß, emoji), whatever layout the box has. Answers `done` per line.

use std::io::{BufRead, Seek, Write};
use std::os::fd::AsFd;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use anyhow::{Context, Result};
use serde::Deserialize;
use smithay_client_toolkit::{compositor::Region, reexports::client::Dispatch};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::client::{
        globals::registry_queue_init,
        protocol::{wl_output, wl_shm, wl_surface},
        Connection, QueueHandle,
    },
    reexports::protocols_wlr::gamma_control::v1::client::{
        zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
        zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

const FONT: &[u8] = include_bytes!("../../ui/fonts/HankenGrotesk-Medium.ttf");
/// At 1920 wide: the typed line, five rows of keys and the hints, with room around them.
const PANEL_HEIGHT: f32 = 32.0 + 64.0 + 22.0 + 5.0 * (72.0 + 10.0) + 60.0;
/// At 1920 wide: the level bar's strip along the bottom, with the bar where the
/// home screen draws its own (230 px up from the bottom).
const LEVEL_HEIGHT: f32 = 230.0;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Keyboard,
    Level,
    Prompt,
}

/// What a surface shows.
enum Scene {
    Board(Board),
    Level(Level),
    Prompt(Prompt),
}

/// A question over the whole screen, with the keys that answer it. Every text is
/// already in the TV's language.
#[derive(Debug, Deserialize)]
struct Prompt {
    title: String,
    /// Such as "1920 × 1080, 60 Hz".
    detail: String,
    /// Such as "Going back in 12 s".
    countdown: String,
    /// Key and what it does: the first is the answer that keeps, so it reads in text color.
    keys: Vec<(String, String)>,
    palette: Palette,
}

/// A volume or brightness level.
#[derive(Debug, Deserialize)]
struct Level {
    /// "Volume", "Muted", "Brightness": already in the TV's language.
    label: String,
    /// 0..1 of the bar.
    value: f32,
    /// The number shown at the end, such as "45%".
    text: String,
    /// A muted volume: the bar in the quiet color.
    #[serde(default)]
    muted: bool,
    palette: Palette,
}

/// For `gamma`: how bright the screen is, 0..1, and how warm (the night light), 0..1.
#[derive(Debug, Deserialize)]
struct Dim {
    brightness: f32,
    #[serde(default)]
    warmth: f32,
}

/// One update from tvd.
#[derive(Debug, Default, Deserialize)]
struct Board {
    rows: Vec<Vec<Key>>,
    /// What was typed since the keyboard opened, so it reads even when the
    /// app's own field is hidden behind the keyboard.
    #[serde(default)]
    preview: String,
    /// Key hints along the bottom, such as "Back  Close".
    #[serde(default)]
    hint: String,
    palette: Palette,
}

#[derive(Debug, Deserialize)]
struct Key {
    label: String,
    /// In key units; a letter is 1.
    #[serde(default = "one")]
    width: f32,
    #[serde(default)]
    focused: bool,
    /// A modifier that is on, such as Shift.
    #[serde(default)]
    active: bool,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Default, Deserialize)]
struct Palette {
    background: String,
    surface: String,
    surface_2: String,
    surface_3: String,
    accent: String,
    on_accent: String,
    text: String,
    muted: String,
}

fn color(hex: &str) -> Color {
    let digits = hex.trim_start_matches('#');
    let channel = |i: usize| digits.get(i..i + 2).and_then(|d| u8::from_str_radix(d, 16).ok()).unwrap_or(0x80);
    Color::from_rgba8(channel(0), channel(2), channel(4), 0xff)
}

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("keyboard") => surface(Mode::Keyboard),
        Some("level") => surface(Mode::Level),
        Some("prompt") => surface(Mode::Prompt),
        Some("gamma") => gamma(),
        Some("type") => typing(),
        _ => anyhow::bail!("usage: rustv-osd keyboard|level|prompt|gamma|type (tvd starts it; JSON lines on stdin)"),
    }
}

fn surface(mode: Mode) -> Result<()> {
    let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
    let (globals, mut queue) = registry_queue_init(&conn)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).context("no wl_compositor")?;
    let layer_shell = LayerShell::bind(&globals, &qh).context("the compositor has no layer shell")?;
    let shm = Shm::bind(&globals, &qh).context("no wl_shm")?;

    // Along the bottom (a prompt: the whole screen), over everything, and never
    // focused: the app keeps the keyboard. An empty input region lets pointer
    // events through to the app as well.
    let surface = compositor.create_surface(&qh);
    let region = Region::new(&compositor)?;
    surface.set_input_region(Some(region.wl_region()));
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("rustv-osd"), None);
    let edges = Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT;
    layer.set_anchor(if mode == Mode::Prompt { edges | Anchor::TOP } else { edges });
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    layer.set_size(0, if mode == Mode::Prompt { 0 } else { 1 });
    layer.commit();

    let mut app = App {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        pool: SlotPool::new(1920 * 480 * 4, &shm)?,
        shm,
        layer,
        width: 0,
        height: 0,
        closed: false,
        font: FontRef::try_from_slice(FONT)?,
        mode,
        scene: None,
    };
    // Wait for the first size from sway.
    while app.width == 0 && !app.closed {
        queue.blocking_dispatch(&mut app)?;
    }

    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let scene = match mode {
            Mode::Keyboard => serde_json::from_str(&line).map(Scene::Board),
            Mode::Level => serde_json::from_str(&line).map(Scene::Level),
            Mode::Prompt => serde_json::from_str(&line).map(Scene::Prompt),
        };
        let Ok(scene) = scene else { continue };
        app.scene = Some(scene);
        app.draw(&qh)?;
        // Handles buffer releases and resizes, and notices a closed surface.
        queue.roundtrip(&mut app)?;
        if app.closed {
            break;
        }
    }
    Ok(())
}

struct App {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    width: u32,
    height: u32,
    closed: bool,
    font: FontRef<'static>,
    mode: Mode,
    scene: Option<Scene>,
}

impl App {
    fn draw(&mut self, _qh: &QueueHandle<Self>) -> Result<()> {
        let (width, height) = (self.width, self.height);
        let Some(scene) = &self.scene else { return Ok(()) };
        if width == 0 || height == 0 {
            return Ok(());
        }
        let mut pixmap = Pixmap::new(width, height).context("pixmap")?;
        match scene {
            Scene::Board(board) => paint(&mut pixmap, board, &self.font),
            Scene::Level(level) => paint_level(&mut pixmap, level, &self.font),
            Scene::Prompt(prompt) => paint_prompt(&mut pixmap, prompt, &self.font),
        }

        let stride = width as i32 * 4;
        let (buffer, canvas) =
            self.pool.create_buffer(width as i32, height as i32, stride, wl_shm::Format::Argb8888)?;
        // tiny-skia is premultiplied RGBA; wl_shm's ARGB8888 is premultiplied BGRA in memory.
        for (out, px) in canvas.chunks_exact_mut(4).zip(pixmap.data().chunks_exact(4)) {
            out.copy_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        buffer.attach_to(surface)?;
        self.layer.commit();
        Ok(())
    }
}

/// The panel: a typed-text line, the keys, the hints. Sized from the output
/// width, so it looks the same at 720p and 4K.
fn paint(pixmap: &mut Pixmap, board: &Board, font: &FontRef) {
    let palette = &board.palette;
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let s = w / 1920.0;

    // Rounded only at the top, so it reads as rising from the bottom edge.
    fill(pixmap, rounded(0.0, 0.0, w, h + 40.0 * s, 36.0 * s), color(&palette.surface));

    let width = (1500.0 * s).min(w - 64.0 * s);
    let left = (w - width) / 2.0;
    let gap = 10.0 * s;
    let key_h = 72.0 * s;

    // What was typed, on a recessed line.
    let line_y = 32.0 * s;
    fill(pixmap, rounded(left, line_y, width, 64.0 * s, 18.0 * s), color(&palette.background));
    let typed = if board.preview.is_empty() { "…" } else { board.preview.as_str() };
    let tint = if board.preview.is_empty() { &palette.muted } else { &palette.text };
    // Keep the end of long text in view.
    let mut shown = typed.to_string();
    while text_width(font, 30.0 * s, &shown) > width - 48.0 * s && shown.chars().count() > 1 {
        shown = shown.chars().skip(1).collect();
    }
    text(pixmap, font, 30.0 * s, &shown, left + 24.0 * s, line_y + 32.0 * s, color(tint), false);

    let units = board.rows.iter().map(|row| row.iter().map(|k| k.width).sum::<f32>()).fold(0.0, f32::max);
    // Room for 11 letters at least, so the US rows keep their size; wider layouts shrink to fit.
    let units = units.max(11.0);
    let unit = (width - gap * (units - 1.0)) / units;
    let mut y = line_y + 64.0 * s + 22.0 * s;
    for row in &board.rows {
        let row_w: f32 = row.iter().map(|k| k.width * unit + gap).sum::<f32>() - gap;
        let mut x = left + (width - row_w) / 2.0;
        for key in row {
            let key_w = key.width * unit + gap * (key.width - 1.0).max(0.0);
            let (face, ink) = if key.focused {
                (color(&palette.accent), color(&palette.on_accent))
            } else if key.active {
                (color(&palette.surface_3), color(&palette.accent))
            } else {
                (color(&palette.surface_2), color(&palette.text))
            };
            fill(pixmap, rounded(x, y, key_w, key_h, 16.0 * s), face);
            text(pixmap, font, 30.0 * s, &key.label, x + key_w / 2.0, y + key_h / 2.0, ink, true);
            x += key_w + gap;
        }
        y += key_h + gap;
    }
    if !board.hint.is_empty() {
        text(pixmap, font, 22.0 * s, &board.hint, left, y + 26.0 * s, color(&palette.muted), false);
    }
}

/// The level bar: a graphite panel with the label, a metal fill and the value,
/// the same as the home screen's volume overlay, centered over a clear strip.
fn paint_level(pixmap: &mut Pixmap, level: &Level, font: &FontRef) {
    let palette = &level.palette;
    let w = pixmap.width() as f32;
    let s = w / 1920.0;
    let (panel_w, panel_h) = (620.0 * s, 96.0 * s);
    let (x, y) = ((w - panel_w) / 2.0, 0.0);
    // A soft shadow under it, as for every floating panel.
    for (spread, alpha) in [(18.0, 0.10), (10.0, 0.14), (4.0, 0.18)] {
        let grow = spread * s;
        let mut shade = Color::BLACK;
        shade.set_alpha(alpha);
        fill(pixmap, rounded(x - grow / 2.0, y + 16.0 * s, panel_w + grow, panel_h + grow / 2.0, 34.0 * s), shade);
    }
    fill(pixmap, rounded(x, y, panel_w, panel_h, 28.0 * s), color(&palette.surface_2));

    let middle = y + panel_h / 2.0;
    let pad = 36.0 * s;
    text(pixmap, font, 26.0 * s, &level.label, x + pad, middle, color(&palette.muted), false);
    let value_w = text_width(font, 28.0 * s, &level.text);
    text(pixmap, font, 28.0 * s, &level.text, x + panel_w - pad - value_w, middle, color(&palette.text), false);

    let track_x = x + pad + 120.0 * s + 28.0 * s;
    let track_w = (x + panel_w - pad - 90.0 * s - 28.0 * s) - track_x;
    let track_h = 10.0 * s;
    let track_y = middle - track_h / 2.0;
    fill(pixmap, rounded(track_x, track_y, track_w, track_h, track_h / 2.0), color(&palette.surface_3));
    let filled = track_w * level.value.clamp(0.0, 1.0);
    if filled >= track_h {
        let ink = if level.muted { &palette.muted } else { &palette.accent };
        fill(pixmap, rounded(track_x, track_y, filled, track_h, track_h / 2.0), color(ink));
    }
}

/// The prompt: the screen dimmed, and in the middle the home screen's panel
/// with the question, the mode, the countdown and the keys that answer it.
fn paint_prompt(pixmap: &mut Pixmap, prompt: &Prompt, font: &FontRef) {
    let palette = &prompt.palette;
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let s = w / 1920.0;
    pixmap.fill(Color::from_rgba8(0, 0, 0, 0xa0));
    let (panel_w, panel_h) = (860.0 * s, 340.0 * s);
    let (x, y) = ((w - panel_w) / 2.0, (h - panel_h) / 2.0);
    for (spread, alpha) in [(36.0, 0.12), (20.0, 0.16), (8.0, 0.2)] {
        let grow = spread * s;
        let mut shade = Color::BLACK;
        shade.set_alpha(alpha);
        fill(pixmap, rounded(x - grow / 2.0, y + 24.0 * s, panel_w + grow, panel_h + grow / 2.0, 40.0 * s), shade);
    }
    fill(pixmap, rounded(x, y, panel_w, panel_h, 32.0 * s), color(&palette.surface));

    let pad = 56.0 * s;
    let left = x + pad;
    text(pixmap, font, 44.0 * s, &prompt.title, left, y + pad + 26.0 * s, color(&palette.text), false);
    text(pixmap, font, 30.0 * s, &prompt.detail, left, y + pad + 84.0 * s, color(&palette.text), false);
    text(pixmap, font, 26.0 * s, &prompt.countdown, left, y + pad + 130.0 * s, color(&palette.muted), false);

    // Key caps, each followed by what it does.
    let cap_h = 44.0 * s;
    let cap_y = y + panel_h - pad - cap_h;
    let mut pen = left;
    for (i, (key, does)) in prompt.keys.iter().enumerate() {
        let cap_w = (text_width(font, 20.0 * s, key) + 28.0 * s).max(64.0 * s);
        fill(pixmap, rounded(pen, cap_y, cap_w, cap_h, 12.0 * s), color(&palette.surface_2));
        text(pixmap, font, 20.0 * s, key, pen + cap_w / 2.0, cap_y + cap_h / 2.0, color(&palette.text), true);
        pen += cap_w + 14.0 * s;
        let ink = if i == 0 { &palette.text } else { &palette.muted };
        text(pixmap, font, 26.0 * s, does, pen, cap_y + cap_h / 2.0, color(ink), false);
        pen += text_width(font, 26.0 * s, does) + 40.0 * s;
    }
}

fn fill(pixmap: &mut Pixmap, path: Option<tiny_skia::Path>, color: Color) {
    let Some(path) = path else { return };
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// A rectangle with round corners, built from quarter curves.
fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    // How far the control points sit along the edge for a circular quarter.
    let k = r * 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn text_width(font: &FontRef, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut last = None;
    let mut width = 0.0;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = last {
            width += scaled.kern(prev, id);
        }
        width += scaled.h_advance(id);
        last = Some(id);
    }
    width
}

/// Draws `text` with its vertical middle at `y`; centered on `x` or starting there.
#[allow(clippy::too_many_arguments)]
fn text(pixmap: &mut Pixmap, font: &FontRef, size: f32, text: &str, x: f32, y: f32, ink: Color, center: bool) {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut pen = if center { x - text_width(font, size, text) / 2.0 } else { x };
    let baseline = y + (scaled.ascent() + scaled.descent()) / 2.0;
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let ink = ink.premultiply().to_color_u8();
    let data = pixmap.data_mut();
    let mut last = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(prev) = last {
            pen += scaled.kern(prev, id);
        }
        let glyph = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(pen, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let (px, py) = (bounds.min.x as i32 + gx as i32, bounds.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= width || py >= height {
                    return;
                }
                let i = (py * width + px) as usize * 4;
                // Source over, premultiplied.
                let a = coverage.clamp(0.0, 1.0);
                for (channel, value) in [ink.red(), ink.green(), ink.blue(), ink.alpha()].into_iter().enumerate() {
                    let below = f32::from(data[i + channel]);
                    data[i + channel] = (f32::from(value) * a + below * (1.0 - a)).round() as u8;
                }
            });
        }
        pen += scaled.h_advance(id);
        last = Some(id);
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        c: LayerSurfaceConfigure,
        _: u32,
    ) {
        let width = if c.new_size.0 == 0 { 1920 } else { c.new_size.0 };
        // The height follows the width, so it keeps its shape on any screen; a prompt fills it.
        let height = match self.mode {
            Mode::Keyboard => (PANEL_HEIGHT * width as f32 / 1920.0).round() as u32,
            Mode::Level => (LEVEL_HEIGHT * width as f32 / 1920.0).round() as u32,
            Mode::Prompt if c.new_size.1 == 0 => 1080,
            Mode::Prompt => c.new_size.1,
        };
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            if self.mode != Mode::Prompt {
                self.layer.set_size(0, height);
                self.layer.commit();
            }
            let _ = self.draw(qh);
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_layer!(App);
delegate_registry!(App);

/// The virtual keyboard protocol, generated from `protocols/` (MIT, see the file).
#[allow(dead_code, non_camel_case_types, non_upper_case_globals, unused_imports, clippy::all)]
mod virtual_keyboard {
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/virtual-keyboard-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("protocols/virtual-keyboard-unstable-v1.xml");
}

use virtual_keyboard::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

/// For `type`: text to type.
#[derive(Debug, Deserialize)]
struct Typing {
    text: String,
}

/// How many different characters one keymap holds (xkb keycodes 9..255).
const KEYS_PER_MAP: usize = 240;

/// The xkb name of the key that types `c`.
fn keysym(c: char) -> String {
    match c {
        '\n' => "Return".into(),
        '\t' => "Tab".into(),
        ' ' => "space".into(),
        // xkb reads U+hex as that character, Latin-1 included.
        _ => format!("U{:04X}", u32::from(c)),
    }
}

/// A keymap with one key per distinct character of `text`, and each character's
/// evdev key code (xkb's minus 8, as the protocol wants).
fn keymap_for(text: &str) -> (String, std::collections::HashMap<char, u32>) {
    let mut codes = std::collections::HashMap::new();
    let mut keys = String::new();
    let mut symbols = String::new();
    for c in text.chars() {
        if codes.contains_key(&c) || c.is_control() && c != '\n' && c != '\t' {
            continue;
        }
        let xkb = 9 + codes.len() as u32;
        keys.push_str(&format!("    <K{xkb}> = {xkb};\n"));
        symbols.push_str(&format!("    key <K{xkb}> {{ [ {} ] }};\n", keysym(c)));
        codes.insert(c, xkb - 8);
    }
    let keymap = format!(
        "xkb_keymap {{\n  xkb_keycodes \"rustv\" {{\n    minimum = 8;\n    maximum = 255;\n{keys}  }};\n  \
         xkb_types \"rustv\" {{ include \"complete\" }};\n  xkb_compat \"rustv\" {{ include \"complete\" }};\n  \
         xkb_symbols \"rustv\" {{\n{symbols}  }};\n}};\n"
    );
    (keymap, codes)
}

/// Types each line's text into the focused app, then answers `done`.
fn typing() -> Result<()> {
    use smithay_client_toolkit::reexports::client::{
        globals::GlobalListContents,
        protocol::{wl_registry, wl_seat},
    };
    struct Typer;
    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Typer {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<wl_seat::WlSeat, ()> for Typer {
        fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }
    impl Dispatch<ZwpVirtualKeyboardManagerV1, ()> for Typer {
        fn event(
            _: &mut Self,
            _: &ZwpVirtualKeyboardManagerV1,
            _: <ZwpVirtualKeyboardManagerV1 as smithay_client_toolkit::reexports::client::Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<ZwpVirtualKeyboardV1, ()> for Typer {
        fn event(
            _: &mut Self,
            _: &ZwpVirtualKeyboardV1,
            _: <ZwpVirtualKeyboardV1 as smithay_client_toolkit::reexports::client::Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<Typer>(&conn)?;
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).context("no seat")?;
    let manager: ZwpVirtualKeyboardManagerV1 =
        globals.bind(&qh, 1..=1, ()).context("the compositor offers no virtual keyboard (zwp_virtual_keyboard_v1)")?;
    let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
    let mut typer = Typer;
    let started = std::time::Instant::now();
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if let Ok(typing) = serde_json::from_str::<Typing>(&line) {
            let chars: Vec<char> = typing.text.chars().collect();
            // A keymap per stretch of text with at most KEYS_PER_MAP different characters.
            let mut rest = chars.as_slice();
            while !rest.is_empty() {
                let mut distinct = std::collections::HashSet::new();
                let take = rest
                    .iter()
                    .take_while(|c| distinct.contains(*c) || distinct.len() < KEYS_PER_MAP && distinct.insert(**c))
                    .count();
                let take = take.max(1).min(rest.len());
                let (part, more) = rest.split_at(take);
                let text: String = part.iter().collect();
                let (keymap, codes) = keymap_for(&text);
                let path = runtime.join(format!("rustv-keymap-{}", std::process::id()));
                let mut file =
                    std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path)?;
                std::fs::remove_file(&path)?;
                file.write_all(keymap.as_bytes())?;
                file.write_all(&[0])?;
                file.rewind()?;
                // 1: XKB_V1.
                keyboard.keymap(1, file.as_fd(), keymap.len() as u32 + 1);
                queue.roundtrip(&mut typer)?;
                for c in part {
                    let Some(code) = codes.get(c) else { continue };
                    let time = started.elapsed().as_millis() as u32;
                    keyboard.key(time, *code, 1);
                    keyboard.key(time + 1, *code, 0);
                    conn.flush()?;
                    std::thread::sleep(std::time::Duration::from_millis(6));
                }
                queue.roundtrip(&mut typer)?;
                rest = more;
            }
        }
        writeln!(out, "done")?;
        out.flush()?;
    }
    Ok(())
}

/// Holds the gamma tables of every output for as long as it runs.
struct Gamma {
    registry: RegistryState,
    outputs: OutputState,
    /// Each output's gamma control and its table size, once sway sent it.
    controls: Vec<(ZwlrGammaControlV1, Option<u32>)>,
}

/// One ramp for each of red, green and blue: a straight line from black to
/// `brightness` of full, the way a dimmer would.
fn ramps(size: u32, brightness: f32, warmth: f32) -> Vec<u16> {
    let ramp = |gain: f32| -> Vec<u16> {
        let top = f32::from(u16::MAX) * (brightness * gain).clamp(0.0, 1.0);
        (0..size).map(|i| (top * i as f32 / (size.max(2) - 1) as f32).round() as u16).collect()
    };
    // At full warmth, white is that of a 3400 K lamp (the blackbody's red, green
    // and blue relative to daylight's 6500 K), as night light tools use.
    let warmth = warmth.clamp(0.0, 1.0);
    let (green, blue) = (1.0 - 0.31 * warmth, 1.0 - 0.6 * warmth);
    [ramp(1.0), ramp(green), ramp(blue)].concat()
}

fn gamma() -> Result<()> {
    let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
    let (globals, mut queue) = registry_queue_init(&conn)?;
    let qh = queue.handle();
    let manager: ZwlrGammaControlManagerV1 =
        globals.bind(&qh, 1..=1, ()).context("the compositor does not offer gamma control (wlr-gamma-control)")?;
    let mut app = Gamma {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        controls: Vec::new(),
    };
    // The outputs, then each one's table size.
    queue.roundtrip(&mut app)?;
    let outputs: Vec<wl_output::WlOutput> = app.outputs.outputs().collect();
    for (index, output) in outputs.iter().enumerate() {
        app.controls.push((manager.get_gamma_control(output, &qh, index), None));
    }
    queue.roundtrip(&mut app)?;

    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    for line in std::io::stdin().lock().lines() {
        let Ok(dim) = serde_json::from_str::<Dim>(&line?) else { continue };
        for (control, size) in &app.controls {
            let Some(size) = size else { continue };
            // sway reads the table from a file descriptor; an unlinked file is enough.
            let path = runtime.join(format!("rustv-gamma-{}", std::process::id()));
            let mut file =
                std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path)?;
            std::fs::remove_file(&path)?;
            let table: Vec<u8> =
                ramps(*size, dim.brightness, dim.warmth).iter().flat_map(|v| v.to_ne_bytes()).collect();
            file.write_all(&table)?;
            file.rewind()?;
            control.set_gamma(file.as_fd());
        }
        queue.roundtrip(&mut app)?;
    }
    Ok(())
}

impl Dispatch<ZwlrGammaControlManagerV1, ()> for Gamma {
    fn event(
        _: &mut Self,
        _: &ZwlrGammaControlManagerV1,
        _: <ZwlrGammaControlManagerV1 as smithay_client_toolkit::reexports::client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrGammaControlV1, usize> for Gamma {
    fn event(
        state: &mut Self,
        _: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => state.controls[*index].1 = Some(size),
            // Another program holds this output's gamma (a night light, say).
            zwlr_gamma_control_v1::Event::Failed => {
                eprintln!("rustv-osd: gamma control of an output failed (is another program dimming it?)");
                state.controls[*index].1 = None;
            }
            _ => {}
        }
    }
}

impl OutputHandler for Gamma {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for Gamma {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_output!(Gamma);
delegate_registry!(Gamma);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_a_board() {
        let board: Board = serde_json::from_str(
            r##"{"rows":[[{"label":"q","focused":true},{"label":"w"}],[{"label":"space","width":5}]],
                 "preview":"hello","hint":"Back  Close",
                 "palette":{"background":"#0e1013","surface":"#1d1f22","surface_2":"#242629","surface_3":"#2e3032",
                            "accent":"#c9ced4","on_accent":"#0b0d0f","text":"#f2f3f3","muted":"#9d9e9f"}}"##,
        )
        .unwrap();
        let mut pixmap = Pixmap::new(1920, PANEL_HEIGHT as u32).unwrap();
        paint(&mut pixmap, &board, &FontRef::try_from_slice(FONT).unwrap());
        // The panel is opaque graphite, and the focused key is the lit plate somewhere.
        assert_eq!(pixmap.pixel(960, PANEL_HEIGHT as u32 - 2).unwrap().alpha(), 255);
        let plate = color("#c9ced4").to_color_u8();
        assert!(pixmap.pixels().iter().any(|p| p.red() == plate.red() && p.green() == plate.green()));
    }

    #[test]
    fn paints_a_level_over_a_clear_strip() {
        let level: Level = serde_json::from_str(
            r##"{"label":"Volume","value":0.5,"text":"50%",
                 "palette":{"background":"#0e1013","surface":"#1d1f22","surface_2":"#242629","surface_3":"#2e3032",
                            "accent":"#c9ced4","on_accent":"#0b0d0f","text":"#f2f3f3","muted":"#9d9e9f"}}"##,
        )
        .unwrap();
        let mut pixmap = Pixmap::new(1920, LEVEL_HEIGHT as u32).unwrap();
        paint_level(&mut pixmap, &level, &FontRef::try_from_slice(FONT).unwrap());
        // The app shows through beside the panel; the panel is opaque.
        assert_eq!(pixmap.pixel(20, 40).unwrap().alpha(), 0);
        assert_eq!(pixmap.pixel(960, 20).unwrap().alpha(), 255);
    }

    #[test]
    fn keymaps_hold_exactly_the_characters_to_type() {
        let (keymap, codes) = keymap_for("ağa\n");
        assert_eq!(codes.len(), 3);
        assert_eq!((codes[&'a'], codes[&'ğ'], codes[&'\n']), (1, 2, 3));
        assert!(keymap.contains("key <K9> { [ U0061 ] };"), "{keymap}");
        assert!(keymap.contains("key <K10> { [ U011F ] };"), "{keymap}");
        assert!(keymap.contains("key <K11> { [ Return ] };"), "{keymap}");
        assert!(keymap.contains("<K11> = 11;"));
        // Emoji are characters like any other.
        assert!(keymap_for("🙂").0.contains("U1F642"));
    }

    #[test]
    fn gamma_ramps_dim_in_a_straight_line() {
        let full = ramps(4, 1.0, 0.0);
        assert_eq!(full.len(), 12);
        assert_eq!((full[0], full[3]), (0, u16::MAX));
        let half = ramps(4, 0.5, 0.0);
        assert_eq!(half[3], 32768);
        // Red, green and blue alike.
        assert_eq!(half[..4], half[4..8]);
    }

    #[test]
    fn warmth_turns_down_green_and_more_blue() {
        let warm = ramps(4, 1.0, 1.0);
        let (red, green, blue) = (warm[3], warm[7], warm[11]);
        assert_eq!(red, u16::MAX);
        assert!(blue < green && green < red, "{red} {green} {blue}");
        assert_eq!(ramps(4, 1.0, 0.5)[11], (f32::from(u16::MAX) * 0.7).round() as u16);
    }
}
