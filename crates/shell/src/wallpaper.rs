//! Your own pictures, behind the home screen and in the `images` screensaver.
//!
//! tvd only says how many there are; each picture is fetched by number when it
//! is needed, turned upright, scaled to fill the screen and then dropped again,
//! so a folder of 24-megapixel photos costs no more memory than one screen.

use std::cell::{Cell, RefCell};
use std::hash::{BuildHasher, Hasher};
use std::io::{Cursor, Read};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{bail, Result};
use image::{imageops::FilterType, DynamicImage, ImageDecoder, ImageReader};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode};

use crate::net::WallpaperConfig;
use crate::settings::Settings;
use crate::Shell;

/// Larger files are refused rather than read into memory.
const MAX_BYTES: u64 = 64 << 20;

static SETTINGS: OnceLock<Settings> = OnceLock::new();

#[derive(Clone, Copy)]
enum Target {
    Home,
    Slide,
}

thread_local! {
    static CONFIG: RefCell<WallpaperConfig> = RefCell::new(WallpaperConfig::default());
    static HOME_TIMER: Timer = Timer::default();
    /// The last picture shown, per target, so the next one differs.
    static LAST: Cell<[Option<usize>; 2]> = const { Cell::new([None; 2]) };
    /// A picture is on its way, per target.
    static LOADING: Cell<[bool; 2]> = const { Cell::new([false; 2]) };
}

pub fn init(settings: Settings) {
    let _ = SETTINGS.set(settings);
}

pub fn count() -> usize {
    CONFIG.with(|cfg| cfg.borrow().count)
}

pub fn slide_seconds() -> f32 {
    CONFIG.with(|cfg| cfg.borrow().slide_seconds)
}

/// New `[wallpaper]` settings from tvd: start, change or stop the home picture.
pub fn configure(ui: &Shell, cfg: WallpaperConfig) {
    if CONFIG.with(|current| *current.borrow() == cfg) {
        return;
    }
    ui.set_slide_seconds(cfg.slide_seconds);
    let home = cfg.home && cfg.count > 0;
    let every = Duration::from_secs_f32(cfg.change_minutes * 60.0);
    CONFIG.with(|current| *current.borrow_mut() = cfg);
    LAST.with(|last| last.set([None; 2]));
    if !home {
        HOME_TIMER.with(Timer::stop);
        ui.set_wallpaper_on(false);
        ui.set_wallpaper_a(Image::default());
        ui.set_wallpaper_b(Image::default());
        return;
    }
    load(ui, Target::Home);
    let weak = ui.as_weak();
    HOME_TIMER.with(|timer| {
        timer.start(TimerMode::Repeated, every, move || {
            if let Some(ui) = weak.upgrade() {
                load(&ui, Target::Home);
            }
        })
    });
}

/// The `images` screensaver starts: the first picture fades in from black.
pub fn start_slides(ui: &Shell) {
    ui.set_photo(Image::default());
    ui.set_photo_before(Image::default());
    ui.set_photo_since(0.0);
    load(ui, Target::Slide);
}

/// Called on every screensaver frame; fetches the next picture once this one has had its time.
pub fn next_slide_if_due(ui: &Shell) {
    if ui.get_screensaver_time() - ui.get_photo_since() >= slide_seconds() {
        load(ui, Target::Slide);
    }
}

