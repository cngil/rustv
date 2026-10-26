"use strict";

// rustv web remote. Talks to tvd over one WebSocket; see protocol.rs for the messages.

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => document.querySelectorAll(sel);
const TOKEN_KEY = "rustv.token";
const TAB_KEY = "rustv.tab";
const LAUNCH_TIMEOUT = 30000;
const OUTPUT_TIMEOUT = 8000;

let token = readToken();
let ws = null;
let retry = 0;
let apps = [];
let state = null;
let audio = { max_volume: 1.0, step: 0.05, outputs: [] };
let settings = null;
let settingsBefore = null; // restored if tvd rejects a change

// Actions waiting for the TV to confirm them, so the remote can show progress and the result.
let pendingLaunch = null; // { id, name, timer }
let pendingOutput = null; // { id, timer }
let pendingStop = false;

function readToken() {
  const match = location.hash.match(/token=([A-Za-z0-9]+)/);
  if (match) {
    localStorage.setItem(TOKEN_KEY, match[1]);
    history.replaceState(null, "", location.pathname);
  }
  return localStorage.getItem(TOKEN_KEY);
}

// ---- connection ----

async function start() {
  if (!token) return showPair();
  try {
    const res = await fetch("/api/v1/state", { headers: { Authorization: `Bearer ${token}` } });
    if (res.status === 401) return showPair("That token was rejected.");
  } catch {
    // Offline for now; connect() keeps retrying.
  }
  $("#pair").hidden = true;
  $("#app").hidden = false;
  connect();
}

function connect() {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  ws = new WebSocket(`${proto}://${location.host}/api/v1/ws?token=${encodeURIComponent(token)}`);
  ws.onopen = () => {
    retry = 0;
    setOnline(true);
  };
  ws.onmessage = (event) => handle(JSON.parse(event.data));
  ws.onclose = () => {
    setOnline(false);
    setTimeout(connect, Math.min(5000, 250 * 2 ** retry++));
  };
}

function setOnline(online) {
  $("#dot").classList.toggle("on", online);
  $("#app").classList.toggle("offline", !online);
  $("#conn").textContent = online ? "Now on TV" : "Reconnecting…";
}

let offlineNotice = 0;
function send(msg) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(msg));
    return true;
  }
  if (Date.now() - offlineNotice > 2500) {
    offlineNotice = Date.now();
    toast("Not connected to the TV", "error");
  }
  return false;
}

function handle(msg) {
  if (msg.type === "apps") {
    apps = msg.apps;
    renderDock();
  } else if (msg.type === "audio") {
    audio = msg;
    settleOutput();
    renderAudio();
  } else if (msg.type === "state") {
    const before = state && state.running;
    state = msg;
    settleApps(before);
    renderState();
  } else if (msg.type === "settings") {
    settings = msg;
    settingsBefore = null;
    renderSettings();
  } else if (msg.type === "error") {
    if (settingsBefore) {
      settings = settingsBefore;
      settingsBefore = null;
      renderSettings();
    }
    clearLaunch();
    clearOutput();
    toast(msg.message, "error");
  }
}

function showPair(message) {
  $("#app").hidden = true;
  $("#pair").hidden = false;
  if (message) toast(message, "error");
}

$("#pair-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const value = $("#pair-token").value.trim();
  if (!value) return;
  token = value;
  localStorage.setItem(TOKEN_KEY, token);
  start();
});

// ---- pending actions ----

function settleApps(before) {
  const running = state.running;
  if (pendingLaunch && running && running.id === pendingLaunch.id) {
    toast(`${running.name} is open on the TV`, "ok");
    clearLaunch();
  } else if (pendingStop && !running) {
    pendingStop = false;
    if (before) toast(`Closed ${before.name}`);
  }
}

function clearLaunch() {
  if (!pendingLaunch) return;
  clearTimeout(pendingLaunch.timer);
  pendingLaunch = null;
  renderDock();
}

function settleOutput() {
  if (!pendingOutput) return;
  const output = audio.outputs.find((o) => o.id === pendingOutput.id);
  if (output && output.default) {
    toast(`Sound now plays on ${output.name}`, "ok");
    clearOutput();
    closeSheet();
  }
}

function clearOutput() {
  if (!pendingOutput) return;
  clearTimeout(pendingOutput.timer);
  pendingOutput = null;
  renderAudio();
}

