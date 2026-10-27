"use strict";

// rustv web remote. Talks to tvd over one WebSocket; see protocol.rs for the messages.

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => document.querySelectorAll(sel);
// ---- language ----

// English is the source text; other languages map English strings to their own.
// `{name}` marks a value filled in by `t()`. Numbers in messages from tvd are
// matched as `{n}`, so "try again in 42 s" finds "try again in {n} s".
const TRANSLATIONS = {
  tr: {
    // pairing
    "rustv remote": "rustv kumanda",
    Run: "TV kutusunda",
    "on the TV box and open the link it prints, or paste the token here.":
      "komutunu çalıştırıp verdiği bağlantıyı aç ya da anahtarı buraya yapıştır.",
    "Pairing token": "Eşleştirme anahtarı",
    Connect: "Bağlan",
    "That token was rejected.": "Bu anahtar kabul edilmedi.",
    // header and remote
    "Connecting…": "Bağlanıyor…",
    Connected: "Bağlı",
    "Reconnecting…": "Yeniden bağlanıyor…",
    "Not connected to the TV": "TV'ye bağlı değil",
    Home: "Ana ekran",
    Settings: "Ayarlar",
    "Close app": "Uygulamayı kapat",
    Asleep: "Uykuda",
    "Any button wakes the TV": "Uyandırmak için bir tuşa bas",
    Up: "Yukarı",
    Down: "Aşağı",
    Left: "Sol",
    Right: "Sağ",
    Back: "Geri",
    Play: "Oynat",
    Mute: "Sesi kapat",
    Unmute: "Sesi aç",
    Muted: "Sessiz",
    Volume: "Ses",
    "Volume down": "Sesi azalt",
    "Volume up": "Sesi artır",
    Output: "Çıkış",
    "No output": "Çıkış yok",
    "Play sound on": "Sesi şuradan çal",
    "No audio outputs found.": "Ses çıkışı bulunamadı.",
    "Playing here": "Burada çalıyor",
    "Speakers and headphones": "Hoparlör ve kulaklık",
    "Sound now plays on {name}": "Ses artık {name} üzerinden çalıyor",
    "Could not switch to {name}": "{name} seçilemedi",
    Close: "Kapat",
    // dock and apps
    Apps: "Uygulamalar",
    "Opening…": "Açılıyor…",
    Done: "Bitti",
    "Drag to move {name}": "{name} uygulamasını taşımak için sürükle",
    "Drag apps to reorder them, then tap Done": "Uygulamaları sürükleyip sırala, sonra Bitti'ye dokun",
    "Order saved on the TV": "Sıra TV'ye kaydedildi",
    "{name} is open on the TV": "{name} TV'de açıldı",
    "{name} is already on the TV": "{name} zaten TV'de açık",
    "{name} did not open": "{name} açılmadı",
    "Closed {name}": "{name} kapatıldı",
    // touchpad and keyboard
    "Drag to move · tap to click": "Sürükle: imleci taşı · dokun: tıkla",
    "Two fingers to scroll · two-finger tap for right click": "İki parmak: kaydır · iki parmakla dokun: sağ tık",
    "Click to control the TV pointer with your mouse": "TV imlecini fareyle kullanmak için tıkla",
    "Press Esc to let go": "Bırakmak için Esc'ye bas",
    "Left click": "Sol tık",
    Middle: "Orta",
    "Right click": "Sağ tık",
    "Type on the TV": "TV'ye yaz",
    "Search, URL, password…": "Arama, adres, şifre…",
    Send: "Gönder",
    Delete: "Sil",
    "Send everything I type on this computer to the TV": "Bu bilgisayarda yazdığım her şeyi TV'ye gönder",
    "Sent to the TV": "TV'ye gönderildi",
    Views: "Görünümler",
    Remote: "Kumanda",
    Touchpad: "Dokunmatik",
    Keyboard: "Klavye",
    // settings
    "Waiting for the TV…": "TV bekleniyor…",
    "rustv {version} · saved to {path}": "rustv {version} · kayıt yeri: {path}",
    "Home screen": "Ana ekran",
    Greeting: "Karşılama",
    Clock: "Saat",
    Date: "Tarih",
    "Volume bar when the volume changes": "Ses değişince ses çubuğu",
    Sounds: "Sesler",
    "Soft clicks when moving between apps and opening one.": "Uygulamalar arasında gezinirken ve açarken yumuşak tıklamalar.",
    "Sound level": "Ses düzeyi",
    Quiet: "Kısık",
    Medium: "Orta",
    Loud: "Yüksek",
    "Pairing code": "Eşleştirme kodu",
    "QR code on the TV until a phone pairs, and when you ask for it.":
      "Bir telefon eşleşene kadar ve sen istediğinde TV'de QR kod.",
    "Pair another phone or computer": "Başka bir telefon ya da bilgisayar eşleştir",
    "Show code": "Kodu göster",
    "The pairing code is on the TV for 2 minutes": "Eşleştirme kodu 2 dakika boyunca TV'de",
    Screensaver: "Ekran koruyucu",
    "Start after": "Başlama süresi",
    "1 min": "1 dk",
    "2 min": "2 dk",
    "5 min": "5 dk",
    "10 min": "10 dk",
    "15 min": "15 dk",
    "30 min": "30 dk",
    "1 h": "1 sa",
    "2 h": "2 sa",
    "4 h": "4 sa",
    Style: "Stil",
    Mix: "Karışık",
    Orbits: "Yörünge",
    "Show the time": "Saati göster",
    "Show the date": "Tarihi göster",
    Sound: "Ses",
    "Volume limit": "Ses sınırı",
    "Above 100% the sound is boosted in software and can distort.":
      "%100'ün üstünde ses yazılımla yükseltilir ve bozulabilir.",
    "Volume buttons change by": "Ses tuşlarının adımı",
    "Language and format": "Dil ve biçim",
    Language: "Dil",
    "Time format": "Saat biçimi",
    "24-hour": "24 saat",
    "12-hour": "12 saat",
    Temperature: "Sıcaklık",
    "Used by the debug overlay.": "Hata ayıklama katmanında kullanılır.",
    "Lock and PIN": "Kilit ve PIN",
    "Locking needs a PIN. It is kept on the TV box, hashed, never in the config.":
      "Kilit için PIN gerekir. PIN, TV kutusunda geri çözülemez biçimde saklanır; ayar dosyasına yazılmaz.",
    Change: "Değiştir",
    "Set PIN": "PIN belirle",
    "Remove the PIN": "PIN'i kaldır",
    Remove: "Kaldır",
    "Lock the TV now": "TV'yi şimdi kilitle",
    "Closes the open app; nothing works until the PIN is entered.":
      "Açık uygulamayı kapatır; PIN girilene kadar hiçbir şey çalışmaz.",
    Lock: "Kilitle",
    "Lock when the TV goes to sleep": "TV uykuya geçince kilitle",
    "Screen casting": "Ekran yansıtma",
    "Mirror an iPhone, iPad or Mac, or play videos and music from them. Needs uxplay on the box.":
      "iPhone, iPad ya da Mac ekranını yansıt veya onlardan video ve müzik çal. Kutuda uxplay gerekir.",
    "Sleep mode": "Uyku modu",
    "Sleep when unused": "Kullanılmayınca uyu",
    "Screen off after no remote input and no sound. Any button wakes it.":
      "Kumandaya dokunulmaz ve ses çalmazsa ekran kapanır. Herhangi bir tuş uyandırır.",
    After: "Süre",
    "Close the open app": "Açık uygulamayı kapat",
    "Frees memory and CPU.": "Bellek ve işlemciyi boşaltır.",
    "Put the TV to sleep now": "TV'yi şimdi uyut",
    Sleep: "Uyut",
    "TV is going to sleep. Any button wakes it.": "TV uykuya geçiyor. Herhangi bir tuş uyandırır.",
    "Debug overlay": "Hata ayıklama katmanı",
    "Show on the TV now": "Şimdi TV'de göster",
    "CPU, GPU, RAM, network and more, over everything.": "CPU, GPU, RAM, ağ ve daha fazlası, her şeyin üstünde.",
    "Show at startup": "Açılışta göster",
    Position: "Konum",
    Top: "Üst",
    Bottom: "Alt",
    // media and sleep timer
    Previous: "Önceki",
    Next: "Sonraki",
    "Back 10 seconds": "10 saniye geri",
    "Forward 10 seconds": "10 saniye ileri",
    "Sleep timer": "Uyku zamanlayıcısı",
    // display
    Display: "Ekran",
    Resolution: "Çözünürlük",
    "Refresh rate": "Yenileme hızı",
    "A new mode goes back by itself after 15 seconds unless you keep it.":
      "Yeni bir mod, korumazsan 15 saniye sonra kendiliğinden geri döner.",
    "The TV's resolutions show here once sway reports them.": "TV'nin çözünürlükleri sway bildirince burada görünür.",
    "Keep this resolution?": "Bu çözünürlük kalsın mı?",
    "{mode}. Going back in {seconds} s.": "{mode}. {seconds} sn sonra geri dönülecek.",
    Keep: "Koru",
    "Go back": "Geri dön",
    "Sleeps in {minutes} min": "{minutes} dk sonra uyuyacak",
    "Turn the TV off after a while, like on a TV remote.": "TV'yi bir süre sonra kapat, TV kumandasındaki gibi.",
    "The TV sleeps in {minutes} min.": "TV {minutes} dk sonra uyuyacak.",
    "{n} min": "{n} dk",
    "{h} h": "{h} sa",
    "Turn the timer off": "Zamanlayıcıyı kapat",
    "Sleep timer off": "Uyku zamanlayıcısı kapatıldı",
    "Remember the volume for each app": "Her uygulamanın sesini hatırla",
    "A loud game and a quiet film each come back at their own level.":
      "Yüksek sesli bir oyun ve kısık bir film, her biri kendi seviyesinde açılır.",
    // Wi-Fi and Bluetooth
    "Wi-Fi": "Wi-Fi",
    "Networks nearby": "Yakındaki ağlar",
    "Switching networks drops the remote for a moment; with a wrong password the TV returns to the old one.":
      "Ağ değiştirince kumanda bir an kopar; şifre yanlışsa TV eski ağa döner.",
    "Search again": "Yeniden ara",
    "Searching…": "Aranıyor…",
    "Wi-Fi is not available": "Wi-Fi kullanılamıyor",
    Connected: "Bağlı",
    Signal: "Sinyal",
    Join: "Bağlan",
    "Joining…": "Bağlanıyor…",
    "Password for {name} (leave empty if the TV knows it)": "{name} şifresi (TV biliyorsa boş bırak)",
    "Joined {name}": "{name} ağına bağlanıldı",
    "Wrong Wi-Fi password": "Wi-Fi şifresi yanlış",
    Bluetooth: "Bluetooth",
    "Bluetooth is not available": "Bluetooth kullanılamıyor",
    "No adapter, or bluetoothd is not running.": "Bluetooth donanımı yok ya da bluetoothd çalışmıyor.",
    "Add a device": "Cihaz ekle",
    "Put headphones or a gamepad in pairing mode, then search.":
      "Kulaklığı ya da oyun kolunu eşleştirme moduna al, sonra ara.",
    Search: "Ara",
    Paired: "Eşleşmiş",
    Nearby: "Yakında",
    Disconnect: "Bağlantıyı kes",
    Pair: "Eşleştir",
    Forget: "Unut",
    "Connected to {name}": "{name} bağlandı",
    "Paired with {name}": "{name} eşleştirildi",
    "Something went wrong": "Bir şeyler ters gitti",
    // wallpaper
    Wallpaper: "Duvar kağıdı",
    "My pictures": "Resimlerim",
    "Behind the home screen": "Ana ekranın arkasında",
    "Your pictures instead of the theme's background.": "Temanın arka planı yerine kendi resimlerin.",
    "Change every": "Değiştirme sıklığı",
    "Screensaver: each picture for": "Ekran koruyucu: her resim",
    "Random order": "Rastgele sıra",
    "1 day": "1 gün",
    "10 s": "10 sn",
    "30 s": "30 sn",
    "Pictures in [wallpaper] images: {count}": "[wallpaper] images içindeki resimler: {count}",
    "Add picture files or folders to [wallpaper] images in the config file.":
      "Yapılandırma dosyasında [wallpaper] images listesine resim dosyaları ya da klasörler ekle.",
    "Settings on the TV": "TV'de ayarlar",
    "A Settings tile after the apps, for whoever has only the TV remote. Only the phone can turn it back on.":
      "Uygulamaların sonunda bir Ayarlar kutucuğu, yalnızca TV kumandası olanlar için. Yeniden yalnızca telefondan açılabilir.",
    // HDMI-CEC
    "TV control (HDMI-CEC)": "TV kontrolü (HDMI-CEC)",
    "Control the TV over HDMI": "TV'yi HDMI üzerinden kontrol et",
    "Needs a CEC adapter on the box (/dev/cec*). Most PC graphics cards have none.":
      "Kutuda bir CEC bağdaştırıcısı gerekir (/dev/cec*). Çoğu PC ekran kartında yoktur.",
    "Turn the TV on and off": "TV'yi aç ve kapat",
    "Waking turns the TV on and switches it to rustv; sleep puts it in standby, unless it shows another input.":
      "Uyanınca TV açılır ve rustv'ye geçer; uykuda TV bekleme moduna geçer, başka bir giriş açıksa dokunulmaz.",
    "Follow the TV": "TV'yi takip et",
    "Turning the TV off puts rustv to sleep; switching the TV to rustv wakes it.":
      "TV kapanınca rustv uyur; TV rustv'ye geçince rustv uyanır.",
    "Use the TV remote": "TV kumandasını kullan",
    "Arrows, OK, Back, play and volume buttons control rustv.":
      "Yön, OK, Geri, oynat ve ses tuşları rustv'yi kontrol eder.",
    // gamepad
    Gamepad: "Oyun kolu",
    "Use a gamepad as the remote": "Oyun kolunu kumanda olarak kullan",
    "On the home screen: D-pad or stick to move, A to open, B back, shoulder buttons for volume. Inside apps the gamepad is the app's.":
      "Ana ekranda: yön tuşları ya da çubukla gez, A aç, B geri, omuz tuşları ses. Uygulamaların içinde oyun kolu uygulamaya aittir.",
    "Guide button goes home": "Ana menü tuşu ana ekrana döner",
    "The Xbox or PS button closes the open app.": "Xbox ya da PS tuşu açık uygulamayı kapatır.",
    // theme
    Theme: "Tema",
    Finish: "Kaplama",
    "Pick a metal, or your own colors on top of it. Dark backgrounds only.":
      "Bir metal seç ya da üstüne kendi renklerini koy. Yalnızca koyu arka planlar.",
    Silver: "Gümüş",
    Titanium: "Titanyum",
    Champagne: "Şampanya",
    Gunmetal: "Füme",
    "Your colors": "Kendi renklerin",
    Accent: "Vurgu",
    Pressed: "Basılı",
    Background: "Arka plan",
    "Back to the finish": "Kaplamaya dön",
    // network
    Network: "Ağ",
    "Open to the network": "Ağa açık olanlar",
    "What this TV box listens on. Ports change in each program's config; the firewall decides who can reach them.":
      "Bu TV kutusunun dinlediği portlar. Portlar her programın kendi ayarından değişir; kimin erişebileceğine güvenlik duvarı karar verir.",
    "Checking…": "Bakılıyor…",
    "all networks": "tüm ağlar",
    "rustv remote and API": "rustv kumanda ve API",
    "mDNS (device discovery)": "mDNS (cihaz bulma)",
    "DHCP (getting an address)": "DHCP (adres alma)",
    "DHCPv6 (getting an IPv6 address)": "DHCPv6 (IPv6 adresi alma)",
    "Printing (CUPS)": "Yazdırma (CUPS)",
    "qBittorrent web UI": "qBittorrent web arayüzü",
    // PIN
    "The TV is locked": "TV kilitli",
    "Enter the PIN": "PIN'i gir",
    "Enter the current PIN": "Mevcut PIN'i gir",
    "Choose a PIN (4 to 8 digits)": "Bir PIN seç (4-8 hane)",
    "Enter the new PIN again": "Yeni PIN'i tekrar gir",
    Confirm: "Onayla",
    Cancel: "Vazgeç",
    "At least 4 digits": "En az 4 hane",
    "PIN removed": "PIN kaldırıldı",
    "The PINs did not match; try again": "PIN'ler eşleşmedi; tekrar dene",
    "PIN saved": "PIN kaydedildi",
    Unlocked: "Kilit açıldı",
    // messages from tvd
    "Set a PIN in the settings first": "Önce ayarlardan bir PIN belirle",
    "Wrong PIN": "Yanlış PIN",
    "Wrong PIN. Entry is blocked for {n} s": "Yanlış PIN. Giriş {n} sn engellendi",
    "Too many wrong PINs; try again in {n} s": "Çok fazla yanlış PIN; {n} sn sonra tekrar dene",
    "The PIN must be 4 to 8 digits": "PIN 4-8 haneli olmalı",
    "The TV is locked. Enter the PIN on a remote to unlock it.": "TV kilitli. Açmak için kumandadan PIN'i gir.",
  },
};

