"use strict";

// rustv web remote. Talks to tvd over one WebSocket; see protocol.rs for the messages.

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => document.querySelectorAll(sel);
const TOKEN_KEY = "rustv.token";
const TAB_KEY = "rustv.tab";

let token = readToken();
let ws = null;
let retry = 0;
let apps = [];
let state = null;
let audio = { max_volume: 1.5, step: 0.05, outputs: [] };

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
    $("#dot").classList.add("on");
  };
  ws.onmessage = (event) => handle(JSON.parse(event.data));
  ws.onclose = () => {
    $("#dot").classList.remove("on");
    setTimeout(connect, Math.min(5000, 250 * 2 ** retry++));
  };
}

function send(msg) {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

function handle(msg) {
  if (msg.type === "apps") {
    apps = msg.apps;
    renderDock();
  } else if (msg.type === "audio") {
    audio = msg;
    renderAudio();
  } else if (msg.type === "state") {
    state = msg;
    renderState();
  } else if (msg.type === "error") {
    toast(msg.message);
  }
}

function showPair(message) {
  $("#app").hidden = true;
  $("#pair").hidden = false;
  if (message) toast(message, $("#pair"));
}

$("#pair-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const value = $("#pair-token").value.trim();
  if (!value) return;
  token = value;
  localStorage.setItem(TOKEN_KEY, token);
  start();
});

// ---- rendering ----

function renderState() {
  const running = state.running;
  $("#now").textContent = running ? running.name : "Home";
  $("#stop").hidden = !running;
  renderVolume();
  renderDock();
}

function renderVolume() {
  const volume = state && state.volume;
  const label = !volume ? "–" : volume.muted ? "Muted" : `${Math.round(volume.level * 100)}%`;
  for (const el of $$(".js-vol-label")) el.textContent = label;
  $("#mute").textContent = volume && volume.muted ? "Unmute" : "Mute";
  $("#mute").classList.toggle("active", Boolean(volume && volume.muted));
  if (volume && !sliderBusy()) $("#vol-slider").value = Math.round(volume.level * 100);
}

function renderAudio() {
  const max = Math.round(audio.max_volume * 100);
  $("#vol-slider").max = max;
  $("#vol-max").textContent = `${max}%`;
  const list = $("#outputs");
  list.replaceChildren();
  if (!audio.outputs.length) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = "No outputs found.";
    list.append(empty);
  }
  for (const output of audio.outputs) {
    const item = document.createElement("button");
    item.className = "output" + (output.default ? " selected" : "");
    const radio = document.createElement("span");
    radio.className = "radio";
    const name = document.createElement("span");
    name.textContent = output.name;
    item.append(radio, name);
    item.addEventListener("click", () => {
      if (output.default) return;
      haptic();
      send({ type: "audio_output", id: output.id });
    });
    list.append(item);
  }
  renderVolume();
}

function renderDock() {
  const dock = $("#dock");
  dock.replaceChildren();
  const runningId = state && state.running ? state.running.id : null;
  for (const app of apps) {
    const item = document.createElement("button");
    item.className = "dock-item" + (app.id === runningId ? " running" : "");
    item.title = app.description || app.name;
    item.append(appIcon(app));
    const name = document.createElement("span");
    name.className = "dock-name";
    name.textContent = app.name;
    item.append(name);
    item.addEventListener("click", () => {
      haptic();
      if (app.id === runningId) return toast(`${app.name} is already on the TV`);
      send({ type: "launch", id: app.id });
      item.classList.add("opening");
    });
    dock.append(item);
  }
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
function toast(text, parent = $("#app")) {
  const el = $("#toast");
  parent.append(el);
  el.textContent = text;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), 3000);
}

function haptic() {
  if (navigator.vibrate) navigator.vibrate(8);
}

// ---- buttons ----

function pressKey(code) {
  haptic();
  send({ type: "key", code });
}

for (const button of $$("[data-key]")) {
  const code = button.dataset.key;
  if (!("repeat" in button.dataset)) {
    button.addEventListener("click", () => pressKey(code));
    continue;
  }
  // Hold to repeat, like a real remote.
  let delay = null;
  let interval = null;
  const release = () => {
    clearTimeout(delay);
    clearInterval(interval);
  };
  button.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    pressKey(code);
    delay = setTimeout(() => (interval = setInterval(() => send({ type: "key", code }), 110)), 400);
  });
  for (const ev of ["pointerup", "pointerleave", "pointercancel"]) button.addEventListener(ev, release);
}

for (const button of $$("[data-vol]")) {
  button.addEventListener("click", () => {
    haptic();
    send({ type: "volume", action: button.dataset.vol });
  });
}

for (const button of $$("[data-btn]")) {
  button.addEventListener("click", () => {
    haptic();
    send({ type: "button", button: button.dataset.btn });
  });
}

$("#home").addEventListener("click", () => {
  haptic();
  send({ type: "stop" });
});
$("#stop").addEventListener("click", () => send({ type: "stop" }));
$("#mute").addEventListener("click", () => {
  haptic();
  send({ type: "volume", action: "mute" });
});

// ---- volume slider ----

const slider = $("#vol-slider");
let sliderTouched = 0;
let sliderTimer = null;

function sliderBusy() {
  return Date.now() - sliderTouched < 800;
}

slider.addEventListener("input", () => {
  sliderTouched = Date.now();
  for (const el of $$(".js-vol-label")) el.textContent = `${slider.value}%`;
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
  send({ type: "text", text: input.value });
  input.value = "";
});

const MODIFIERS = new Set(["ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight"]);

document.addEventListener("keydown", (event) => {
  const forwarding = $("#capture").checked || document.pointerLockElement === pad;
  if (!forwarding || event.target === $("#text") || MODIFIERS.has(event.code)) return;
  if (event.code === "Escape" && document.pointerLockElement === pad) return; // browser releases the lock
  event.preventDefault();
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

for (const tab of $$("[data-tab]")) tab.addEventListener("click", () => showTab(tab.dataset.tab));
showTab(localStorage.getItem(TAB_KEY) || "remote");

start();