// ---- rendering ----

function renderState() {
  const running = state.running;
  $("#now").textContent = state.asleep ? "Asleep" : running ? running.name : "Home";
  $("#conn").textContent = state.asleep ? "Any button wakes the TV" : "Now on TV";
  $("#stop").hidden = !running;
  renderSettings();
  renderVolume();
  renderDock();
}

function renderVolume() {
  const volume = state && state.volume;
  const box = $("#volume");
  box.classList.toggle("muted", Boolean(volume && volume.muted));
  $("#mute").setAttribute("aria-label", volume && volume.muted ? "Unmute" : "Mute");
  $("#mute use").setAttribute("href", volume && volume.muted ? "#i-muted" : "#i-speaker");
  if (!volume) {
    $("#vol-value").textContent = "–";
    return;
  }
  if (!sliderBusy()) {
    slider.value = Math.round(volume.level * 100);
    paintSlider();
  }
  $("#vol-value").textContent = volume.muted ? "Muted" : `${Math.round((sliderBusy() ? slider.value : volume.level * 100))}%`;
}

const OUTPUT_ICONS = { hdmi: "#i-tv", analog: "#i-headphones", bluetooth: "#i-bluetooth", usb: "#i-usb", other: "#i-speaker" };

function renderAudio() {
  slider.max = Math.round(audio.max_volume * 100);
  const current = audio.outputs.find((o) => o.default);
  $("#output-name").textContent = current ? current.name : "No output";
  $(".js-output-icon use").setAttribute("href", OUTPUT_ICONS[current ? current.kind : "other"] || "#i-speaker");
  $("#output").disabled = audio.outputs.length === 0;

  const list = $("#outputs");
  list.replaceChildren();
  if (!audio.outputs.length) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = "No audio outputs found.";
    list.append(empty);
  }
  for (const output of audio.outputs) {
    const item = document.createElement("button");
    item.className = "output";
    if (output.default) item.classList.add("selected");
    if (pendingOutput && pendingOutput.id === output.id) item.classList.add("pending");
    item.innerHTML = `
      <span class="out-icon"><svg><use href="${OUTPUT_ICONS[output.kind] || "#i-speaker"}"/></svg></span>
      <span class="out-text"><div class="out-name"></div><div class="out-detail"></div></span>
      <svg class="out-mark"><use href="#i-check"/></svg>`;
    item.querySelector(".out-name").textContent = output.name;
    item.querySelector(".out-detail").textContent = output.default ? "Playing here" : output.detail;
    item.addEventListener("click", () => chooseOutput(output));
    list.append(item);
  }
  renderVolume();
}

function chooseOutput(output) {
  if (output.default) return closeSheet();
  if (pendingOutput) return;
  haptic();
  if (!send({ type: "audio_output", id: output.id })) return;
  pendingOutput = {
    id: output.id,
    timer: setTimeout(() => {
      clearOutput();
      toast(`Could not switch to ${output.name}`, "error");
    }, OUTPUT_TIMEOUT),
  };
  renderAudio();
}

// Press and hold an app to rearrange the dock; the TV follows the same order.
let editing = false;
let drag = null; // { item, pointerId, lastX, offset }
let dockDirty = false;

function renderDock() {
  // Re-rendering mid-drag would drop the app out from under the finger.
  if (drag) {
    dockDirty = true;
    return;
  }
  const dock = $("#dock");
  dock.replaceChildren();
  dock.classList.toggle("editing", editing);
  const runningId = state && state.running ? state.running.id : null;
  for (const app of apps) {
    const item = document.createElement("button");
    item.className = "dock-item";
    item.dataset.id = app.id;
    if (app.id === runningId) item.classList.add("running");
    if (pendingLaunch && pendingLaunch.id === app.id) item.classList.add("opening");
    item.title = editing ? `Drag to move ${app.name}` : app.description || app.name;
    item.append(appIcon(app));
    const name = document.createElement("span");
    name.className = "dock-name";
    name.textContent = pendingLaunch && pendingLaunch.id === app.id ? "Opening…" : app.name;
    item.append(name);
    bindDockItem(item, app, runningId);
    dock.append(item);
  }
  if (editing) {
    const done = document.createElement("button");
    done.className = "dock-done";
    done.textContent = "Done";
    done.addEventListener("click", finishEditing);
    dock.append(done);
  }
  updateDockFade();
}