fn load(ui: &Shell, target: Target) {
    let slot = target as usize;
    let (count, shuffle) = CONFIG.with(|cfg| (cfg.borrow().count, cfg.borrow().shuffle));
    if count == 0 || LOADING.with(|loading| loading.get()[slot]) {
        return;
    }
    let Some(settings) = SETTINGS.get().cloned() else { return };
    let n = next_index(count, shuffle, LAST.with(|last| last.get()[slot]), random());
    LAST.with(|last| {
        let mut all = last.get();
        all[slot] = Some(n);
        last.set(all);
    });
    set_loading(slot, true);

    let size = ui.window().size();
    let (width, height) = match (size.width, size.height) {
        (0, _) | (_, 0) => (1920, 1080),
        (w, h) => (w.min(3840), h.min(2160)),
    };
    let weak = ui.as_weak();
    let spawned = std::thread::Builder::new().name("wallpaper".into()).spawn(move || {
        let picture = fetch(&settings, n).and_then(|bytes| decode(&bytes, width, height));
        let _ = weak.upgrade_in_event_loop(move |ui| {
            set_loading(slot, false);
            match picture {
                Ok(pixels) => show(&ui, target, Image::from_rgba8(pixels)),
                Err(err) => eprintln!("rustv-shell: wallpaper {n}: {err:#}"),
            }
        });
    });
    if spawned.is_err() {
        set_loading(slot, false);
    }
}

fn set_loading(slot: usize, on: bool) {
    LOADING.with(|loading| {
        let mut all = loading.get();
        all[slot] = on;
        loading.set(all);
    });
}

fn show(ui: &Shell, target: Target, picture: Image) {
    match target {
        // Two layers crossfade: the new picture goes into the hidden one, which then comes to the front.
        Target::Home => {
            if ui.get_wallpaper_b_front() {
                ui.set_wallpaper_a(picture);
            } else {
                ui.set_wallpaper_b(picture);
            }
            ui.set_wallpaper_b_front(!ui.get_wallpaper_b_front());
            ui.set_wallpaper_on(true);
        }
        Target::Slide => {
            if !ui.get_screensaver_active() {
                return;
            }
            ui.set_photo_before(ui.get_photo());
            ui.set_photo(picture);
            ui.set_photo_since(ui.get_screensaver_time());
        }
    }
}

fn fetch(settings: &Settings, n: usize) -> Result<Vec<u8>> {
    let token = settings.token()?;
    let response = ureq::get(&format!("http://{}/api/v1/wallpapers/{n}", settings.addr))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(20))
        .call()?;
    let mut bytes = Vec::new();
    response.into_reader().take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("larger than {} MB", MAX_BYTES >> 20);
    }
    Ok(bytes)
}

/// Upright (phone photos store their rotation separately), scaled and cropped to fill the screen.
fn decode(bytes: &[u8], width: u32, height: u32) -> Result<SharedPixelBuffer<Rgba8Pixel>> {
    let mut decoder = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut picture = DynamicImage::from_decoder(decoder)?;
    picture.apply_orientation(orientation);
    let picture = picture.resize_to_fill(width, height, FilterType::Triangle).to_rgba8();
    let (w, h) = picture.dimensions();
    Ok(SharedPixelBuffer::clone_from_slice(picture.as_raw(), w, h))
}

/// The next picture: in the listed order, or at random but never the same one twice in a row.
fn next_index(count: usize, shuffle: bool, last: Option<usize>, random: u64) -> usize {
    match last {
        _ if count <= 1 => 0,
        None if !shuffle => 0,
        Some(last) if !shuffle => (last + 1) % count,
        Some(last) => (last + 1 + (random as usize) % (count - 1)) % count,
        None => random as usize % count,
    }
}

fn random() -> u64 {
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_next_picture() {
        assert_eq!(next_index(3, false, None, 7), 0);
        assert_eq!(next_index(3, false, Some(2), 7), 0);
        assert_eq!(next_index(1, true, Some(0), 7), 0);
        for random in 0..20 {
            let n = next_index(4, true, Some(2), random);
            assert!(n < 4 && n != 2);
        }
    }

    #[test]
    fn fills_the_screen_upright() {
        // A 4x2 PNG becomes exactly the screen size.
        let mut png = Vec::new();
        DynamicImage::new_rgba8(4, 2).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let pixels = decode(&png, 30, 20).unwrap();
        assert_eq!((pixels.width(), pixels.height()), (30, 20));
    }
}