const LANG_KEY = "rustv.lang";
let lang = localStorage.getItem(LANG_KEY) || (navigator.language || "").slice(0, 2);
if (!TRANSLATIONS[lang]) lang = "en";

function t(text, vars = {}) {
  if (text == null) return text;
  const table = TRANSLATIONS[lang] || {};
  let out = table[text];
  if (out === undefined) {
    // Messages with a number in them, such as "try again in 42 s".
    const numbers = String(text).match(/\d+/g);
    const pattern = String(text).replace(/\d+/g, "{n}");
    if (numbers && table[pattern] !== undefined) {
      let i = 0;
      out = table[pattern].replace(/\{n\}/g, () => numbers[i++]);
    } else {
      out = text;
    }
  }
  return String(out).replace(/\{(\w+)\}/g, (match, name) => (name in vars ? vars[name] : match));
}

/** "50%" in English, "%50" in Turkish. */
function percent(value) {
  return lang === "tr" ? `%${value}` : `${value}%`;
}

// Static text in the page, remembered in English so it can switch back and forth.
// Elements whose text the script sets itself are left to the script.
const DYNAMIC = "#now, #conn, #vol-value, #output-name, #pin-title, #pin-hint, #dock, #outputs, #settings-body, #pin-pad, #pin-dots, #toast, code, script, svg";
const staticText = [];
const staticAttrs = [];