// Fade the edge that has more apps behind it, so a cut-off icon reads as "scroll".
function updateDockFade() {
  const dock = $("#dock");
  const more = dock.scrollWidth - dock.clientWidth;
  dock.classList.toggle("more-left", more > 2 && dock.scrollLeft > 2);
  dock.classList.toggle("more-right", more > 2 && dock.scrollLeft < more - 2);
}

$("#dock").addEventListener("scroll", updateDockFade, { passive: true });
addEventListener("resize", updateDockFade);

function bindDockItem(item, app, runningId) {
  let hold = null;
  let start = null;
  let held = false;
  const cancelHold = () => {
    clearTimeout(hold);
    hold = null;
  };
  item.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    if (editing) return startDrag(item, event);
    held = false;
    start = { x: event.clientX, y: event.clientY };
    hold = setTimeout(() => {
      held = true;
      startEditing();
    }, 450);
  });
  item.addEventListener("pointermove", (event) => {
    if (drag && drag.item === item) return moveDrag(event);
    // Scrolling the dock is not a long press.
    if (hold && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 10) cancelHold();
  });
  for (const type of ["pointerup", "pointercancel"]) {
    item.addEventListener(type, () => {
      cancelHold();
      if (drag && drag.item === item) endDrag();
    });
  }
  item.addEventListener("pointerleave", cancelHold);
  item.addEventListener("contextmenu", (event) => event.preventDefault());
  item.addEventListener("click", () => {
    if (editing || held) return;
    launch(app, runningId);
  });
}

function startEditing() {
  if (navigator.vibrate) navigator.vibrate(25);
  editing = true;
  renderDock();
  toast("Drag apps to reorder them, then tap Done");
}

function finishEditing() {
  haptic();
  editing = false;
  renderDock();
  toast("Order saved on the TV", "ok");
}

function startDrag(item, event) {
  item.setPointerCapture(event.pointerId);
  drag = { item, lastX: event.clientX, offset: 0 };
  item.classList.add("dragging");
}

function moveDrag(event) {
  const { item } = drag;
  drag.offset += event.clientX - drag.lastX;
  drag.lastX = event.clientX;
  // Swap with a neighbour once the dragged app passes its middle, then correct
  // the offset by how far the swap moved the app in the layout.
  const center = item.offsetLeft + item.offsetWidth / 2 + drag.offset;
  const next = item.nextElementSibling;
  const prev = item.previousElementSibling;
  let neighbour = null;
  if (next && next.classList.contains("dock-item") && center > next.offsetLeft + next.offsetWidth / 2) {
    neighbour = () => item.parentNode.insertBefore(next, item);
  } else if (prev && center < prev.offsetLeft + prev.offsetWidth / 2) {
    neighbour = () => item.parentNode.insertBefore(item, prev);
  }
  if (neighbour) {
    const before = item.offsetLeft;
    neighbour();
    drag.offset -= item.offsetLeft - before;
    haptic();
  }
  item.style.transform = `translateX(${drag.offset}px) scale(1.08)`;
}

function endDrag() {
  const { item } = drag;
  drag = null;
  item.classList.remove("dragging");
  item.style.transform = "";
  const ids = [...$$("#dock .dock-item")].map((el) => el.dataset.id);
  const changed = ids.some((id, i) => !apps[i] || apps[i].id !== id);
  if (changed) {
    apps = ids.map((id) => apps.find((app) => app.id === id)).filter(Boolean);
    send({ type: "reorder", ids });
  }
  if (dockDirty) {
    dockDirty = false;
    renderDock();
  }
}

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && editing && !drag) finishEditing();
});

function launch(app, runningId) {
  haptic();
  if (app.id === runningId) return toast(`${app.name} is already on the TV`);
  if (pendingLaunch && pendingLaunch.id === app.id) return;
  if (!send({ type: "launch", id: app.id })) return;
  clearLaunch();
  pendingLaunch = {
    id: app.id,
    timer: setTimeout(() => {
      clearLaunch();
      toast(`${app.name} did not open`, "error");
    }, LAUNCH_TIMEOUT),
  };
  renderDock();
}

