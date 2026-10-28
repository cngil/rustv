"use strict";

// rustv web remote. Talks to tvd over one WebSocket; see protocol.rs for the messages.

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => document.querySelectorAll(sel);
// ---- language ----

// Every text comes from the shared catalog (crates/locales/*.toml), which tvd
// serves as /locales.js: `{ en: { key: text }, tr: { ... } }`. Keys are stable ids
// such as "remote.timer.title"; `{name}` marks a value `t()` fills in.
const LOCALES = window.LOCALES || { en: {} };

const LANG_KEY = "rustv.lang";
let lang = localStorage.getItem(LANG_KEY) || (navigator.language || "").slice(0, 2);
if (!LOCALES[lang]) lang = "en";

/** The CLDR plural category; must match plural_form() in crates/locales. */
function pluralForm(n) {
  return lang !== "tr" && Number(n) === 1 ? "one" : "other";
}

/** The text for `key` in the current language (English if it lacks one), with `vars` filled in. */
function t(key, vars = {}) {
  if (key == null) return key;
  const lookup = (k) => (LOCALES[lang] && LOCALES[lang][k]) ?? LOCALES.en[k];
  let text = vars.n !== undefined ? lookup(`${key}.${pluralForm(vars.n)}`) : undefined;
  text = text ?? lookup(key);
  // A missing key shows as itself rather than vanishing.
  if (text === undefined) return key;
  return String(text).replace(/\{(\w+)\}/g, (match, name) => (name in vars ? vars[name] : match));
}

/** A text tvd words for the screens (`{ key, args }`), or its English fallback. */
function worded(label, english) {
  return label ? t(label.key, label.args || {}) : english;
}

/** "50%" in English, "%50" in Turkish. */
function percent(value) {
  return t("unit.percent", { n: value });
}

/** Static text in the page carries its key: `data-i18n` for the text, `data-i18n-<attribute>` for attributes. */
function applyLanguage() {
  document.documentElement.lang = lang;
  for (const el of $$("[data-i18n]")) el.textContent = t(el.dataset.i18n);
  for (const attr of ["aria-label", "placeholder", "title"]) {
    const data = `data-i18n-${attr}`;
    for (const el of $$(`[${data}]`)) el.setAttribute(attr, t(el.getAttribute(data)));
  }
}

function setLanguage(next) {
  if (!LOCALES[next]) next = "en";
  if (next === lang) return;
  lang = next;
  localStorage.setItem(LANG_KEY, lang);
  applyLanguage();
  if (state) renderState();
  renderAudio();
  renderDock();
  renderSettings();
  renderPin();
}

// ---- theme ----

const THEME_KEY = "rustv.theme";
let palette = null;

// The palette tvd works out from [theme]; cached so the next load starts in the right colors.
function applyTheme(p) {
  if (!p) return;
  palette = p;
  const root = document.documentElement.style;
  const vars = {
    "--accent": p.accent,
    "--accent-2": p.accent_2,
    "--bg": p.background,
    "--bg-top": p.background_top,
    "--surface": p.surface,
    "--surface-2": p.surface_2,
    "--surface-3": p.surface_3,
    "--line": p.line,
  };
  // Palettes from an older tvd lack these; the stylesheet's defaults stay then.
  if (p.on_accent) Object.assign(vars, { "--on-accent": p.on_accent, "--text": p.text, "--muted": p.muted });
  for (const [name, value] of Object.entries(vars)) root.setProperty(name, value);
  document.querySelector('meta[name="theme-color"]').setAttribute("content", p.background);
  localStorage.setItem(THEME_KEY, JSON.stringify(p));
}

try {
  applyTheme(JSON.parse(localStorage.getItem(THEME_KEY)));
} catch {
  // No saved theme yet.
}

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

function start() {
  if (!token) return showPair();
  // Show the remote and connect at once; checking the token runs alongside and
  // only takes over if the token was revoked.
  $("#pair").hidden = true;
  $("#app").hidden = false;
  connect();
  fetch("/api/v1/state", { headers: { Authorization: `Bearer ${token}` } })
    .then((res) => {
      if (res.status === 401) {
        if (ws) ws.onclose = null;
        if (ws) ws.close();
        showPair(t("remote.pair.rejected"));
      }
    })
    .catch(() => {
      // Offline for now; connect() keeps retrying.
    });
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
  $("#conn").textContent = t(online ? "common.connected" : "remote.status.reconnecting");
}

let offlineNotice = 0;
function send(msg) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(msg));
    return true;
  }
  if (Date.now() - offlineNotice > 2500) {
    offlineNotice = Date.now();
    toast(t("remote.status.not_connected"), "error");
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
  } else if (msg.type === "theme") {
    applyTheme(msg);
    renderSettings();
  } else if (msg.type === "settings") {
    settings = msg;
    settingsBefore = null;
    if (settings.locale) setLanguage(settings.locale.language);
    renderSettings();
  } else if (msg.type === "error") {
    if (settingsBefore) {
      settings = settingsBefore;
      settingsBefore = null;
      renderSettings();
    }
    clearLaunch();
    clearOutput();
    toast(msg.key ? t(msg.key, msg.args || {}) : msg.message, "error");
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
    toast(t("remote.app.opened", { name: running.name }), "ok");
    clearLaunch();
  } else if (pendingStop && !running) {
    pendingStop = false;
    if (before) toast(t("remote.app.closed", { name: before.name }));
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
    toast(t("remote.output.switched", { name: worded(output.name_label, output.name) }), "ok");
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
  $("#now").textContent = state.asleep ? t("remote.status.asleep") : running ? running.name : t("common.home");
  $("#conn").textContent = state.asleep
    ? t("remote.status.any_button_wakes")
    : sleepLeft() !== null
      ? t("remote.status.sleeps_in", { minutes: Math.max(1, Math.ceil(sleepLeft() / 60)) })
      : t("common.connected");
  $("#timer-open").classList.toggle("active", sleepLeft() !== null);
  renderTimer();
  $("#stop").hidden = !running;
  renderSettings();
  syncLock();
  renderTrial();
  renderVolume();
  renderDock();
}

