//! HDMI-CEC: the TV and rustv sleep and wake together, and the TV's own remote
//! works on rustv.
//!
//! tvd talks to the kernel CEC framework (`/dev/cecN`) directly, so nothing
//! else needs installing. That covers boards and GPUs that wire CEC to the HDMI
//! port (Raspberry Pi, some NUCs, DisplayPort adapters with CEC tunneling) and
//! USB adapters such as the Pulse-Eight once `inputattach --pulse8-cec` runs.
//! Most PC graphics cards carry no CEC at all; then there is no `/dev/cec*` and
//! this module stays idle.
//!
//! rustv shows up on the CEC bus as a playback device. The kernel answers the
//! basic questions (physical address, vendor, OSD name); tvd handles the rest:
//!
//! - rustv wakes: "Image View On" turns the TV on, "Active Source" switches it to rustv.
//! - rustv goes to sleep: "Standby" turns the TV off, but only if rustv is what it shows.
//! - The TV turns off: rustv goes to sleep. The TV switches to rustv: rustv wakes.
//! - Buttons on the TV remote arrive as "User Control Pressed" and become remote commands.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::audio::VolumeAction;
use crate::media;
use crate::protocol::Command;
use crate::state::AppState;

const CHECK_EVERY: Duration = Duration::from_secs(5);
/// Holding OK this long opens the on-screen keyboard.
const HOLD: Duration = Duration::from_millis(600);
/// The UI command code of OK.
const SELECT: u8 = 0x00;
/// How long one read waits before checking whether the adapter should close.
const RECEIVE_TIMEOUT_MS: u32 = 1000;

// ioctls from <linux/cec.h>.
const CEC_ADAP_G_PHYS_ADDR: u64 = 0x8002_6101;
const CEC_ADAP_G_LOG_ADDRS: u64 = 0x805c_6103;
const CEC_ADAP_S_LOG_ADDRS: u64 = 0xc05c_6104;
const CEC_TRANSMIT: u64 = 0xc038_6105;
const CEC_RECEIVE: u64 = 0xc038_6106;
const CEC_S_MODE: u64 = 0x4004_6109;

const MODE_INITIATOR: u32 = 0x01;
const MODE_FOLLOWER: u32 = 0x10;
const TX_STATUS_OK: u8 = 0x01;
const VERSION_1_4: u8 = 5;
const VENDOR_ID_NONE: u32 = 0xffff_ffff;
const LOG_ADDR_TYPE_PLAYBACK: u8 = 3;
const PRIM_DEVTYPE_PLAYBACK: u8 = 4;
const ALL_DEVTYPE_PLAYBACK: u8 = 0x10;
const LOG_ADDR_INVALID: u8 = 0xff;
const PHYS_ADDR_INVALID: u16 = 0xffff;

/// Logical addresses.
const TV: u8 = 0;
const BROADCAST: u8 = 0xf;

// Opcodes.
const FEATURE_ABORT: u8 = 0x00;
const IMAGE_VIEW_ON: u8 = 0x04;
const STANDBY: u8 = 0x36;
const USER_CONTROL_PRESSED: u8 = 0x44;
const USER_CONTROL_RELEASED: u8 = 0x45;
const ROUTING_CHANGE: u8 = 0x80;
const ROUTING_INFORMATION: u8 = 0x81;
const ACTIVE_SOURCE: u8 = 0x82;
const REQUEST_ACTIVE_SOURCE: u8 = 0x85;
const SET_STREAM_PATH: u8 = 0x86;
const MENU_REQUEST: u8 = 0x8d;
const MENU_STATUS: u8 = 0x8e;
const GIVE_DEVICE_POWER_STATUS: u8 = 0x8f;
const REPORT_POWER_STATUS: u8 = 0x90;
/// Answered by the kernel itself (or meant for it), so never refused here.
const KERNEL_HANDLES: &[u8] = &[0x46, 0x83, 0x84, 0x8c, 0x9f, 0xa5, 0xff];