function appIcon(app) {
  if (app.icon) {
    const img = document.createElement("img");
    img.className = "dock-icon";
    img.alt = "";
    img.src = `/api/v1/apps/${encodeURIComponent(app.id)}/icon?token=${encodeURIComponent(token)}`;
    return img;
  }
  const tile = document.createElement("span");
  tile.className = "dock-icon letter";
  tile.style.setProperty("--hue", hue(app.id));
  tile.textContent = app.name.slice(0, 1).toUpperCase();
  return tile;
}

function hue(text) {
  let h = 0;
  for (const c of text) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}

let toastTimer = null;
function toast(text, kind = "info") {
  const el = $("#toast");
  el.textContent = text;
  el.className = `toast ${kind}`;
  el.hidden = false;
  // Restart the entry animation for back-to-back messages.
  void el.offsetWidth;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), kind === "error" ? 4500 : 2600);
}

function haptic() {
  if (navigator.vibrate) navigator.vibrate(8);
}

// ---- buttons ----

// Hold to repeat, like a real remote.
function holdable(button, action) {
  let delay = null;
  let interval = null;
  const release = () => {
    clearTimeout(delay);
    clearInterval(interval);
    button.classList.remove("pressed");
  };
  button.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    button.classList.add("pressed");
    haptic();
    action();
    delay = setTimeout(() => (interval = setInterval(action, 110)), 400);
  });
  for (const ev of ["pointerup", "pointerleave", "pointercancel"]) button.addEventListener(ev, release);
  // Keyboard activation (Enter/Space on a focused button).
  button.addEventListener("click", (event) => {
    if (event.detail === 0) action();
  });
}

for (const button of $$("[data-key]")) {
  const sendKey = () => send({ type: "key", code: button.dataset.key });
  if ("repeat" in button.dataset) {
    holdable(button, sendKey);
  } else {
    button.addEventListener("click", () => {
      haptic();
      sendKey();
    });
  }
}

for (const button of $$("[data-vol]")) {
  holdable(button, () => send({ type: "volume", action: button.dataset.vol }));
}

for (const button of $$("[data-btn]")) {
  button.addEventListener("click", () => {
    haptic();
    send({ type: "button", button: button.dataset.btn });
  });
}

function stopApp() {
  haptic();
  if (send({ type: "stop" })) pendingStop = true;
}

$("#home").addEventListener("click", stopApp);
$("#stop").addEventListener("click", stopApp);
$("#mute").addEventListener("click", () => {
  haptic();
  send({ type: "volume", action: "mute" });
});

// ---- output sheet ----

function openSheet() {
  haptic();
  renderAudio();
  $("#sheet").hidden = false;
}

function closeSheet() {
  $("#sheet").hidden = true;
}

$("#output").addEventListener("click", openSheet);
for (const el of $$("[data-close]")) el.addEventListener("click", closeSheet);
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !$("#sheet").hidden) closeSheet();
});

// ---- settings ----

