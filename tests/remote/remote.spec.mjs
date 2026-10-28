// What a person sees and does in the web remote. Each test opens a fresh page
// against the same tvd (see playwright.config.mjs); tests that change a setting
// put it back.

import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";

const TOKEN = process.env.RUSTV_TEST_TOKEN;
const CONFIG = process.env.RUSTV_TEST_CONFIG;

async function open(page) {
  await page.goto(`/#token=${TOKEN}`);
  await expect(page.locator("#dot")).toHaveClass(/on/);
}

async function openSettings(page) {
  await page.locator("#settings-open").click();
  await expect(page.locator("#settings-body .settings-group").first()).toBeVisible();
}

/** The settings row with this label. */
function row(page, label) {
  return page.locator("#settings-body .setting").filter({ has: page.locator(".setting-label", { hasText: label }) });
}

test("pairs from the link and connects", async ({ page }) => {
  await open(page);
  await expect(page.locator("#app")).toBeVisible();
  await expect(page.locator("#pair")).toBeHidden();
  // The token leaves the address bar once it is stored.
  expect(page.url()).not.toContain("token=");
});

test("shows the pairing screen for a wrong token", async ({ page }) => {
  await page.goto("/#token=wrongwrongwrongwrongwrongwrong");
  await expect(page.locator("#pair")).toBeVisible();
});

test("lists the configured apps in the dock", async ({ page }) => {
  await open(page);
  await expect(page.locator("#dock .dock-name")).toHaveText(["Films", "Games", "Router"]);
});

test("settings show every section, with switches only on real settings", async ({ page }) => {
  await open(page);
  await openSettings(page);
  const titles = await page.locator("#settings-body h3").allTextContents();
  for (const title of ["Home screen", "Theme", "Screensaver", "Sound", "Language and format", "Lock and PIN", "Sleep mode", "Wallpaper", "TV control (HDMI-CEC)", "Network"]) {
    expect(titles).toContain(title);
  }
  // Rows that only explain something must not grow a switch.
  await expect(row(page, "Open to the network").locator(".toggle")).toHaveCount(0);
  await expect(row(page, "Greeting").locator(".toggle")).toHaveCount(1);
});

test("wallpapers: only the listed pictures, only with the token", async ({ page, request }) => {
  await open(page);
  await openSettings(page);
  await expect(page.locator("#settings-body")).toContainText("Pictures in [wallpaper] images: 1");
  const auth = { headers: { Authorization: `Bearer ${TOKEN}` } };
  const first = await request.get("/api/v1/wallpapers/0", auth);
  expect(first.status()).toBe(200);
  expect(first.headers()["content-type"]).toBe("image/png");
  // notes.txt is in the folder but is no picture, so there is no second one.
  expect((await request.get("/api/v1/wallpapers/1", auth)).status()).toBe(404);
  expect((await request.get("/api/v1/wallpapers/0")).status()).toBe(401);
});

test("a changed setting is saved to config.toml", async ({ page }) => {
  await open(page);
  await openSettings(page);
  await row(page, "Start after").getByRole("button", { name: "10 min", exact: true }).click();
  await expect.poll(() => readFileSync(CONFIG, "utf8")).toContain("idle_minutes = 10");
  await row(page, "Start after").getByRole("button", { name: "5 min", exact: true }).click();
  await expect.poll(() => readFileSync(CONFIG, "utf8")).toContain("idle_minutes = 5");
});

test("switching to Turkish translates the remote, and back", async ({ page }) => {
  await open(page);
  await openSettings(page);
  await row(page, "Language").getByRole("button", { name: "Türkçe", exact: true }).click();
  await expect(page.locator('[data-tab="remote"]')).toHaveText("Kumanda");
  await expect(page.locator("#settings-title")).toHaveText("Ayarlar");
  await row(page, "Dil").getByRole("button", { name: "English", exact: true }).click();
  await expect(page.locator('[data-tab="remote"]')).toHaveText("Remote");
});