function collectStatic() {
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.nodeValue.trim() && !node.parentElement.closest(DYNAMIC)) staticText.push([node, node.nodeValue]);
  }
  for (const el of document.querySelectorAll("[aria-label], [placeholder], [title]")) {
    for (const attr of ["aria-label", "placeholder", "title"]) {
      if (el.hasAttribute(attr)) staticAttrs.push([el, attr, el.getAttribute(attr)]);
    }
  }
}

function applyLanguage() {
  document.documentElement.lang = lang;
  for (const [node, english] of staticText) {
    // Keep the spaces around the words, they separate them from <code> and <br>.
    const [, lead, words, trail] = english.match(/^(\s*)([\s\S]*?)(\s*)$/);
    node.nodeValue = lead + t(words) + trail;
  }
  for (const [el, attr, english] of staticAttrs) el.setAttribute(attr, t(english));
}

function setLanguage(next) {
  if (!TRANSLATIONS[next] && next !== "en") next = "en";
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
        showPair(t("That token was rejected."));
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
  $("#conn").textContent = t(online ? "Connected" : "Reconnecting…");
}

let offlineNotice = 0;
function send(msg) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(msg));
    return true;
  }
  if (Date.now() - offlineNotice > 2500) {
    offlineNotice = Date.now();
    toast(t("Not connected to the TV"), "error");
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
    toast(t(msg.message), "error");
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
    toast(t("{name} is open on the TV", { name: running.name }), "ok");
    clearLaunch();
  } else if (pendingStop && !running) {
    pendingStop = false;
    if (before) toast(t("Closed {name}", { name: before.name }));
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
    toast(t("Sound now plays on {name}", { name: t(output.name) }), "ok");
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
  $("#now").textContent = state.asleep ? t("Asleep") : running ? running.name : t("Home");
  $("#conn").textContent = state.asleep
    ? t("Any button wakes the TV")
    : sleepLeft() !== null
      ? t("Sleeps in {minutes} min", { minutes: Math.max(1, Math.ceil(sleepLeft() / 60)) })
      : t("Connected");
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
  $("#mute").setAttribute("aria-label", t(volume && volume.muted ? "Unmute" : "Mute"));
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
    ? t("Muted")
    : percent(Math.round(sliderBusy() ? slider.value : volume.level * 100));
}