// Each `key` is a config value (`section.name`) that tvd saves to config.toml.
// `live` items change the TV right now without touching the config.
const SETTINGS = [
  {
    title: "Home screen",
    items: [
      { key: "shell.greeting", label: "Greeting" },
      { key: "shell.clock", label: "Clock" },
      { key: "shell.date", label: "Date" },
      { key: "shell.hints", label: "Key hints" },
      { key: "shell.volume_overlay", label: "Volume bar when the volume changes" },
      { key: "shell.sounds", label: "Sounds", hint: "Soft clicks when moving between apps and opening one." },
      {
        key: "shell.sound_volume",
        label: "Sound level",
        needs: "shell.sounds",
        options: [[0.25, "Quiet"], [0.5, "Medium"], [1, "Loud"]],
      },
      { key: "shell.pairing_qr", label: "Pairing code", hint: "QR code on the TV until a phone pairs, and when you ask for it." },
      {
        action: "show_pairing",
        label: "Pair another phone or computer",
        button: "Show code",
        done: "The pairing code is on the TV for 2 minutes",
        needs: "shell.pairing_qr",
      },
    ],
  },
  {
    title: "Screensaver",
    items: [
      { key: "screensaver.enabled", label: "Screensaver" },
      {
        key: "screensaver.idle_minutes",
        label: "Start after",
        needs: "screensaver.enabled",
        options: [[1, "1 min"], [2, "2 min"], [5, "5 min"], [10, "10 min"], [15, "15 min"], [30, "30 min"], [60, "1 h"]],
      },
      {
        key: "screensaver.style",
        label: "Style",
        needs: "screensaver.enabled",
        options: [["random", "Mix"], ["aurora", "Aurora"], ["orbits", "Orbits"], ["clock", "Clock"]],
      },
      { key: "screensaver.clock", label: "Show the time", needs: "screensaver.enabled" },
      { key: "screensaver.date", label: "Show the date", needs: "screensaver.enabled" },
    ],
  },
  {
    title: "Sound",
    items: [
      {
        key: "audio.max_volume",
        label: "Volume limit",
        options: [[1, "100%"], [1.2, "120%"], [1.5, "150%"]],
        hint: "Above 100% the sound is boosted in software and can distort.",
      },
      { key: "audio.step", label: "Volume buttons change by", options: [[0.02, "2%"], [0.05, "5%"], [0.1, "10%"]] },
    ],
  },
  {
    title: "Screen casting",
    items: [
      {
        key: "cast.airplay",
        label: "AirPlay",
        hint: "Mirror an iPhone, iPad or Mac, or play videos and music from them. Needs uxplay on the box.",
      },
    ],
  },
  {
    title: "Sleep",
    items: [
      {
        key: "sleep.enabled",
        label: "Sleep when unused",
        hint: "Screen off after no remote input and no sound. Any button wakes it.",
      },
      {
        key: "sleep.after_minutes",
        label: "After",
        needs: "sleep.enabled",
        options: [[15, "15 min"], [30, "30 min"], [60, "1 h"], [120, "2 h"], [240, "4 h"]],
      },
      { key: "sleep.close_apps", label: "Close the open app", hint: "Frees memory and CPU." },
      { action: "sleep", label: "Put the TV to sleep now", button: "Sleep", done: "TV is going to sleep. Any button wakes it." },
    ],
  },
  {
    title: "Debug overlay",
    items: [
      { live: "overlay", label: "Show on the TV now", hint: "CPU, GPU, RAM, network and more, over everything." },
      { key: "overlay.enabled", label: "Show at startup" },
      { key: "overlay.position", label: "Position", options: [["top", "Top"], ["bottom", "Bottom"]] },
    ],
  },
];

function settingValue(key) {
  // Action rows (Sleep now, Show code) have no value.
  if (!key) return undefined;
  const [section, name] = key.split(".");
  return settings && settings[section] ? settings[section][name] : undefined;
}

function sameValue(a, b) {
  return typeof a === "number" && typeof b === "number" ? Math.abs(a - b) < 1e-6 : a === b;
}

function changeSetting(key, value) {
  haptic();
  if (!send({ type: "set", key, value })) return;
  // Show the change at once; the next `settings` message confirms it (or an error puts it back).
  if (!settingsBefore) settingsBefore = structuredClone(settings);
  const [section, name] = key.split(".");
  settings[section][name] = value;
  renderSettings();
}

function renderSettings() {
  const body = $("#settings-body");
  if (!body || $("#settings").hidden) return;
  body.replaceChildren();
  if (!settings) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = "Waiting for the TV…";
    body.append(empty);
    return;
  }
  for (const group of SETTINGS) {
    const section = document.createElement("section");
    section.className = "settings-group";
    const title = document.createElement("h3");
    title.textContent = group.title;
    section.append(title);
    for (const item of group.items) section.append(settingRow(item));
    body.append(section);
  }
  const about = document.createElement("p");
  about.className = "settings-about";
  about.textContent = `rustv ${settings.version} · saved to ${settings.config_path}`;
  body.append(about);
}