test("no catalog key shows as raw text, in either language", async ({ page }) => {
  await open(page);
  await openSettings(page);
  // What a missing translation looks like: the key itself, such as "setting.sleep.enabled.label".
  const rawKeys = () =>
    page.evaluate(() => {
      const pattern = /\b(common|error|network|option|osk|output|remote|section|setting|stats|tv|unit)\.[a-z0-9_]+(\.[a-z0-9_]+)*\b/;
      const texts = [document.body.innerText];
      for (const el of document.querySelectorAll("[aria-label], [placeholder], [title]")) {
        texts.push(el.getAttribute("aria-label") || "", el.getAttribute("placeholder") || "", el.getAttribute("title") || "");
      }
      return texts.flatMap((text) => text.match(new RegExp(pattern, "g")) || []);
    });
  expect(await rawKeys()).toEqual([]);
  await row(page, "Language").getByRole("button", { name: "Türkçe", exact: true }).click();
  await expect(page.locator("#settings-title")).toHaveText("Ayarlar");
  expect(await rawKeys()).toEqual([]);
  await row(page, "Dil").getByRole("button", { name: "English", exact: true }).click();
  await expect(page.locator("#settings-title")).toHaveText("Settings");
});

test("a finish recolors the remote", async ({ page }) => {
  await open(page);
  await openSettings(page);
  const accent = () => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--accent").trim());
  await row(page, "Finish").getByRole("button", { name: "Champagne", exact: true }).click();
  await expect.poll(accent).toBe("#d5c4a1");
  await row(page, "Finish").getByRole("button", { name: "Silver", exact: true }).click();
  await expect.poll(accent).toBe("#c9ced4");
});

test("the sleep timer shows in the header and can be turned off", async ({ page }) => {
  await open(page);
  await page.locator("#timer-open").click();
  await page.locator("#timer-chips").getByRole("button", { name: "15 min", exact: true }).click();
  await expect(page.locator("#conn")).toHaveText(/Sleeps in 15 min/);
  await page.locator("#timer-open").click();
  await page.locator("#timer-chips").getByRole("button", { name: "Turn the timer off", exact: true }).click();
  await expect(page.locator("#conn")).toHaveText("Connected");
});

test.describe("on a small phone", () => {
  test.use({ viewport: { width: 360, height: 640 } });

  test("everything fits on one screen without overlapping", async ({ page }) => {
    await open(page);
    const box = (sel) => page.locator(sel).boundingBox();
    const [volume, dock, tabs] = [await box("#volume"), await box("#dock"), await box(".tabs")];
    expect(volume.y + volume.height).toBeLessThanOrEqual(dock.y);
    expect(dock.y + dock.height).toBeLessThanOrEqual(tabs.y);
    expect(tabs.y + tabs.height).toBeLessThanOrEqual(640);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow).toBeLessThanOrEqual(0);
  });
});

test("holding OK holds the key on the TV until the finger lifts", async ({ page }) => {
  await open(page);
  // Capture instead of sending: the test tvd's virtual keyboard reaches the real session.
  await page.evaluate(() => {
    window.sent = [];
    ws.send = (data) => window.sent.push(JSON.parse(data));
  });
  const ok = page.locator("#dpad .ok");
  const box = await ok.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(700);
  expect(await page.evaluate(() => window.sent)).toEqual([{ type: "key", code: "Enter", action: "down" }]);
  await page.mouse.up();
  expect(await page.evaluate(() => window.sent)).toEqual([
    { type: "key", code: "Enter", action: "down" },
    { type: "key", code: "Enter", action: "up" },
  ]);
});

/** Presses an app in the dock and keeps it held for `ms`. Returns the app's centre. */
async function holdApp(page, name, ms = 600) {
  const item = page.locator("#dock .dock-item").filter({ hasText: name });
  const box = await item.boundingBox();
  const at = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  await page.waitForTimeout(ms);
  return { item, at, width: box.width };
}

