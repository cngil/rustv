//! `rustv-keyboard`, the on-screen keyboard for the TV remote and gamepads.
//!
//! Only a renderer. tvd starts it when the keyboard opens and writes the whole
//! keyboard as one JSON line on stdin after every change: the keys, which one
//! has the focus, what was typed so far and the theme colors. tvd moves the
//! focus and types the letters into the app; this process never takes input
//! or focus. It draws on sway's overlay layer, so it shows over full screen
//! apps, and quits when tvd closes its stdin.

use std::io::BufRead;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use anyhow::{Context, Result};
use serde::Deserialize;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::client::{
        globals::registry_queue_init,
        protocol::{wl_output, wl_shm, wl_surface},
        Connection, QueueHandle,
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
    let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
    let (globals, mut queue) = registry_queue_init(&conn)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).context("no wl_compositor")?;
    let layer_shell = LayerShell::bind(&globals, &qh).context("the compositor has no layer shell")?;
    let shm = Shm::bind(&globals, &qh).context("no wl_shm")?;

    // Along the bottom, over everything, and never focused: the app keeps the keyboard.
    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("rustv-keyboard"), None);
    layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    layer.set_size(0, 1);
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
        board: Board::default(),
    };
    // Wait for the first size from sway.
    while app.width == 0 && !app.closed {
        queue.blocking_dispatch(&mut app)?;
    }

    for line in std::io::stdin().lock().lines() {
        let Ok(board) = serde_json::from_str::<Board>(&line?) else { continue };
        app.board = board;
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
    board: Board,
}

impl App {
    fn draw(&mut self, _qh: &QueueHandle<Self>) -> Result<()> {
        let (width, height) = (self.width, self.height);
        if width == 0 || height == 0 || self.board.rows.is_empty() {
            return Ok(());
        }
        let mut pixmap = Pixmap::new(width, height).context("pixmap")?;
        paint(&mut pixmap, &self.board, &self.font);

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
    let unit = (width - gap * (11.0 - 1.0)) / units.max(11.0);
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
        // The height follows the width, so the keys keep their shape on any screen.
        let height = (PANEL_HEIGHT * width as f32 / 1920.0).round() as u32;
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.layer.set_size(0, height);
            self.layer.commit();
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
}