function settingRow(item) {
  const row = document.createElement("div");
  row.className = "setting";
  const off = item.needs && !settingValue(item.needs);
  row.classList.toggle("disabled", Boolean(off));

  const text = document.createElement("div");
  text.className = "setting-text";
  const label = document.createElement("div");
  label.className = "setting-label";
  label.textContent = item.label;
  text.append(label);
  if (item.hint) {
    const hint = document.createElement("div");
    hint.className = "setting-hint";
    hint.textContent = item.hint;
    text.append(hint);
  }
  row.append(text);

  const value = item.live === "overlay" ? Boolean(state && state.overlay) : settingValue(item.key);
  if (item.action) {
    const button = document.createElement("button");
    button.className = "chip";
    button.textContent = item.button;
    button.disabled = Boolean(off);
    button.addEventListener("click", () => {
      haptic();
      if (send({ type: item.action })) {
        closeSettings();
        toast(item.done);
      }
    });
    row.append(button);
  } else if (item.options) {
    row.classList.add("stacked");
    const chips = document.createElement("div");
    chips.className = "chips";
    const options = [...item.options];
    // A value set in the config file that is not one of the presets.
    if (value !== undefined && !options.some(([v]) => sameValue(v, value))) options.push([value, String(value)]);
    for (const [optionValue, optionLabel] of options) {
      const chip = document.createElement("button");
      chip.className = "chip";
      chip.textContent = optionLabel;
      chip.disabled = Boolean(off);
      chip.setAttribute("aria-pressed", String(sameValue(optionValue, value)));
      chip.addEventListener("click", () => {
        if (!sameValue(optionValue, value)) changeSetting(item.key, optionValue);
      });
      chips.append(chip);
    }
    row.append(chips);
  } else {
    const toggle = document.createElement("button");
    toggle.className = "toggle";
    toggle.setAttribute("role", "switch");
    toggle.setAttribute("aria-checked", String(Boolean(value)));
    toggle.setAttribute("aria-label", item.label);
    toggle.disabled = Boolean(off);
    toggle.addEventListener("click", () => {
      if (item.live === "overlay") {
        haptic();
        send({ type: "overlay", visible: !value });
      } else {
        changeSetting(item.key, !value);
      }
    });
    row.append(toggle);
  }
  return row;
}

function openSettings() {
  haptic();
  $("#settings").hidden = false;
  renderSettings();
}

function closeSettings() {
  $("#settings").hidden = true;
}

$("#settings-open").addEventListener("click", openSettings);
for (const el of $$("[data-close-settings]")) el.addEventListener("click", closeSettings);
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !$("#settings").hidden) closeSettings();
});

// ---- volume slider ----

const slider = $("#vol-slider");
let sliderTouched = 0;
let sliderTimer = null;

function sliderBusy() {
  return Date.now() - sliderTouched < 800;
}

function paintSlider() {
  slider.style.setProperty("--fill", `${(slider.value / slider.max) * 100}%`);
}

slider.addEventListener("input", () => {
  sliderTouched = Date.now();
  paintSlider();
  $("#vol-value").textContent = `${slider.value}%`;
  if (sliderTimer) return;
  // At most one update every 80 ms while dragging.
  sliderTimer = setTimeout(() => {
    sliderTimer = null;
    send({ type: "volume", action: "set", level: slider.value / 100 });
  }, 80);
});
slider.addEventListener("change", () => {
  sliderTouched = Date.now();
  send({ type: "volume", action: "set", level: slider.value / 100 });
});

// ---- touchpad ----

const pad = $("#pad");
const pointers = new Map();
let acc = { dx: 0, dy: 0, sx: 0, sy: 0 };
let gesture = { moved: false, start: 0, fingers: 0 };
let frame = null;

function flush() {
  frame = null;
  const mx = Math.trunc(acc.dx);
  const my = Math.trunc(acc.dy);
  if (mx || my) {
    send({ type: "move", dx: mx, dy: my });
    acc.dx -= mx;
    acc.dy -= my;
  }
  const sx = Math.trunc(acc.sx);
  const sy = Math.trunc(acc.sy);
  if (sx || sy) {
    send({ type: "scroll", dx: sx, dy: sy });
    acc.sx -= sx;
    acc.sy -= sy;
  }
}

function schedule() {
  if (!frame) frame = requestAnimationFrame(flush);
}

// Faster swipes move further, slow ones stay precise.
function accelerate(d, dt) {
  const speed = Math.abs(d) / Math.max(dt, 1);
  return d * Math.min(1.4 + speed * 1.6, 5);
}

pad.addEventListener("pointerdown", (event) => {
  if (event.pointerType === "mouse") return;
  pad.setPointerCapture(event.pointerId);
  pad.classList.add("touching");
  pointers.set(event.pointerId, { x: event.clientX, y: event.clientY, t: event.timeStamp });
  if (pointers.size === 1) gesture = { moved: false, start: event.timeStamp, fingers: 1 };
  gesture.fingers = Math.max(gesture.fingers, pointers.size);
});