test("hold an app and drag it, in one go, to move it", async ({ page }) => {
  await open(page);
  // Capture instead of sending, so the shared tvd keeps its order.
  await page.evaluate(() => {
    window.sent = [];
    ws.send = (data) => window.sent.push(JSON.parse(data));
  });
  const { item, at, width } = await holdApp(page, "Films");
  await expect(item).toHaveClass(/lifted/);
  await page.mouse.move(at.x + width * 0.7, at.y, { steps: 6 });
  await page.mouse.move(at.x + width * 1.4, at.y, { steps: 6 });
  await page.mouse.up();
  await expect(page.locator("#dock .dock-name")).toHaveText(["Games", "Films", "Router"]);
  expect(await page.evaluate(() => window.sent)).toEqual([{ type: "reorder", ids: ["games", "films", "router"] }]);
  // A drag is not a launch, and no menu opens.
  await expect(page.locator("#appmenu")).toBeHidden();
});

test("hold an app and let go for its menu; remove puts it in settings", async ({ page }) => {
  await open(page);
  await holdApp(page, "Router");
  await page.mouse.up();
  const menu = page.locator("#appmenu");
  await expect(menu).toBeVisible();
  await expect(menu.locator("#appmenu-title")).toHaveText("Router");
  await expect(menu.locator(".appmenu-action .out-name")).toHaveText([
    "Open on the TV",
    "Info",
    "Move",
    "Remove from home",
  ]);
  await menu.getByRole("button", { name: /Info/ }).click();
  await expect(menu.locator("#appmenu-info")).toBeVisible();
  await expect(menu.locator("#appmenu-info dd").nth(0)).toHaveText("Web page");

  await menu.getByRole("button", { name: /Remove from home/ }).click();
  await expect(page.locator("#dock .dock-name")).toHaveText(["Films", "Games"]);
  await openSettings(page);
  await expect(row(page, "Router")).toBeVisible();
  await row(page, "Router").getByRole("button", { name: "Put back" }).click();
  await expect(page.locator("#dock .dock-name")).toHaveText(["Films", "Games", "Router"]);
  await expect(row(page, "Router")).toHaveCount(0);
});

test("right-click opens an app's menu, and Move wiggles the dock", async ({ page }) => {
  await open(page);
  await page.locator("#dock .dock-item").filter({ hasText: "Games" }).click({ button: "right" });
  await expect(page.locator("#appmenu")).toBeVisible();
  await page.locator("#appmenu").getByRole("button", { name: /^Move/ }).click();
  await expect(page.locator("#appmenu")).toBeHidden();
  await expect(page.locator("#dock")).toHaveClass(/editing/);
  await page.locator(".dock-done").click();
  await expect(page.locator("#dock")).not.toHaveClass(/editing/);
});

test.describe("on a touch screen", () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } });

  test("a held finger drags an app without scrolling the dock", async ({ page }) => {
    await open(page);
    await page.evaluate(() => {
      window.sent = [];
      ws.send = (data) => window.sent.push(JSON.parse(data));
    });
    const box = await page.locator("#dock .dock-item").filter({ hasText: "Films" }).boundingBox();
    const [x, y] = [box.x + box.width / 2, box.y + box.height / 2];
    const cdp = await page.context().newCDPSession(page);
    const touch = (type, px) => cdp.send("Input.dispatchTouchEvent", { type, touchPoints: type === "touchEnd" ? [] : [{ x: px, y }] });
    await touch("touchStart", x);
    await page.waitForTimeout(700);
    for (let step = 1; step <= 12; step++) {
      await touch("touchMove", x + (box.width * 1.4 * step) / 12);
      await page.waitForTimeout(16);
    }
    await touch("touchEnd");
    await expect(page.locator("#dock .dock-name")).toHaveText(["Games", "Films", "Router"]);
    expect(await page.evaluate(() => window.sent)).toEqual([{ type: "reorder", ids: ["games", "films", "router"] }]);
    await expect(page.locator("#appmenu")).toBeHidden();
  });

  test("a held finger let go opens the menu", async ({ page }) => {
    await open(page);
    const box = await page.locator("#dock .dock-item").filter({ hasText: "Games" }).boundingBox();
    const point = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point] });
    await page.waitForTimeout(700);
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await expect(page.locator("#appmenu")).toBeVisible();
    await expect(page.locator("#appmenu-title")).toHaveText("Games");
  });
});