const OUTPUT_ICONS = { hdmi: "#i-tv", analog: "#i-headphones", bluetooth: "#i-bluetooth", usb: "#i-usb", other: "#i-speaker" };

function renderAudio() {
  slider.max = Math.round(audio.max_volume * 100);
  const current = audio.outputs.find((o) => o.default);
  $("#output-name").textContent = current ? t(current.name) : t("No output");
  $(".js-output-icon use").setAttribute("href", OUTPUT_ICONS[current ? current.kind : "other"] || "#i-speaker");
  $("#output").disabled = audio.outputs.length === 0;

  const list = $("#outputs");
  list.replaceChildren();
  if (!audio.outputs.length) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = t("No audio outputs found.");
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
    item.querySelector(".out-name").textContent = t(output.name);
    item.querySelector(".out-detail").textContent = output.default ? t("Playing here") : output.detail;
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
      toast(t("Could not switch to {name}", { name: t(output.name) }), "error");
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
    item.title = editing ? t("Drag to move {name}", { name: app.name }) : app.description || app.name;
    item.append(appIcon(app));
    const name = document.createElement("span");
    name.className = "dock-name";
    name.textContent = pendingLaunch && pendingLaunch.id === app.id ? t("Opening…") : app.name;
    item.append(name);
    bindDockItem(item, app, runningId);
    dock.append(item);
  }
  if (editing) {
    const done = document.createElement("button");
    done.className = "dock-done";
    done.textContent = t("Done");
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
  toast(t("Drag apps to reorder them, then tap Done"));
}