function renderVolume() {
  const volume = state && state.volume;
  const box = $("#volume");
  box.classList.toggle("muted", Boolean(volume && volume.muted));
  $("#mute").setAttribute("aria-label", t(volume && volume.muted ? "remote.volume.unmute" : "remote.volume.mute"));
  $("#mute use").setAttribute("href", volume && volume.muted ? "#i-muted" : "#i-speaker");
  if (!volume) {
    $("#vol-value").textContent = "–";
    return;
  }
  if (!sliderBusy()) {
    slider.value = Math.round(volume.level * 100);
    paintSlider();
  }
  $("#vol-value").textContent = volume.muted
    ? t("remote.volume.muted")
    : percent(Math.round(sliderBusy() ? slider.value : volume.level * 100));
}

const OUTPUT_ICONS = { hdmi: "#i-tv", analog: "#i-headphones", bluetooth: "#i-bluetooth", usb: "#i-usb", other: "#i-speaker" };

function renderAudio() {
  slider.max = Math.round(audio.max_volume * 100);
  const current = audio.outputs.find((o) => o.default);
  $("#output-name").textContent = current ? worded(current.name_label, current.name) : t("remote.output.none");
  $(".js-output-icon use").setAttribute("href", OUTPUT_ICONS[current ? current.kind : "other"] || "#i-speaker");
  $("#output").disabled = audio.outputs.length === 0;

  const list = $("#outputs");
  list.replaceChildren();
  if (!audio.outputs.length) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = t("remote.output.empty");
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
    item.querySelector(".out-name").textContent = worded(output.name_label, output.name);
    item.querySelector(".out-detail").textContent = output.default ? t("remote.output.playing_here") : output.detail;
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
      toast(t("remote.output.failed", { name: worded(output.name_label, output.name) }), "error");
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
    item.title = editing ? t("remote.dock.drag_to_move", { name: app.name }) : app.description || app.name;
    item.append(appIcon(app));
    const name = document.createElement("span");
    name.className = "dock-name";
    name.textContent = pendingLaunch && pendingLaunch.id === app.id ? t("remote.dock.opening") : app.name;
    item.append(name);
    bindDockItem(item, app, runningId);
    dock.append(item);
  }
  if (editing) {
    const done = document.createElement("button");
    done.className = "dock-done";
    done.textContent = t("common.done");
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
  toast(t("remote.dock.reorder_hint"));
}