pad.addEventListener("pointermove", (event) => {
  const last = pointers.get(event.pointerId);
  if (!last) return;
  const dx = event.clientX - last.x;
  const dy = event.clientY - last.y;
  const dt = event.timeStamp - last.t;
  pointers.set(event.pointerId, { x: event.clientX, y: event.clientY, t: event.timeStamp });
  if (Math.abs(dx) + Math.abs(dy) > 1) gesture.moved = true;
  if (pointers.size >= 2) {
    acc.sy -= dy / 12 / pointers.size;
    acc.sx += dx / 12 / pointers.size;
  } else {
    acc.dx += accelerate(dx, dt);
    acc.dy += accelerate(dy, dt);
  }
  schedule();
});

function endTouch(event) {
  if (!pointers.delete(event.pointerId) || pointers.size > 0) return;
  pad.classList.remove("touching");
  if (!gesture.moved && event.timeStamp - gesture.start < 250) {
    haptic();
    send({ type: "button", button: gesture.fingers >= 2 ? "right" : "left" });
  }
}
pad.addEventListener("pointerup", endTouch);
pad.addEventListener("pointercancel", endTouch);

// Desktop: lock the mouse to the pad and drive the TV pointer directly.
pad.addEventListener("click", () => {
  if (!matchMedia("(pointer: fine)").matches) return;
  if (document.pointerLockElement !== pad) pad.requestPointerLock();
});

document.addEventListener("pointerlockchange", () => {
  pad.classList.toggle("locked", document.pointerLockElement === pad);
});

document.addEventListener("mousemove", (event) => {
  if (document.pointerLockElement !== pad) return;
  acc.dx += event.movementX;
  acc.dy += event.movementY;
  schedule();
});

const MOUSE_BUTTONS = ["left", "middle", "right"];
for (const [type, action] of [["mousedown", "down"], ["mouseup", "up"]]) {
  document.addEventListener(type, (event) => {
    if (document.pointerLockElement !== pad) return;
    const button = MOUSE_BUTTONS[event.button];
    if (button) send({ type: "button", button, action });
  });
}

pad.addEventListener(
  "wheel",
  (event) => {
    event.preventDefault();
    acc.sy -= Math.sign(event.deltaY);
    acc.sx += Math.sign(event.deltaX);
    schedule();
  },
  { passive: false },
);
pad.addEventListener("contextmenu", (event) => event.preventDefault());

// ---- keyboard ----

$("#text-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const input = $("#text");
  if (!input.value) return send({ type: "key", code: "Enter" });
  if (send({ type: "text", text: input.value })) {
    toast("Sent to the TV", "ok");
    input.value = "";
  }
});

const MODIFIERS = new Set(["ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight"]);

// Light up the matching on-screen button when a forwarded key is pressed.
function flash(code) {
  const button = document.querySelector(`.dpad [data-key="${code}"]`);
  if (!button) return;
  button.classList.add("pressed");
  setTimeout(() => button.classList.remove("pressed"), 140);
}

document.addEventListener("keydown", (event) => {
  const forwarding = $("#capture").checked || document.pointerLockElement === pad;
  if (!forwarding || event.target === $("#text") || MODIFIERS.has(event.code) || !$("#sheet").hidden || !$("#settings").hidden) return;
  if (event.code === "Escape" && document.pointerLockElement === pad) return; // browser releases the lock
  event.preventDefault();
  flash(event.code);
  send({
    type: "key",
    code: event.code,
    shift: event.shiftKey,
    ctrl: event.ctrlKey,
    alt: event.altKey,
    meta: event.metaKey,
  });
});

// ---- tabs ----

function showTab(name) {
  if (!document.querySelector(`.view[data-view="${name}"]`)) name = "remote";
  for (const view of $$(".view")) view.classList.toggle("active", view.dataset.view === name);
  for (const tab of $$("[data-tab]")) tab.classList.toggle("active", tab.dataset.tab === name);
  localStorage.setItem(TAB_KEY, name);
}

for (const tab of $$("[data-tab]")) {
  tab.addEventListener("click", () => {
    haptic();
    showTab(tab.dataset.tab);
  });
}
showTab(localStorage.getItem(TAB_KEY) || "remote");
paintSlider();

start();