const ABORT_UNRECOGNIZED: u8 = 0;
const POWER_ON: u8 = 0;
const POWER_STANDBY: u8 = 1;
const MENU_ACTIVATED: u8 = 0;

/// `struct cec_msg`.
#[repr(C)]
#[derive(Default)]
struct RawMsg {
    tx_ts: u64,
    rx_ts: u64,
    len: u32,
    timeout: u32,
    sequence: u32,
    flags: u32,
    msg: [u8; 16],
    reply: u8,
    rx_status: u8,
    tx_status: u8,
    tx_arb_lost_cnt: u8,
    tx_nack_cnt: u8,
    tx_low_drive_cnt: u8,
    tx_error_cnt: u8,
}

/// `struct cec_log_addrs`.
#[repr(C)]
#[derive(Default)]
struct RawLogAddrs {
    log_addr: [u8; 4],
    log_addr_mask: u16,
    cec_version: u8,
    num_log_addrs: u8,
    vendor_id: u32,
    flags: u32,
    osd_name: [u8; 15],
    primary_device_type: [u8; 4],
    log_addr_type: [u8; 4],
    all_device_types: [u8; 4],
    features: [[u8; 12]; 4],
}

// The ioctl numbers above encode these sizes.
const _: () = assert!(std::mem::size_of::<RawMsg>() == 56);
const _: () = assert!(std::mem::size_of::<RawLogAddrs>() == 92);