function finishEditing() {
  haptic();
  editing = false;
  renderDock();
  toast(t("remote.dock.order_saved"), "ok");
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
  if (app.id === runningId) return toast(t("remote.app.already_open", { name: app.name }));
  if (pendingLaunch && pendingLaunch.id === app.id) return;
  if (!send({ type: "launch", id: app.id })) return;
  clearLaunch();
  pendingLaunch = {
    id: app.id,
    timer: setTimeout(() => {
      clearLaunch();
      toast(t("remote.app.did_not_open", { name: app.name }), "error");
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
  tile.textContent = app.name.slice(0, 1).toUpperCase();
  return tile;
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

// ---- media keys ----

for (const button of $$("[data-media]")) {
  button.addEventListener("click", () => {
    haptic();
    send({ type: "media", action: button.dataset.media, seconds: Number(button.dataset.seconds || 0) });
  });
}

// ---- display mode ----

/** "1920x1080@59.94" → { size: "1920x1080", hz: "59.94" } */
function splitMode(mode) {
  const [size, hz] = String(mode).split("@");
  return { size, hz };
}

function modeLabel(mode) {
  const { size, hz } = splitMode(mode);
  return `${size.replace("x", " × ")}, ${hz} Hz`;
}

/** A row of chips that sends the picked value through `pick`. */
function choiceRow(label, hint, options, current, pick) {
  const row = document.createElement("div");
  row.className = "setting stacked";
  const text = document.createElement("div");
  text.className = "setting-text";
  text.innerHTML = '<div class="setting-label"></div>';
  text.querySelector(".setting-label").textContent = label;
  if (hint) {
    const note = document.createElement("div");
    note.className = "setting-hint";
    note.textContent = hint;
    text.append(note);
  }
  const chips = document.createElement("div");
  chips.className = "chips";
  for (const [value, name] of options) {
    const chip = document.createElement("button");
    chip.className = "chip";
    chip.textContent = name;
    chip.setAttribute("aria-pressed", String(value === current));
    chip.addEventListener("click", () => {
      haptic();
      if (value !== current) pick(value);
    });
    chips.append(chip);
  }
  row.append(text, chips);
  return row;
}

function displayRows() {
  const display = settings.display || { modes: [] };
  const brightness = settingRow({
    key: "display.brightness",
    label: label("display.brightness"),
    hint: hint("display.brightness"),
    options: [[0.4, pct(40)], [0.6, pct(60)], [0.8, pct(80)], [1, pct(100)]],
  });
  const warmth = settingRow({
    key: "display.warmth",
    label: label("display.warmth"),
    hint: hint("display.warmth"),
    options: WARMTH,
  });
  if (!display.modes || !display.modes.length) {
    const note = document.createElement("p");
    note.className = "setting-hint";
    note.textContent = t("remote.display.unavailable");
    return [brightness, warmth, note];
  }
  const modes = display.modes.map(splitMode);
  const on = splitMode(display.current || (display.mode !== "auto" ? display.mode : display.modes[0]));
  const sizes = [...new Set(modes.map((m) => m.size))];
  const rates = modes.filter((m) => m.size === on.size).map((m) => m.hz);
  const tryMode = (mode) => send({ type: "display_mode", mode });
  const resolution = choiceRow(
    t("setting.display.resolution.label"),
    t("setting.display.resolution.hint"),
    sizes.map((size) => [size, size.replace("x", " × ")]),
    on.size,
    (size) => {
      // Keep the refresh rate where the new size offers it, else the fastest up to 60 Hz.
      const offered = modes.filter((m) => m.size === size).map((m) => m.hz);
      const hz = offered.includes(on.hz) ? on.hz : offered.find((h) => Number(h) <= 60) || offered[0];
      tryMode(`${size}@${hz}`);
    },
  );
  const refresh = choiceRow(t("setting.display.refresh.label"), "", rates.map((hz) => [hz, `${hz} Hz`]), on.hz, (hz) =>
    tryMode(`${on.size}@${hz}`),
  );
  return [brightness, warmth, resolution, refresh];
}

/** The "keep this resolution?" prompt, while a mode is on trial. */
function renderTrial() {
  const trial = state && state.display_trial;
  $("#trial").hidden = !trial;
  if (!trial) return;
  $("#trial-text").textContent = t("remote.display.going_back", {
    mode: modeLabel(trial.mode),
    seconds: trial.seconds,
  });
}

$("#trial-keep").addEventListener("click", () => {
  haptic();
  send({ type: "display_keep" });
});
$("#trial-revert").addEventListener("click", () => {
  haptic();
  send({ type: "display_revert" });
});

// ---- sleep timer ----

let sleepInfo = null; // { at: ms when it fires } from the last state

function sleepLeft() {
  if (!state || state.sleep_in == null) return null;
  if (!sleepInfo || sleepInfo.from !== state) sleepInfo = { from: state, at: Date.now() + state.sleep_in * 1000 };
  return Math.max(0, Math.round((sleepInfo.at - Date.now()) / 1000));
}

// The header counts down between state updates.
setInterval(() => {
  if (state && state.sleep_in != null) renderState();
}, 30000);

const TIMER_CHOICES = [15, 30, 45, 60, 90, 120];

function renderTimer() {
  const chips = $("#timer-chips");
  if (!chips || $("#timer").hidden) return;
  const left = sleepLeft();
  $("#timer-status").textContent =
    left === null
      ? t("remote.timer.explain")
      : t("remote.timer.sleeps_in", { minutes: Math.max(1, Math.ceil(left / 60)) });
  chips.replaceChildren();
  for (const minutes of TIMER_CHOICES) {
    const chip = document.createElement("button");
    chip.className = "chip";
    chip.textContent = minutes % 60 ? t("unit.minutes", { n: minutes }) : t("unit.hours", { n: minutes / 60 });
    chip.addEventListener("click", () => {
      haptic();
      if (send({ type: "sleep_timer", minutes })) {
        closeTimer();
        toast(t("remote.timer.sleeps_in", { minutes }), "ok");
      }
    });
    chips.append(chip);
  }
  if (left !== null) {
    const off = document.createElement("button");
    off.className = "chip";
    off.textContent = t("remote.timer.off_button");
    off.addEventListener("click", () => {
      haptic();
      if (send({ type: "sleep_timer", minutes: 0 })) {
        closeTimer();
        toast(t("remote.timer.off"));
      }
    });
    chips.append(off);
  }
}

function closeTimer() {
  $("#timer").hidden = true;
}

$("#timer-open").addEventListener("click", () => {
  haptic();
  $("#timer").hidden = false;
  renderTimer();
});
for (const el of $$("[data-close-timer]")) el.addEventListener("click", closeTimer);

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
// Labels are catalog keys: setting.<config key>.label and .hint, shared with the TV's own settings.
const label = (key) => `setting.${key}.label`;
const hint = (key) => `setting.${key}.hint`;
const unit = (key, n) => ({ key, vars: { n } });
const minutes = (n) => unit("unit.minutes", n);
const hours = (n) => unit("unit.hours", n);
const seconds = (n) => unit("unit.seconds", n);
const pct = (n) => unit("unit.percent", n);
const WARMTH = [[0, "option.warmth.off"], [0.3, "option.warmth.low"], [0.6, "option.warmth.medium"], [1, "option.warmth.high"]];

/** `HH:MM` as the TV's clock shows it: 22:00, or 10 PM with the 12-hour clock. */
function clockTime(time) {
  const [h, m] = time.split(":").map(Number);
  if (!settings || !settings.locale || settings.locale.clock !== "12h") return { text: time };
  const minutes = m ? `:${String(m).padStart(2, "0")}` : "";
  return { text: `${h % 12 || 12}${minutes} ${h < 12 ? "AM" : "PM"}` };
}

/** A label, hint or choice: a catalog key, `{ key, vars }` for one with values, or `{ text }` as is. */
function say(text) {
  if (text && typeof text === "object") return "text" in text ? text.text : t(text.key, text.vars);
  return t(text);
}

const SETTINGS = [
  {
    title: "section.home_screen",
    items: [
      { key: "shell.greeting", label: label("shell.greeting") },
      { key: "shell.clock", label: label("shell.clock") },
      { key: "shell.date", label: label("shell.date") },
      { key: "shell.volume_overlay", label: label("shell.volume_overlay") },
      { key: "shell.sounds", label: label("shell.sounds"), hint: hint("shell.sounds") },
      {
        key: "shell.sound_volume",
        label: label("shell.sound_volume"),
        needs: "shell.sounds",
        options: [
          [0.25, "option.shell.sound_volume.quiet"],
          [0.5, "option.shell.sound_volume.medium"],
          [1, "option.shell.sound_volume.loud"],
        ],
      },
      { key: "shell.pairing_qr", label: label("shell.pairing_qr"), hint: hint("shell.pairing_qr") },
      {
        action: "show_pairing",
        label: label("pairing.show"),
        button: "remote.settings.show_code",
        done: "remote.settings.code_shown",
        needs: "shell.pairing_qr",
      },
      { key: "shell.settings", label: label("shell.settings"), hint: hint("shell.settings") },
    ],
  },
  { title: "section.theme", custom: themeRows },
  { title: "section.display", custom: displayRows },
  {
    title: "section.night",
    items: [
      { key: "night.enabled", label: label("night.enabled"), hint: hint("night.enabled") },
      {
        key: "night.from",
        label: label("night.from"),
        needs: "night.enabled",
        options: ["20:00", "21:00", "22:00", "23:00", "00:00"].map((time) => [time, clockTime(time)]),
      },
      {
        key: "night.to",
        label: label("night.to"),
        needs: "night.enabled",
        options: ["05:00", "06:00", "07:00", "08:00", "09:00"].map((time) => [time, clockTime(time)]),
      },
      {
        key: "night.brightness",
        label: label("night.brightness"),
        needs: "night.enabled",
        options: [[0.4, pct(40)], [0.6, pct(60)], [0.7, pct(70)], [0.8, pct(80)], [1, pct(100)]],
      },
      { key: "night.warmth", label: label("night.warmth"), needs: "night.enabled", options: WARMTH },
      { key: "night.audio", label: label("night.audio"), hint: hint("night.audio"), needs: "night.enabled" },
    ],
  },
  {
    title: "section.screensaver",
    items: [
      { key: "screensaver.enabled", label: label("screensaver.enabled") },
      {
        key: "screensaver.idle_minutes",
        label: label("screensaver.idle_minutes"),
        needs: "screensaver.enabled",
        options: [[1, minutes(1)], [2, minutes(2)], [5, minutes(5)], [10, minutes(10)], [15, minutes(15)], [30, minutes(30)], [60, hours(1)]],
      },
      {
        key: "screensaver.style",
        label: label("screensaver.style"),
        needs: "screensaver.enabled",
        options: ["random", "aurora", "orbits", "clock", "images"].map((style) => [style, `option.screensaver.style.${style}`]),
      },
      { key: "screensaver.clock", label: label("screensaver.clock"), needs: "screensaver.enabled" },
      { key: "screensaver.date", label: label("screensaver.date"), needs: "screensaver.enabled" },
    ],
  },
  {
    title: "section.sound",
    items: [
      {
        key: "audio.max_volume",
        label: label("audio.max_volume"),
        options: [[1, pct(100)], [1.2, pct(120)], [1.5, pct(150)]],
        hint: hint("audio.max_volume"),
      },
      { key: "audio.per_app_volume", label: label("audio.per_app_volume"), hint: hint("audio.per_app_volume") },
      { key: "audio.night", label: label("audio.night"), hint: hint("audio.night") },
      { key: "audio.step", label: label("audio.step"), options: [[0.02, pct(2)], [0.05, pct(5)], [0.1, pct(10)]] },
    ],
  },
  {
    title: "section.language",
    items: [
      { key: "locale.language", label: label("locale.language"), options: [["en", "option.locale.language.en"], ["tr", "option.locale.language.tr"]] },
      { key: "locale.clock", label: label("locale.clock"), options: [["24h", "option.locale.clock.24h"], ["12h", "option.locale.clock.12h"]] },
      {
        key: "locale.temperature",
        label: label("locale.temperature"),
        hint: hint("locale.temperature"),
        options: [["celsius", "option.locale.temperature.celsius"], ["fahrenheit", "option.locale.temperature.fahrenheit"]],
      },
    ],
  },
  {
    title: "section.lock",
    items: [
      {
        label: label("lock.pin"),
        hint: hint("lock.pin"),
        button: () => (settingValue("lock.pin_set") ? "remote.settings.change" : "remote.settings.set_pin"),
        run: () => startPinFlow(settingValue("lock.pin_set") ? "change" : "set"),
      },
      {
        label: label("lock.remove_pin"),
        needs: "lock.pin_set",
        button: "remote.settings.remove",
        run: () => startPinFlow("remove"),
      },
      {
        label: label("lock.lock_now"),
        hint: hint("lock.lock_now"),
        needs: "lock.pin_set",
        button: "remote.settings.lock",
        run: () => {
          if (send({ type: "lock" })) closeSettings();
        },
      },
      { key: "lock.on_sleep", label: label("lock.on_sleep"), needs: "lock.pin_set" },
    ],
  },
  {
    title: "section.gamepad",
    items: [
      { key: "gamepad.enabled", label: label("gamepad.enabled"), hint: hint("gamepad.enabled") },
      {
        key: "gamepad.guide_closes_app",
        label: label("gamepad.guide_closes_app"),
        hint: hint("gamepad.guide_closes_app"),
        needs: "gamepad.enabled",
      },
    ],
  },
  {
    title: "section.casting",
    items: [{ key: "cast.airplay", label: label("cast.airplay"), hint: hint("cast.airplay") }],
  },
  {
    title: "section.sleep",
    items: [
      { key: "sleep.enabled", label: label("sleep.enabled"), hint: hint("sleep.enabled") },
      {
        key: "sleep.after_minutes",
        label: label("sleep.after_minutes"),
        needs: "sleep.enabled",
        options: [[15, minutes(15)], [30, minutes(30)], [60, hours(1)], [120, hours(2)], [240, hours(4)]],
      },
      { key: "sleep.close_apps", label: label("sleep.close_apps"), hint: hint("sleep.close_apps") },
      { action: "sleep", label: label("sleep.now"), button: "remote.settings.sleep", done: "remote.settings.going_to_sleep" },
    ],
  },
  {
    title: "section.wallpaper",
    custom: () => [
      wallpaperCount(),
      ...[
        { key: "wallpaper.home", label: label("wallpaper.home"), hint: hint("wallpaper.home") },
        {
          key: "wallpaper.change_minutes",
          label: label("wallpaper.change_minutes"),
          needs: "wallpaper.home",
          options: [[5, minutes(5)], [15, minutes(15)], [30, minutes(30)], [60, hours(1)], [240, hours(4)], [1440, unit("unit.days", 1)]],
        },
        {
          key: "wallpaper.slide_seconds",
          label: label("wallpaper.slide_seconds"),
          options: [[10, seconds(10)], [30, seconds(30)], [60, minutes(1)], [300, minutes(5)]],
        },
        { key: "wallpaper.shuffle", label: label("wallpaper.shuffle") },
      ].map(settingRow),
    ],
  },
  {
    title: "section.keyboard",
    items: [
      { key: "keyboard.enabled", label: label("keyboard.enabled"), hint: hint("keyboard.enabled") },
      { key: "keyboard.hold_ok", label: label("keyboard.hold_ok"), hint: hint("keyboard.hold_ok"), needs: "keyboard.enabled" },
      {
        key: "keyboard.hold_guide",
        label: label("keyboard.hold_guide"),
        hint: hint("keyboard.hold_guide"),
        needs: "keyboard.enabled",
      },
    ],
  },
  {
    title: "section.cec",
    items: [
      { key: "cec.enabled", label: label("cec.enabled"), hint: hint("cec.enabled") },
      { key: "cec.power_tv", label: label("cec.power_tv"), hint: hint("cec.power_tv"), needs: "cec.enabled" },
      { key: "cec.follow_tv", label: label("cec.follow_tv"), hint: hint("cec.follow_tv"), needs: "cec.enabled" },
      { key: "cec.remote_keys", label: label("cec.remote_keys"), hint: hint("cec.remote_keys"), needs: "cec.enabled" },
    ],
  },
  {
    title: "section.overlay",
    items: [
      { live: "overlay", label: label("overlay.live"), hint: hint("overlay.live") },
      { key: "overlay.enabled", label: label("overlay.enabled") },
      {
        key: "overlay.position",
        label: label("overlay.position"),
        options: [["top", "option.overlay.position.top"], ["bottom", "option.overlay.position.bottom"]],
      },
    ],
  },
];

// How many pictures the config lists; the list itself is edited in the config file.
function wallpaperCount() {
  const count = settings.wallpaper ? settings.wallpaper.count : 0;
  const note = document.createElement("p");
  note.className = "setting-hint";
  note.textContent = count
    ? t("remote.wallpaper.count", { count })
    : t("remote.wallpaper.none");
  return note;
}

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
  if (key === "locale.language") setLanguage(value);
  renderSettings();
}

function renderSettings() {
  const body = $("#settings-body");
  if (!body || $("#settings").hidden) return;
  body.replaceChildren();
  if (!settings) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = t("remote.settings.waiting");
    body.append(empty);
    return;
  }
  for (const group of SETTINGS) {
    const section = document.createElement("section");
    section.className = "settings-group";
    const title = document.createElement("h3");
    title.textContent = say(group.title);
    section.append(title);
    for (const row of group.custom ? group.custom() : group.items.map(settingRow)) section.append(row);
    body.append(section);
  }
  body.append(wifiGroup(), bluetoothGroup(), networkGroup());

  const about = document.createElement("p");
  about.className = "settings-about";
  about.textContent = t("remote.settings.about", { version: settings.version, path: settings.config_path });
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
  label.textContent = say(item.label);
  text.append(label);
  if (item.hint) {
    const hint = document.createElement("div");
    hint.className = "setting-hint";
    hint.textContent = say(item.hint);
    text.append(hint);
  }
  row.append(text);

  const value = item.live === "overlay" ? Boolean(state && state.overlay) : settingValue(item.key);
  if ((item.action || item.run) && item.button !== null) {
    const button = document.createElement("button");
    button.className = "chip";
    button.textContent = say(typeof item.button === "function" ? item.button() : item.button);
    button.disabled = Boolean(off);
    button.addEventListener("click", () => {
      haptic();
      if (item.run) return item.run();
      if (send({ type: item.action })) {
        closeSettings();
        toast(say(item.done));
      }
    });
    row.append(button);
  } else if (item.options) {
    row.classList.add("stacked");
    const chips = document.createElement("div");
    chips.className = "chips";
    const options = [...item.options];
    // A value set in the config file that is not one of the presets.
    if (value !== undefined && !options.some(([v]) => sameValue(v, value))) options.push([value, { text: String(value) }]);
    for (const [optionValue, optionLabel] of options) {
      const chip = document.createElement("button");
      chip.className = "chip";
      chip.textContent = say(optionLabel);
      chip.disabled = Boolean(off);
      chip.setAttribute("aria-pressed", String(sameValue(optionValue, value)));
      chip.addEventListener("click", () => {
        if (!sameValue(optionValue, value)) changeSetting(item.key, optionValue);
      });
      chips.append(chip);
    }
    row.append(chips);
  } else if (item.key || item.live) {
    const toggle = document.createElement("button");
    toggle.className = "toggle";
    toggle.setAttribute("role", "switch");
    toggle.setAttribute("aria-checked", String(Boolean(value)));
    toggle.setAttribute("aria-label", say(item.label));
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

// ---- theme picker ----

const COLOR_KEYS = [
  ["accent", "remote.theme.accent"],
  ["accent_2", "remote.theme.pressed"],
  ["background", "remote.theme.background"],
];

function themeRows() {
  const theme = settings.theme || { preset: "silver" };
  const own = COLOR_KEYS.filter(([key]) => theme[key]);

  const presets = document.createElement("div");
  presets.className = "setting stacked";
  const text = document.createElement("div");
  text.className = "setting-text";
  text.innerHTML = '<div class="setting-label"></div><div class="setting-hint"></div>';
  text.querySelector(".setting-label").textContent = t("setting.theme.preset.label");
  text.querySelector(".setting-hint").textContent = t("setting.theme.preset.hint");
  const chips = document.createElement("div");
  chips.className = "chips";
  for (const preset of settings.presets || []) {
    const chip = document.createElement("button");
    chip.className = "chip theme-chip";
    chip.setAttribute("aria-pressed", String(preset.name === theme.preset && own.length === 0));
    const swatch = document.createElement("span");
    swatch.className = "swatch";
    swatch.style.background = preset.accent;
    swatch.style.boxShadow = `0 0 0 3px ${preset.background}`;
    chip.append(swatch, t(`option.theme.preset.${preset.name}`));
    chip.addEventListener("click", () => {
      // A preset brings its own colors, so drop the custom ones.
      if (preset.name !== theme.preset) changeSetting("theme.preset", preset.name);
      for (const [key] of own) changeSetting(`theme.${key}`, null);
    });
    chips.append(chip);
  }
  presets.append(text, chips);

  const colors = document.createElement("div");
  colors.className = "setting stacked";
  const label = document.createElement("div");
  label.className = "setting-label";
  label.textContent = t("remote.theme.your_colors");
  const pickers = document.createElement("div");
  pickers.className = "color-pickers";
  for (const [key, name] of COLOR_KEYS) {
    const field = document.createElement("label");
    field.className = "color-field" + (theme[key] ? " own" : "");
    const input = document.createElement("input");
    input.type = "color";
    input.value = (palette && palette[key]) || "#c9ced4";
    input.addEventListener("change", () => changeSetting(`theme.${key}`, input.value));
    const caption = document.createElement("span");
    caption.textContent = t(name);
    field.append(input, caption);
    pickers.append(field);
  }
  colors.append(label, pickers);
  if (own.length) {
    const reset = document.createElement("button");
    reset.className = "chip";
    reset.textContent = t("remote.theme.back_to_finish");
    reset.addEventListener("click", () => {
      for (const [key] of own) changeSetting(`theme.${key}`, null);
    });
    colors.append(reset);
  }
  return [presets, colors];
}

// ---- network overview (read-only) ----

let exposures = null;

async function loadNetwork() {
  try {
    const res = await fetch("/api/v1/network", { headers: { Authorization: `Bearer ${token}` } });
    exposures = res.ok ? await res.json() : [];
  } catch {
    exposures = [];
  }
  renderSettings();
}

function networkGroup() {
  const section = document.createElement("section");
  section.className = "settings-group";
  const title = document.createElement("h3");
  title.textContent = t("section.network");
  section.append(title);
  const intro = settingRow({
    label: "remote.network.title",
    hint: "remote.network.hint",
  });
  section.append(intro);
  for (const e of exposures || []) {
    const row = document.createElement("div");
    row.className = "setting";
    const text = document.createElement("div");
    text.className = "setting-text";
    const label = document.createElement("div");
    label.className = "setting-label";
    label.textContent = worded(e.service_label, e.service);
    const hint = document.createElement("div");
    hint.className = "setting-hint";
    const address = worded(e.address_label, e.address);
    hint.textContent = `${e.protocol.toUpperCase()} ${e.port} · ${address}`;
    text.append(label, hint);
    row.append(text);
    section.append(row);
  }
  if (!exposures) {
    const row = settingRow({ label: "common.checking" });
    section.append(row);
  }
  return section;
}

// ---- Wi-Fi and Bluetooth ----

let wifi = null; // networks, or { error }
let bluetooth = null;
let wirelessBusy = ""; // what is in progress, shown on its button

async function api(path) {
  const res = await fetch(path, { headers: { Authorization: `Bearer ${token}` } });
  const body = await res.json();
  if (!res.ok) throw new Error(body.error || res.statusText);
  return body;
}

async function loadWifi(rescan = false) {
  try {
    wifi = await api(`/api/v1/wifi${rescan ? "?rescan=true" : ""}`);
  } catch (err) {
    wifi = { error: err.message };
  }
  renderSettings();
}

async function loadBluetooth() {
  try {
    bluetooth = await api("/api/v1/bluetooth");
  } catch {
    bluetooth = { available: false, devices: [] };
  }
  renderSettings();
}

/** Runs a command that takes a while, with its button showing progress, then reloads. */
async function wirelessCommand(busy, msg, done, reload) {
  wirelessBusy = busy;
  renderSettings();
  const ok = await sendAndWait(msg);
  wirelessBusy = "";
  if (ok && done) toast(t(done.text, done.vars), "ok");
  await reload();
}

// Sends a command over REST so the remote can wait for its result.
async function sendAndWait(msg) {
  try {
    const res = await fetch("/api/v1/command", {
      method: "POST",
      headers: { Authorization: `Bearer ${token}` },
      body: JSON.stringify(msg),
    });
    if (res.ok) return true;
    const body = await res.json().catch(() => ({}));
    toast(body.key ? t(body.key, body.args || {}) : body.error || t("common.something_went_wrong"), "error");
  } catch {
    toast(t("remote.status.not_connected"), "error");
  }
  return false;
}

function infoRow(label, hint) {
  return settingRow({ label, hint });
}

function wifiGroup() {
  const section = document.createElement("section");
  section.className = "settings-group";
  const title = document.createElement("h3");
  title.textContent = t("section.wifi");
  section.append(title);
  section.append(
    settingRow({
      label: "remote.wifi.nearby",
      hint: "remote.wifi.nearby_hint",
      button: wirelessBusy === "wifi-scan" ? "common.searching" : "remote.wifi.search_again",
      run: () => {
        wirelessBusy = "wifi-scan";
        renderSettings();
        loadWifi(true).then(() => (wirelessBusy = ""));
      },
    }),
  );
  if (!wifi) {
    section.append(infoRow("common.checking"));
  } else if (wifi.error) {
    section.append(infoRow("remote.wifi.unavailable", wifi.error));
  } else {
    for (const network of wifi.slice(0, 12)) {
      const busy = wirelessBusy === `wifi:${network.ssid}`;
      section.append(
        settingRow({
          label: network.ssid,
          hint: `${network.connected ? t("common.connected") + " · " : ""}${t("remote.wifi.signal")} ${percent(network.signal)}${network.secure ? " · 🔒" : ""}`,
          button: network.connected ? null : busy ? "remote.wifi.joining" : "remote.wifi.join",
          run: network.connected
            ? null
            : () => {
                let password = null;
                if (network.secure) {
                  password = window.prompt(t("remote.wifi.password", { name: network.ssid }));
                  if (password === null) return;
                }
                wirelessCommand(
                  `wifi:${network.ssid}`,
                  { type: "wifi_connect", ssid: network.ssid, password: password || null },
                  { text: "remote.wifi.joined", vars: { name: network.ssid } },
                  () => loadWifi(),
                );
              },
        }),
      );
    }
  }
  return section;
}

const BT_ICONS = { "audio-headset": "🎧", "audio-headphones": "🎧", "audio-card": "🔊", "input-gaming": "🎮", "input-keyboard": "⌨️", "input-mouse": "🖱️", phone: "📱" };

function bluetoothGroup() {
  const section = document.createElement("section");
  section.className = "settings-group";
  const title = document.createElement("h3");
  title.textContent = t("section.bluetooth");
  section.append(title);
  if (!bluetooth) {
    section.append(infoRow("common.checking"));
    return section;
  }
  if (!bluetooth.available) {
    section.append(infoRow("remote.bluetooth.unavailable", "remote.bluetooth.no_adapter"));
    return section;
  }
  const power = settingRow({ label: "section.bluetooth" });
  const toggle = document.createElement("button");
  toggle.className = "toggle";
  toggle.setAttribute("role", "switch");
  toggle.setAttribute("aria-checked", String(bluetooth.powered));
  toggle.addEventListener("click", () =>
    wirelessCommand("bt-power", { type: "bluetooth", action: bluetooth.powered ? "power_off" : "power_on" }, null, loadBluetooth),
  );
  power.append(toggle);
  section.append(power);
  if (!bluetooth.powered) return section;

  section.append(
    settingRow({
      label: "remote.bluetooth.add",
      hint: "remote.bluetooth.add_hint",
      button: wirelessBusy === "bt-scan" ? "common.searching" : "remote.bluetooth.search",
      run: () => wirelessCommand("bt-scan", { type: "bluetooth", action: "scan" }, null, loadBluetooth),
    }),
  );
  for (const device of bluetooth.devices.slice(0, 15)) {
    const busy = wirelessBusy === `bt:${device.address}`;
    const act = (action, done) =>
      wirelessCommand(`bt:${device.address}`, { type: "bluetooth", action, address: device.address }, done, loadBluetooth);
    const icon = BT_ICONS[device.icon] ? `${BT_ICONS[device.icon]} ` : "";
    const row = settingRow({
      label: icon + device.name,
      hint: device.connected ? "common.connected" : device.paired ? "remote.bluetooth.paired" : "remote.bluetooth.nearby",
      button: busy ? "…" : device.connected ? "remote.bluetooth.disconnect" : device.paired ? "remote.pair.connect" : "remote.bluetooth.pair",
      run: () =>
        device.connected
          ? act("disconnect")
          : device.paired
            ? act("connect", { text: "remote.bluetooth.connected_to", vars: { name: device.name } })
            : act("pair", { text: "remote.bluetooth.paired_with", vars: { name: device.name } }),
    });
    if (device.paired && !busy) {
      const forget = document.createElement("button");
      forget.className = "chip";
      forget.textContent = t("remote.bluetooth.forget");
      forget.addEventListener("click", () => act("remove"));
      row.append(forget);
    }
    section.append(row);
  }
  return section;
}

// ---- PIN keypad: unlocking, and setting / changing / removing the PIN ----

let pinFlow = null; // { mode, step, current, first }
let pinDigits = "";

const PIN_STEPS = {
  unlock: ["unlock"],
  set: ["new", "confirm"],
  change: ["current", "new", "confirm"],
  remove: ["current"],
};
const PIN_HINTS = {
  unlock: "remote.pin.enter",
  current: "remote.pin.enter_current",
  new: "remote.pin.choose",
  confirm: "remote.pin.again",
};

function buildPad() {
  const pad = $("#pin-pad");
  for (const key of ["1", "2", "3", "4", "5", "6", "7", "8", "9", "del", "0", "ok"]) {
    const button = document.createElement("button");
    button.className = "pin-key" + (key === "ok" ? " primary" : "");
    button.textContent = key === "del" ? "⌫" : key === "ok" ? t("remote.dpad.ok") : key;
    button.setAttribute("aria-label", key === "del" ? t("remote.key.delete") : key === "ok" ? t("remote.pin.confirm") : key);
    button.addEventListener("click", () => pinKey(key));
    pad.append(button);
  }
}
buildPad();

function startPinFlow(mode) {
  pinFlow = { mode, step: 0, current: null, first: null };
  pinDigits = "";
  if (mode !== "unlock") closeSettings();
  renderPin();
}

function pinStep() {
  return pinFlow && PIN_STEPS[pinFlow.mode][pinFlow.step];
}

function renderPin() {
  const show = Boolean(pinFlow);
  $("#pin").hidden = !show;
  if (!show) return;
  const locked = pinFlow.mode === "unlock";
  $("#pin-title").textContent = t(locked ? "remote.pin.locked" : pinFlow.mode === "remove" ? "setting.lock.remove_pin.label" : "remote.pin.title");
  $("#pin-hint").textContent = t(PIN_HINTS[pinStep()]);
  $("#pin-cancel").hidden = locked;
  const dots = $("#pin-dots");
  dots.replaceChildren();
  for (let i = 0; i < Math.max(4, pinDigits.length); i++) {
    const dot = document.createElement("span");
    dot.className = "pin-dot" + (i < pinDigits.length ? " filled" : "");
    dots.append(dot);
  }
}

function pinKey(key) {
  haptic();
  if (key === "del") pinDigits = pinDigits.slice(0, -1);
  else if (key === "ok") return submitPin();
  else if (pinDigits.length < 8) pinDigits += key;
  renderPin();
}

function submitPin() {
  if (pinDigits.length < 4) return toast(t("remote.pin.too_short"), "error");
  const step = pinStep();
  const digits = pinDigits;
  pinDigits = "";
  if (step === "unlock") {
    send({ type: "unlock", pin: digits });
  } else if (step === "current") {
    pinFlow.current = digits;
    if (pinFlow.mode === "remove") {
      send({ type: "set_pin", current: digits });
      pinFlow = null;
      toast(t("remote.pin.removed"), "ok");
    } else {
      pinFlow.step++;
    }
  } else if (step === "new") {
    pinFlow.first = digits;
    pinFlow.step++;
  } else if (step === "confirm") {
    if (digits !== pinFlow.first) {
      toast(t("remote.pin.mismatch"), "error");
      pinFlow.step = PIN_STEPS[pinFlow.mode].indexOf("new");
    } else {
      send({ type: "set_pin", current: pinFlow.current, pin: digits });
      pinFlow = null;
      toast(t("remote.pin.saved"), "ok");
    }
  }
  renderPin();
}

$("#pin-cancel").addEventListener("click", () => {
  pinFlow = null;
  renderPin();
});

// The keypad follows the TV: it opens when the TV locks and closes when it unlocks.
function syncLock() {
  const locked = Boolean(state && state.locked);
  if (locked && (!pinFlow || pinFlow.mode !== "unlock")) startPinFlow("unlock");
  if (!locked && pinFlow && pinFlow.mode === "unlock") {
    pinFlow = null;
    renderPin();
    toast(t("remote.pin.unlocked"), "ok");
  }
}

function openSettings() {
  haptic();
  $("#settings").hidden = false;
  renderSettings();
  loadNetwork();
  loadWifi();
  loadBluetooth();
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
  $("#vol-value").textContent = percent(slider.value);
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
    toast(t("remote.text.sent"), "ok");
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
  if (!forwarding || event.target === $("#text") || MODIFIERS.has(event.code) || !$("#sheet").hidden || !$("#settings").hidden || !$("#timer").hidden) return;
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

applyLanguage();
// Placeholders until tvd answers; the script sets these two itself from then on.
$("#conn").textContent = t("remote.status.connecting");
$("#output-name").textContent = t("remote.output.label");
start();