function finishEditing() {
  haptic();
  editing = false;
  renderDock();
  toast(t("Order saved on the TV"), "ok");
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
  if (app.id === runningId) return toast(t("{name} is already on the TV", { name: app.name }));
  if (pendingLaunch && pendingLaunch.id === app.id) return;
  if (!send({ type: "launch", id: app.id })) return;
  clearLaunch();
  pendingLaunch = {
    id: app.id,
    timer: setTimeout(() => {
      clearLaunch();
      toast(t("{name} did not open", { name: app.name }), "error");
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
  if (!display.modes || !display.modes.length) {
    const note = document.createElement("p");
    note.className = "setting-hint";
    note.textContent = t("The TV's resolutions show here once sway reports them.");
    return [note];
  }
  const modes = display.modes.map(splitMode);
  const on = splitMode(display.current || (display.mode !== "auto" ? display.mode : display.modes[0]));
  const sizes = [...new Set(modes.map((m) => m.size))];
  const rates = modes.filter((m) => m.size === on.size).map((m) => m.hz);
  const tryMode = (mode) => send({ type: "display_mode", mode });
  const resolution = choiceRow(
    t("Resolution"),
    t("A new mode goes back by itself after 15 seconds unless you keep it."),
    sizes.map((size) => [size, size.replace("x", " × ")]),
    on.size,
    (size) => {
      // Keep the refresh rate where the new size offers it, else the fastest up to 60 Hz.
      const offered = modes.filter((m) => m.size === size).map((m) => m.hz);
      const hz = offered.includes(on.hz) ? on.hz : offered.find((h) => Number(h) <= 60) || offered[0];
      tryMode(`${size}@${hz}`);
    },
  );
  const refresh = choiceRow(t("Refresh rate"), "", rates.map((hz) => [hz, `${hz} Hz`]), on.hz, (hz) =>
    tryMode(`${on.size}@${hz}`),
  );
  return [resolution, refresh];
}

/** The "keep this resolution?" prompt, while a mode is on trial. */
function renderTrial() {
  const trial = state && state.display_trial;
  $("#trial").hidden = !trial;
  if (!trial) return;
  $("#trial-text").textContent = t("{mode}. Going back in {seconds} s.", {
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
      ? t("Turn the TV off after a while, like on a TV remote.")
      : t("The TV sleeps in {minutes} min.", { minutes: Math.max(1, Math.ceil(left / 60)) });
  chips.replaceChildren();
  for (const minutes of TIMER_CHOICES) {
    const chip = document.createElement("button");
    chip.className = "chip";
    chip.textContent = minutes % 60 ? t("{n} min", { n: minutes }) : t("{h} h", { h: minutes / 60 });
    chip.addEventListener("click", () => {
      haptic();
      if (send({ type: "sleep_timer", minutes })) {
        closeTimer();
        toast(t("The TV sleeps in {minutes} min.", { minutes }), "ok");
      }
    });
    chips.append(chip);
  }
  if (left !== null) {
    const off = document.createElement("button");
    off.className = "chip";
    off.textContent = t("Turn the timer off");
    off.addEventListener("click", () => {
      haptic();
      if (send({ type: "sleep_timer", minutes: 0 })) {
        closeTimer();
        toast(t("Sleep timer off"));
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
const SETTINGS = [
  {
    title: "Home screen",
    items: [
      { key: "shell.greeting", label: "Greeting" },
      { key: "shell.clock", label: "Clock" },
      { key: "shell.date", label: "Date" },
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
      {
        key: "shell.settings",
        label: "Settings on the TV",
        hint: "A Settings tile after the apps, for whoever has only the TV remote. Only the phone can turn it back on.",
      },
    ],
  },
  { title: "Theme", custom: themeRows },
  { title: "Display", custom: displayRows },
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
        options: [["random", "Mix"], ["aurora", "Aurora"], ["orbits", "Orbits"], ["clock", "Clock"], ["images", "My pictures"]],
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
      {
        key: "audio.per_app_volume",
        label: "Remember the volume for each app",
        hint: "A loud game and a quiet film each come back at their own level.",
      },
      { key: "audio.step", label: "Volume buttons change by", options: [[0.02, "2%"], [0.05, "5%"], [0.1, "10%"]] },
    ],
  },
  {
    title: "Language and format",
    items: [
      { key: "locale.language", label: "Language", options: [["en", "English"], ["tr", "Türkçe"]] },
      { key: "locale.clock", label: "Time format", options: [["24h", "24-hour"], ["12h", "12-hour"]] },
      {
        key: "locale.temperature",
        label: "Temperature",
        hint: "Used by the debug overlay.",
        options: [["celsius", "°C"], ["fahrenheit", "°F"]],
      },
    ],
  },
  {
    title: "Lock and PIN",
    items: [
      {
        label: "PIN",
        hint: "Locking needs a PIN. It is kept on the TV box, hashed, never in the config.",
        button: () => (settingValue("lock.pin_set") ? "Change" : "Set PIN"),
        run: () => startPinFlow(settingValue("lock.pin_set") ? "change" : "set"),
      },
      {
        label: "Remove the PIN",
        needs: "lock.pin_set",
        button: "Remove",
        run: () => startPinFlow("remove"),
      },
      {
        label: "Lock the TV now",
        hint: "Closes the open app; nothing works until the PIN is entered.",
        needs: "lock.pin_set",
        button: "Lock",
        run: () => {
          if (send({ type: "lock" })) closeSettings();
        },
      },
      { key: "lock.on_sleep", label: "Lock when the TV goes to sleep", needs: "lock.pin_set" },
    ],
  },
  {
    title: "Gamepad",
    items: [
      {
        key: "gamepad.enabled",
        label: "Use a gamepad as the remote",
        hint: "On the home screen: D-pad or stick to move, A to open, B back, shoulder buttons for volume. Inside apps the gamepad is the app's.",
      },
      {
        key: "gamepad.guide_closes_app",
        label: "Guide button goes home",
        hint: "The Xbox or PS button closes the open app.",
        needs: "gamepad.enabled",
      },
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
    title: "Sleep mode",
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
    title: "Wallpaper",
    custom: () => [
      wallpaperCount(),
      ...[
        {
          key: "wallpaper.home",
          label: "Behind the home screen",
          hint: "Your pictures instead of the theme's background.",
        },
        {
          key: "wallpaper.change_minutes",
          label: "Change every",
          needs: "wallpaper.home",
          options: [[5, "5 min"], [15, "15 min"], [30, "30 min"], [60, "1 h"], [240, "4 h"], [1440, "1 day"]],
        },
        {
          key: "wallpaper.slide_seconds",
          label: "Screensaver: each picture for",
          options: [[10, "10 s"], [30, "30 s"], [60, "1 min"], [300, "5 min"]],
        },
        { key: "wallpaper.shuffle", label: "Random order" },
      ].map(settingRow),
    ],
  },
  {
    title: "TV control (HDMI-CEC)",
    items: [
      {
        key: "cec.enabled",
        label: "Control the TV over HDMI",
        hint: "Needs a CEC adapter on the box (/dev/cec*). Most PC graphics cards have none.",
      },
      {
        key: "cec.power_tv",
        label: "Turn the TV on and off",
        hint: "Waking turns the TV on and switches it to rustv; sleep puts it in standby, unless it shows another input.",
        needs: "cec.enabled",
      },
      {
        key: "cec.follow_tv",
        label: "Follow the TV",
        hint: "Turning the TV off puts rustv to sleep; switching the TV to rustv wakes it.",
        needs: "cec.enabled",
      },
      {
        key: "cec.remote_keys",
        label: "Use the TV remote",
        hint: "Arrows, OK, Back, play and volume buttons control rustv.",
        needs: "cec.enabled",
      },
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

// How many pictures the config lists; the list itself is edited in the config file.
function wallpaperCount() {
  const count = settings.wallpaper ? settings.wallpaper.count : 0;
  const note = document.createElement("p");
  note.className = "setting-hint";
  note.textContent = count
    ? t("Pictures in [wallpaper] images: {count}", { count })
    : t("Add picture files or folders to [wallpaper] images in the config file.");
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
    empty.textContent = t("Waiting for the TV…");
    body.append(empty);
    return;
  }
  for (const group of SETTINGS) {
    const section = document.createElement("section");
    section.className = "settings-group";
    const title = document.createElement("h3");
    title.textContent = t(group.title);
    section.append(title);
    for (const row of group.custom ? group.custom() : group.items.map(settingRow)) section.append(row);
    body.append(section);
  }
  body.append(wifiGroup(), bluetoothGroup(), networkGroup());

  const about = document.createElement("p");
  about.className = "settings-about";
  about.textContent = t("rustv {version} · saved to {path}", { version: settings.version, path: settings.config_path });
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
  label.textContent = t(item.label);
  text.append(label);
  if (item.hint) {
    const hint = document.createElement("div");
    hint.className = "setting-hint";
    hint.textContent = t(item.hint);
    text.append(hint);
  }
  row.append(text);

  const value = item.live === "overlay" ? Boolean(state && state.overlay) : settingValue(item.key);
  if ((item.action || item.run) && item.button !== null) {
    const button = document.createElement("button");
    button.className = "chip";
    button.textContent = t(typeof item.button === "function" ? item.button() : item.button);
    button.disabled = Boolean(off);
    button.addEventListener("click", () => {
      haptic();
      if (item.run) return item.run();
      if (send({ type: item.action })) {
        closeSettings();
        toast(t(item.done));
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
      chip.textContent = /^\d+(\.\d+)?%$/.test(optionLabel) ? percent(optionLabel.slice(0, -1)) : t(optionLabel);
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
    toggle.setAttribute("aria-label", t(item.label));
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
  ["accent", "Accent"],
  ["accent_2", "Pressed"],
  ["background", "Background"],
];

function themeRows() {
  const theme = settings.theme || { preset: "silver" };
  const own = COLOR_KEYS.filter(([key]) => theme[key]);

  const presets = document.createElement("div");
  presets.className = "setting stacked";
  const text = document.createElement("div");
  text.className = "setting-text";
  text.innerHTML = '<div class="setting-label"></div><div class="setting-hint"></div>';
  text.querySelector(".setting-label").textContent = t("Finish");
  text.querySelector(".setting-hint").textContent = t("Pick a metal, or your own colors on top of it. Dark backgrounds only.");
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
    chip.append(swatch, t(preset.name[0].toUpperCase() + preset.name.slice(1)));
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
  label.textContent = t("Your colors");
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
    reset.textContent = t("Back to the finish");
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
  title.textContent = t("Network");
  section.append(title);
  const intro = settingRow({
    label: "Open to the network",
    hint: "What this TV box listens on. Ports change in each program's config; the firewall decides who can reach them.",
  });
  section.append(intro);
  for (const e of exposures || []) {
    const row = document.createElement("div");
    row.className = "setting";
    const text = document.createElement("div");
    text.className = "setting-text";
    const label = document.createElement("div");
    label.className = "setting-label";
    label.textContent = t(e.service);
    const hint = document.createElement("div");
    hint.className = "setting-hint";
    const more = e.address.match(/^(.*) and (\d+) more addresses$/);
    const address = more ? (lang === "tr" ? `${more[1]} ve ${more[2]} adres daha` : e.address) : t(e.address);
    hint.textContent = `${e.protocol.toUpperCase()} ${e.port} · ${address}`;
    text.append(label, hint);
    row.append(text);
    section.append(row);
  }
  if (!exposures) {
    const row = settingRow({ label: "Checking…" });
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
    toast(t(body.error || "Something went wrong"), "error");
  } catch {
    toast(t("Not connected to the TV"), "error");
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
  title.textContent = t("Wi-Fi");
  section.append(title);
  section.append(
    settingRow({
      label: "Networks nearby",
      hint: "Switching networks drops the remote for a moment; with a wrong password the TV returns to the old one.",
      button: wirelessBusy === "wifi-scan" ? "Searching…" : "Search again",
      run: () => {
        wirelessBusy = "wifi-scan";
        renderSettings();
        loadWifi(true).then(() => (wirelessBusy = ""));
      },
    }),
  );
  if (!wifi) {
    section.append(infoRow("Checking…"));
  } else if (wifi.error) {
    section.append(infoRow("Wi-Fi is not available", wifi.error));
  } else {
    for (const network of wifi.slice(0, 12)) {
      const busy = wirelessBusy === `wifi:${network.ssid}`;
      section.append(
        settingRow({
          label: network.ssid,
          hint: `${network.connected ? t("Connected") + " · " : ""}${t("Signal")} ${percent(network.signal)}${network.secure ? " · 🔒" : ""}`,
          button: network.connected ? null : busy ? "Joining…" : "Join",
          run: network.connected
            ? null
            : () => {
                let password = null;
                if (network.secure) {
                  password = window.prompt(t("Password for {name} (leave empty if the TV knows it)", { name: network.ssid }));
                  if (password === null) return;
                }
                wirelessCommand(
                  `wifi:${network.ssid}`,
                  { type: "wifi_connect", ssid: network.ssid, password: password || null },
                  { text: "Joined {name}", vars: { name: network.ssid } },
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
  title.textContent = t("Bluetooth");
  section.append(title);
  if (!bluetooth) {
    section.append(infoRow("Checking…"));
    return section;
  }
  if (!bluetooth.available) {
    section.append(infoRow("Bluetooth is not available", "No adapter, or bluetoothd is not running."));
    return section;
  }
  const power = settingRow({ label: "Bluetooth" });
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
      label: "Add a device",
      hint: "Put headphones or a gamepad in pairing mode, then search.",
      button: wirelessBusy === "bt-scan" ? "Searching…" : "Search",
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
      hint: device.connected ? "Connected" : device.paired ? "Paired" : "Nearby",
      button: busy ? "…" : device.connected ? "Disconnect" : device.paired ? "Connect" : "Pair",
      run: () =>
        device.connected
          ? act("disconnect")
          : device.paired
            ? act("connect", { text: "Connected to {name}", vars: { name: device.name } })
            : act("pair", { text: "Paired with {name}", vars: { name: device.name } }),
    });
    if (device.paired && !busy) {
      const forget = document.createElement("button");
      forget.className = "chip";
      forget.textContent = t("Forget");
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
  unlock: "Enter the PIN",
  current: "Enter the current PIN",
  new: "Choose a PIN (4 to 8 digits)",
  confirm: "Enter the new PIN again",
};

function buildPad() {
  const pad = $("#pin-pad");
  for (const key of ["1", "2", "3", "4", "5", "6", "7", "8", "9", "del", "0", "ok"]) {
    const button = document.createElement("button");
    button.className = "pin-key" + (key === "ok" ? " primary" : "");
    button.textContent = key === "del" ? "⌫" : key === "ok" ? "OK" : key;
    button.setAttribute("aria-label", key === "del" ? t("Delete") : key === "ok" ? t("Confirm") : key);
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
  $("#pin-title").textContent = t(locked ? "The TV is locked" : pinFlow.mode === "remove" ? "Remove the PIN" : "PIN");
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
  if (pinDigits.length < 4) return toast(t("At least 4 digits"), "error");
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
      toast(t("PIN removed"), "ok");
    } else {
      pinFlow.step++;
    }
  } else if (step === "new") {
    pinFlow.first = digits;
    pinFlow.step++;
  } else if (step === "confirm") {
    if (digits !== pinFlow.first) {
      toast(t("The PINs did not match; try again"), "error");
      pinFlow.step = PIN_STEPS[pinFlow.mode].indexOf("new");
    } else {
      send({ type: "set_pin", current: pinFlow.current, pin: digits });
      pinFlow = null;
      toast(t("PIN saved"), "ok");
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
    toast(t("Unlocked"), "ok");
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
    toast(t("Sent to the TV"), "ok");
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

collectStatic();
applyLanguage();
start();