fn ioctl<T>(file: &File, request: u64, arg: &mut T) -> io::Result<()> {
    // SAFETY: each request is a CEC ioctl and `T` the #[repr(C)] struct of the size it encodes.
    let ret = unsafe { libc::ioctl(file.as_raw_fd(), request as _, arg as *mut T) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// An open CEC adapter, with rustv claimed as a playback device on it.
struct Adapter {
    file: File,
    path: PathBuf,
    name: String,
    /// Set to close the adapter: the reader stops within a second.
    stop: AtomicBool,
    /// Reading failed (the adapter was unplugged); the watcher opens it again.
    gone: AtomicBool,
}

impl Adapter {
    /// Blocks while the kernel claims a logical address, up to a second or so.
    fn open(path: &Path, name: &str) -> Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path).with_context(|| {
            format!("opening {} (is your user in the group that owns it, usually \"video\"?)", path.display())
        })?;
        let mut mode = MODE_INITIATOR | MODE_FOLLOWER;
        ioctl(&file, CEC_S_MODE, &mut mode).context("taking over the adapter")?;
        // A previous run (or cec-ctl) may have left addresses claimed; claiming again needs a clean slate.
        let mut current = RawLogAddrs::default();
        ioctl(&file, CEC_ADAP_G_LOG_ADDRS, &mut current).context("reading logical addresses")?;
        if current.num_log_addrs > 0 {
            ioctl(&file, CEC_ADAP_S_LOG_ADDRS, &mut RawLogAddrs::default()).context("clearing logical addresses")?;
        }
        let mut addrs =
            RawLogAddrs { cec_version: VERSION_1_4, num_log_addrs: 1, vendor_id: VENDOR_ID_NONE, ..Default::default() };
        addrs.osd_name[..name.len()].copy_from_slice(name.as_bytes());
        addrs.primary_device_type[0] = PRIM_DEVTYPE_PLAYBACK;
        addrs.log_addr_type[0] = LOG_ADDR_TYPE_PLAYBACK;
        addrs.all_device_types[0] = ALL_DEVTYPE_PLAYBACK;
        ioctl(&file, CEC_ADAP_S_LOG_ADDRS, &mut addrs).context("claiming a playback address")?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            name: name.to_string(),
            stop: AtomicBool::new(false),
            gone: AtomicBool::new(false),
        })
    }

    /// Where rustv sits in the HDMI tree, such as 1.0.0.0 for the TV's first input.
    /// Invalid while the TV is unplugged or has not sent its EDID.
    fn physical_address(&self) -> u16 {
        let mut addr = PHYS_ADDR_INVALID;
        let _ = ioctl(&self.file, CEC_ADAP_G_PHYS_ADDR, &mut addr);
        addr
    }

    fn logical_address(&self) -> Option<u8> {
        let mut addrs = RawLogAddrs::default();
        ioctl(&self.file, CEC_ADAP_G_LOG_ADDRS, &mut addrs).ok()?;
        (addrs.num_log_addrs > 0 && addrs.log_addr[0] != LOG_ADDR_INVALID).then_some(addrs.log_addr[0])
    }

    /// Sends `message` (opcode and operands) and waits until the bus took it.
    fn transmit(&self, to: u8, message: &[u8]) -> Result<()> {
        let from = self.logical_address().context("no logical address yet (is the TV connected?)")?;
        let mut raw = RawMsg { len: 1 + message.len() as u32, ..Default::default() };
        raw.msg[0] = from << 4 | to;
        raw.msg[1..=message.len()].copy_from_slice(message);
        ioctl(&self.file, CEC_TRANSMIT, &mut raw).context("sending")?;
        if raw.tx_status & TX_STATUS_OK == 0 {
            bail!("not acknowledged (tx status {:#04x})", raw.tx_status);
        }
        Ok(())
    }

    /// The next message for rustv, or `None` after a second without one.
    fn receive(&self) -> io::Result<Option<Vec<u8>>> {
        let mut raw = RawMsg { timeout: RECEIVE_TIMEOUT_MS, ..Default::default() };
        match ioctl(&self.file, CEC_RECEIVE, &mut raw) {
            Ok(()) => Ok(Some(raw.msg[..(raw.len as usize).min(16)].to_vec())),
            Err(err) if matches!(err.raw_os_error(), Some(libc::ETIMEDOUT | libc::EINTR)) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

impl Drop for Adapter {
    /// Leaves the bus, so the TV stops listing rustv as an input.
    fn drop(&mut self) {
        let _ = ioctl(&self.file, CEC_ADAP_S_LOG_ADDRS, &mut RawLogAddrs::default());
    }
}

/// CEC as seen by the rest of tvd.
#[derive(Default)]
pub struct Cec {
    adapter: Mutex<Option<Arc<Adapter>>>,
    /// The TV shows another input (a console, its own tuner). Sleep then leaves the TV alone.
    elsewhere: AtomicBool,
    /// The TV said it went to standby, so there is no need to tell it.
    tv_off: AtomicBool,
    /// OK is down and not decided yet: a tap (Enter) or a hold (the keyboard).
    ok_down: Mutex<Option<OkDown>>,
    ok_count: std::sync::atomic::AtomicU64,
}

struct OkDown {
    since: std::time::Instant,
    /// The TV repeated the press, as it does while a button is held.
    repeated: bool,
    id: u64,
}

impl Cec {
    fn adapter(&self) -> Option<Arc<Adapter>> {
        self.adapter.lock().unwrap().clone()
    }

    /// rustv woke up: turn the TV on and switch it to rustv.
    pub async fn tv_on(&self) {
        let Some(adapter) = self.adapter() else { return };
        self.elsewhere.store(false, Ordering::Relaxed);
        self.tv_off.store(false, Ordering::Relaxed);
        let sent = tokio::task::spawn_blocking(move || {
            adapter.transmit(TV, &[IMAGE_VIEW_ON])?;
            let [high, low] = adapter.physical_address().to_be_bytes();
            adapter.transmit(BROADCAST, &[ACTIVE_SOURCE, high, low])
        })
        .await;
        match sent {
            Ok(Ok(())) => info!("CEC: TV on, switched to rustv"),
            Ok(Err(err)) => warn!("CEC: turning the TV on: {err:#}"),
            Err(err) => warn!("CEC: {err}"),
        }
    }

    /// rustv went to sleep: turn the TV off too, unless it is showing something else.
    pub async fn tv_standby(&self) {
        let Some(adapter) = self.adapter() else { return };
        if self.elsewhere.load(Ordering::Relaxed) {
            info!("CEC: the TV shows another input, leaving it on");
            return;
        }
        if self.tv_off.swap(true, Ordering::Relaxed) {
            return;
        }
        match tokio::task::spawn_blocking(move || adapter.transmit(TV, &[STANDBY])).await {
            Ok(Ok(())) => info!("CEC: TV to standby"),
            Ok(Err(err)) => warn!("CEC: turning the TV off: {err:#}"),
            Err(err) => warn!("CEC: {err}"),
        }
    }
}

/// Opens the adapter while `cec.enabled` is on, and opens it again after it
/// was unplugged or the settings changed, for as long as tvd runs.
pub fn watch(state: Weak<AppState>) {
    tokio::spawn(async move {
        let mut last_problem = String::new();
        loop {
            let Some(app) = state.upgrade() else { return };
            let cfg = app.config().cec.clone();
            let open = app.cec.adapter();
            if let Some(adapter) = open {
                let device_changed = cfg.device.as_ref().is_some_and(|path| *path != adapter.path);
                if adapter.gone.load(Ordering::Relaxed) || !cfg.enabled || device_changed || cfg.name != adapter.name {
                    info!(path = %adapter.path.display(), "CEC: closing the adapter");
                    adapter.stop.store(true, Ordering::Relaxed);
                    *app.cec.adapter.lock().unwrap() = None;
                }
            } else if cfg.enabled {
                match connect(&app, &cfg).await {
                    Ok(()) => last_problem.clear(),
                    // Said once, not every five seconds.
                    Err(err) => {
                        let problem = format!("{err:#}");
                        if problem != last_problem {
                            warn!("CEC: {problem}");
                            last_problem = problem;
                        }
                    }
                }
            }
            drop(app);
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

async fn connect(app: &Arc<AppState>, cfg: &crate::config::Cec) -> Result<()> {
    let path = match &cfg.device {
        Some(path) => path.clone(),
        None => find_adapter().context("no HDMI-CEC adapter (/dev/cec*); this graphics card or cable has no CEC")?,
    };
    let name = cfg.name.clone();
    let opening = path.clone();
    let adapter = Arc::new(tokio::task::spawn_blocking(move || Adapter::open(&opening, &name)).await??);
    info!(path = %path.display(), address = %format_address(adapter.physical_address()), "CEC: connected");

    let (frames, mut incoming) = mpsc::unbounded_channel();
    let reader = Arc::clone(&adapter);
    std::thread::Builder::new().name("cec".into()).spawn(move || read(&reader, &frames))?;
    let (weak, handling) = (Arc::downgrade(app), Arc::clone(&adapter));
    tokio::spawn(async move {
        while let Some(frame) = incoming.recv().await {
            let Some(app) = weak.upgrade() else { return };
            handle(&app, &handling, &frame).await;
        }
    });

    app.cec.elsewhere.store(false, Ordering::Relaxed);
    app.cec.tv_off.store(false, Ordering::Relaxed);
    *app.cec.adapter.lock().unwrap() = Some(Arc::clone(&adapter));
    // Whatever the TV shows now (a console, say) answers, which tells sleep to leave the TV alone.
    let _ = tokio::task::spawn_blocking(move || adapter.transmit(BROADCAST, &[REQUEST_ACTIVE_SOURCE])).await;
    Ok(())
}

fn find_adapter() -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir("/dev")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("cec")))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Runs on its own thread: the kernel only offers a blocking read.
fn read(adapter: &Adapter, frames: &mpsc::UnboundedSender<Vec<u8>>) {
    while !adapter.stop.load(Ordering::Relaxed) {
        match adapter.receive() {
            Ok(Some(frame)) => {
                if frames.send(frame).is_err() {
                    return;
                }
            }
            Ok(None) => {}
            Err(err) => {
                warn!(path = %adapter.path.display(), "CEC: reading: {err}");
                adapter.gone.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
}

async fn handle(app: &Arc<AppState>, adapter: &Arc<Adapter>, frame: &[u8]) {
    debug!(frame = %hex(frame), "CEC: received");
    let me = Me {
        address: adapter.physical_address(),
        awake: !app.is_asleep(),
        shown: !app.cec.elsewhere.load(Ordering::Relaxed),
    };
    let (reply, event) = react(frame, &me);
    if let Some((to, message)) = reply {
        let adapter = Arc::clone(adapter);
        if let Ok(Err(err)) = tokio::task::spawn_blocking(move || adapter.transmit(to, &message)).await {
            debug!("CEC: answering: {err:#}");
        }
    }
    let cfg = app.config().cec.clone();
    match event {
        Some(Event::Standby) => {
            app.cec.tv_off.store(true, Ordering::Relaxed);
            if cfg.follow_tv && !app.is_asleep() {
                info!("CEC: the TV turned off, going to sleep");
                app.go_to_sleep().await;
            }
        }
        Some(Event::Shown) => {
            app.cec.elsewhere.store(false, Ordering::Relaxed);
            app.cec.tv_off.store(false, Ordering::Relaxed);
            if cfg.follow_tv && app.is_asleep() {
                info!("CEC: the TV switched to rustv, waking up");
                app.external_activity().await;
            }
        }
        Some(Event::Elsewhere) => {
            app.cec.elsewhere.store(true, Ordering::Relaxed);
            app.cec.tv_off.store(false, Ordering::Relaxed);
        }
        Some(Event::Button(code)) if cfg.remote_keys => press(app, code).await,
        Some(Event::Released) if cfg.remote_keys => released(app).await,
        Some(Event::Button(_) | Event::Released) | None => {}
    }
}

/// Whether OK has to wait to see if it is held: only where holding it opens the keyboard.
fn ok_can_hold(app: &AppState) -> bool {
    let cfg = app.config().keyboard.clone();
    cfg.enabled
        && cfg.hold_ok
        && !app.keyboard.is_open()
        && !app.is_asleep()
        && !app.is_locked()
        && !crate::shell_controls::POINTER.load(Ordering::Relaxed)
}

/// OK went down, or the TV repeated it because it is still down.
async fn ok_pressed(app: &Arc<AppState>) {
    // Decided under the lock, acted on after it.
    let started = {
        let mut down = app.cec.ok_down.lock().unwrap();
        match down.as_mut() {
            Some(held) => {
                held.repeated = true;
                if held.since.elapsed() >= HOLD {
                    *down = None;
                    Err(true)
                } else {
                    Err(false)
                }
            }
            None => {
                let id = app.cec.ok_count.fetch_add(1, Ordering::Relaxed) + 1;
                *down = Some(OkDown { since: std::time::Instant::now(), repeated: false, id });
                Ok(id)
            }
        }
    };
    let id = match started {
        Ok(id) => id,
        // Held long enough: the keyboard.
        Err(true) => return app.set_keyboard(true).await,
        Err(false) => return,
    };
    // Neither a release nor a repeat by then: a TV that sends neither, so treat it as a tap.
    let app = Arc::clone(app);
    tokio::spawn(async move {
        tokio::time::sleep(HOLD).await;
        let decided = {
            let mut down = app.cec.ok_down.lock().unwrap();
            if down.as_ref().is_some_and(|held| held.id == id) {
                down.take()
            } else {
                None
            }
        };
        match decided {
            Some(held) if held.repeated => app.set_keyboard(true).await,
            Some(_) => tap_ok(&app).await,
            None => {}
        }
    });
}

/// A button was let go: an OK that is still undecided was a tap.
async fn released(app: &Arc<AppState>) {
    let undecided = app.cec.ok_down.lock().unwrap().take();
    if undecided.is_some() {
        tap_ok(app).await;
    }
}

async fn tap_ok(app: &Arc<AppState>) {
    let enter = Command::Key {
        code: "Enter".into(),
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
        action: Default::default(),
    };
    if let Err(err) = app.execute(enter).await {
        debug!("CEC: OK: {err:#}");
    }
}

/// A button on the TV remote.
async fn press(app: &Arc<AppState>, code: u8) {
    if code == SELECT && (app.cec.ok_down.lock().unwrap().is_some() || ok_can_hold(app)) {
        return ok_pressed(app).await;
    }
    let Some(button) = button(code) else {
        debug!(code, "CEC: button without a meaning in rustv");
        return;
    };
    debug!(?button, "CEC: button");
    let command = match button {
        Button::Key(code) => Command::Key {
            code: code.into(),
            shift: false,
            ctrl: false,
            alt: false,
            meta: false,
            action: Default::default(),
        },
        Button::Media(action, seconds) => Command::Media { action, seconds },
        Button::Volume(action) => Command::Volume { action, level: None },
        Button::Home => Command::Stop,
        Button::Power | Button::PowerOn if app.is_asleep() => return app.external_activity().await,
        Button::Power | Button::PowerOff if !app.is_asleep() => Command::Sleep,
        Button::Power | Button::PowerOn | Button::PowerOff => return,
    };
    if let Err(err) = app.execute(command).await {
        debug!("CEC: button: {err:#}");
    }
}

/// What rustv knows about itself when a message arrives.
struct Me {
    address: u16,
    awake: bool,
    /// The TV shows rustv, as far as we know.
    shown: bool,
}

#[derive(Debug, PartialEq)]
enum Event {
    /// The TV (or the whole system) went to standby.
    Standby,
    /// The TV switched to rustv.
    Shown,
    /// The TV switched to something else.
    Elsewhere,
    /// A button on the TV remote, as a CEC UI command code.
    Button(u8),
    /// The TV remote's button was let go.
    Released,
}

/// The answer to `frame` (to whom, and what), and what it means for rustv.
fn react(frame: &[u8], me: &Me) -> (Option<(u8, Vec<u8>)>, Option<Event>) {
    // A lone header is a poll, answered by the hardware.
    let [header, opcode, args @ ..] = frame else { return (None, None) };
    let from = header >> 4;
    let broadcast = header & 0xf == BROADCAST;
    let address = |at: usize| args.get(at..at + 2).map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
    let [high, low] = me.address.to_be_bytes();
    let active_source = || (BROADCAST, vec![ACTIVE_SOURCE, high, low]);
    let routed = |to: Option<u16>| match to.map(|to| route(to, me.address)) {
        Some(Route::Us) => (Some(active_source()), Some(Event::Shown)),
        Some(Route::Other) => (None, Some(Event::Elsewhere)),
        Some(Route::Above) | None => (None, None),
    };
    match *opcode {
        STANDBY => (None, Some(Event::Standby)),
        ACTIVE_SOURCE => match address(0).map(|to| route(to, me.address)) {
            Some(Route::Other) => (None, Some(Event::Elsewhere)),
            _ => (None, None),
        },
        SET_STREAM_PATH | ROUTING_INFORMATION => routed(address(0)),
        ROUTING_CHANGE => routed(address(2)),
        REQUEST_ACTIVE_SOURCE if me.awake && me.shown && me.address != PHYS_ADDR_INVALID => {
            (Some(active_source()), None)
        }
        USER_CONTROL_PRESSED => (None, args.first().map(|&code| Event::Button(code))),
        USER_CONTROL_RELEASED => (None, Some(Event::Released)),
        GIVE_DEVICE_POWER_STATUS if !broadcast => {
            (Some((from, vec![REPORT_POWER_STATUS, if me.awake { POWER_ON } else { POWER_STANDBY }])), None)
        }
        // Some TVs only pass their remote's buttons on once the menu is "active".
        MENU_REQUEST if !broadcast => (Some((from, vec![MENU_STATUS, MENU_ACTIVATED])), None),
        // Directed messages rustv does not know get a polite "no", as CEC asks for.
        op if !broadcast && from != BROADCAST && op != FEATURE_ABORT && !KERNEL_HANDLES.contains(&op) => {
            (Some((from, vec![FEATURE_ABORT, op, ABORT_UNRECOGNIZED])), None)
        }
        _ => (None, None),
    }
}

#[derive(Debug, PartialEq)]
enum Route {
    Us,
    /// A switch or receiver rustv is plugged into; it passes the route on.
    Above,
    Other,
}

/// Where a route to physical address `to` leads, seen from rustv at `me`.
/// 0.0.0.0 is the TV itself (its tuner or apps).
fn route(to: u16, me: u16) -> Route {
    if me == PHYS_ADDR_INVALID || to == PHYS_ADDR_INVALID || to == 0 {
        return Route::Other;
    }
    if to == me {
        return Route::Us;
    }
    // 1.0.0.0 is above 1.2.0.0: every digit it sets matches, the rest are 0.
    let digits = (to.trailing_zeros() / 4) * 4;
    if me >> digits == to >> digits {
        Route::Above
    } else {
        Route::Other
    }
}

#[derive(Debug, Clone, Copy)]
enum Button {
    Key(&'static str),
    Media(media::Action, i64),
    Volume(VolumeAction),
    Home,
    Power,
    PowerOff,
    PowerOn,
}

const DIGITS: [&str; 10] =
    ["Digit0", "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8", "Digit9"];

/// CEC UI command codes, as the TV remote sends them.
fn button(code: u8) -> Option<Button> {
    Some(match code {
        0x00 | 0x2b => Button::Key("Enter"),
        0x01 => Button::Key("ArrowUp"),
        0x02 => Button::Key("ArrowDown"),
        0x03 => Button::Key("ArrowLeft"),
        0x04 => Button::Key("ArrowRight"),
        // Root menu, often the Home button.
        0x09 => Button::Home,
        // Setup menu: the shell opens its settings on the Menu key.
        0x0a => Button::Key("ContextMenu"),
        // Back, or Exit on older remotes.
        0x0d => Button::Key("Escape"),
        0x20..=0x29 => Button::Key(DIGITS[usize::from(code - 0x20)]),
        0x30 => Button::Key("PageUp"),
        0x31 => Button::Key("PageDown"),
        0x40 | 0x6b => Button::Power,
        0x6c => Button::PowerOff,
        0x6d => Button::PowerOn,
        0x41 => Button::Volume(VolumeAction::Up),
        0x42 => Button::Volume(VolumeAction::Down),
        0x43 | 0x65 => Button::Volume(VolumeAction::Mute),
        0x44 | 0x46 | 0x60 | 0x61 => Button::Media(media::Action::PlayPause, 0),
        0x48 => Button::Media(media::Action::Seek, -10),
        0x49 => Button::Media(media::Action::Seek, 10),
        0x4b => Button::Media(media::Action::Next, 0),
        0x4c => Button::Media(media::Action::Previous, 0),
        _ => return None,
    })
}

fn format_address(address: u16) -> String {
    if address == PHYS_ADDR_INVALID {
        return "unknown".into();
    }
    let digit = |shift: u16| (address >> shift) & 0xf;
    format!("{}.{}.{}.{}", digit(12), digit(8), digit(4), digit(0))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: u16 = 0x1000;

    fn me(awake: bool, shown: bool) -> Me {
        Me { address: ME, awake, shown }
    }

    #[test]
    fn follows_the_tv() {
        // TV (0) broadcasts Standby.
        assert_eq!(react(&[0x0f, STANDBY], &me(true, true)), (None, Some(Event::Standby)));
        // The TV switches to rustv: answer with Active Source.
        let ours = vec![ACTIVE_SOURCE, 0x10, 0x00];
        assert_eq!(
            react(&[0x0f, SET_STREAM_PATH, 0x10, 0x00], &me(false, true)),
            (Some((0xf, ours.clone())), Some(Event::Shown))
        );
        assert_eq!(
            react(&[0x0f, ROUTING_CHANGE, 0x20, 0x00, 0x10, 0x00], &me(true, false)),
            (Some((0xf, ours)), Some(Event::Shown))
        );
        // A console at 2.0.0.0 takes over, or the TV goes to its own tuner.
        assert_eq!(react(&[0x8f, ACTIVE_SOURCE, 0x20, 0x00], &me(true, true)), (None, Some(Event::Elsewhere)));
        assert_eq!(react(&[0x0f, ACTIVE_SOURCE, 0x00, 0x00], &me(true, true)), (None, Some(Event::Elsewhere)));
        assert_eq!(
            react(&[0x0f, ROUTING_CHANGE, 0x10, 0x00, 0x30, 0x00], &me(true, true)),
            (None, Some(Event::Elsewhere))
        );
    }

    #[test]
    fn answers_questions() {
        assert_eq!(
            react(&[0x04, GIVE_DEVICE_POWER_STATUS], &me(false, true)).0,
            Some((0, vec![REPORT_POWER_STATUS, POWER_STANDBY]))
        );
        assert_eq!(react(&[0x04, MENU_REQUEST, 0x02], &me(true, true)).0, Some((0, vec![MENU_STATUS, MENU_ACTIVATED])));
        // Only an awake rustv on screen claims to be the active source.
        assert!(react(&[0x0f, REQUEST_ACTIVE_SOURCE], &me(true, true)).0.is_some());
        assert!(react(&[0x0f, REQUEST_ACTIVE_SOURCE], &me(false, true)).0.is_none());
        assert!(react(&[0x0f, REQUEST_ACTIVE_SOURCE], &me(true, false)).0.is_none());
    }

    #[test]
    fn refuses_only_unknown_directed_messages() {
        // Give Deck Status (0x1a) to rustv: Feature Abort.
        assert_eq!(
            react(&[0x04, 0x1a, 0x01], &me(true, true)).0,
            Some((0, vec![FEATURE_ABORT, 0x1a, ABORT_UNRECOGNIZED]))
        );
        // Broadcasts, Feature Abort itself and what the kernel answers stay unanswered.
        assert_eq!(react(&[0x0f, 0x1a], &me(true, true)), (None, None));
        assert_eq!(react(&[0x04, FEATURE_ABORT, 0x1a, 0x00], &me(true, true)), (None, None));
        assert_eq!(react(&[0x04, 0x83], &me(true, true)), (None, None));
        // A poll.
        assert_eq!(react(&[0x04], &me(true, true)), (None, None));
    }

    #[test]
    fn remote_buttons() {
        assert_eq!(react(&[0x04, USER_CONTROL_PRESSED, 0x01], &me(true, true)), (None, Some(Event::Button(0x01))));
        assert_eq!(react(&[0x04, USER_CONTROL_RELEASED], &me(true, true)), (None, Some(Event::Released)));
        assert!(matches!(button(0x01), Some(Button::Key("ArrowUp"))));
        assert!(matches!(button(0x25), Some(Button::Key("Digit5"))));
        assert!(matches!(button(0x0d), Some(Button::Key("Escape"))));
        assert!(matches!(button(0x0a), Some(Button::Key("ContextMenu"))));
        assert!(matches!(button(0x48), Some(Button::Media(media::Action::Seek, -10))));
        assert!(matches!(button(0x41), Some(Button::Volume(VolumeAction::Up))));
        assert!(button(0x71).is_none());
    }

    #[test]
    fn routes() {
        assert_eq!(route(0x1000, 0x1000), Route::Us);
        assert_eq!(route(0x1000, 0x1200), Route::Above);
        assert_eq!(route(0x2000, 0x1200), Route::Other);
        assert_eq!(route(0x1300, 0x1200), Route::Other);
        assert_eq!(route(0x0000, 0x1000), Route::Other);
        assert_eq!(route(0x1000, PHYS_ADDR_INVALID), Route::Other);
    }

    #[test]
    fn formats_addresses() {
        assert_eq!(format_address(0x1200), "1.2.0.0");
        assert_eq!(format_address(PHYS_ADDR_INVALID), "unknown");
    }
}
