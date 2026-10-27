// Browser tests for the web remote, against a real tvd with a throwaway config.
//
//   cargo build -p tvd
//   cd tests/remote && npm install && npx playwright install chromium
//   npx playwright test
//
// tvd needs /dev/uinput (it creates its virtual keyboard at start). The tests
// never press remote keys, so nothing reaches a real TV even on a TV box.

import { defineConfig, devices } from "@playwright/test";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const PORT = 8799;
const TOKEN = "0123456789abcdef0123456789abcdef";
// Playwright loads this file again in every worker; only the first load sets up
// the directory, and the workers find it through the environment.
const fresh = !process.env.RUSTV_TEST_DIR;
const dir = process.env.RUSTV_TEST_DIR ?? mkdtempSync(join(tmpdir(), "rustv-remote-test-"));
process.env.RUSTV_TEST_DIR = dir;
const config = join(dir, "config.toml");
if (fresh) {
  mkdirSync(join(dir, "state"));
  // One 1x1 PNG as the only wallpaper, next to a file that is not a picture.
  mkdirSync(join(dir, "pictures"));
  const png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
  writeFileSync(join(dir, "pictures", "one.png"), Buffer.from(png, "base64"));
  writeFileSync(join(dir, "pictures", "notes.txt"), "not a picture");
  writeFileSync(join(dir, "token"), `${TOKEN}\n`);
  writeFileSync(
    config,
  `[server]
bind = "127.0.0.1:${PORT}"
token_file = "${join(dir, "token")}"

[audio]
per_app_volume = false

[sleep]
enabled = false

[gamepad]
enabled = false

[screensaver]
idle_minutes = 5

[wallpaper]
images = ["${join(dir, "pictures")}"]

[[app]]
id = "films"
name = "Films"
kind = "command"
command = ["true"]

[[app]]
id = "games"
name = "Games"
kind = "command"
command = ["true"]

[[app]]
id = "router"
name = "Router"
kind = "web"
url = "http://localhost:1"
`,
  );
}

// Read by the tests.
process.env.RUSTV_TEST_TOKEN = TOKEN;
process.env.RUSTV_TEST_CONFIG = config;

const tvd = resolve(import.meta.dirname, "../../target/debug/tvd");

export default defineConfig({
  testDir: ".",
  // One tvd for all tests, and several of them change its settings.
  workers: 1,
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    ...devices["Pixel 7"],
    trace: "retain-on-failure",
  },
  webServer: {
    command: `${tvd} --config ${config}`,
    url: `http://127.0.0.1:${PORT}/api/v1/health`,
    env: { XDG_STATE_HOME: join(dir, "state"), RUST_LOG: "warn" },
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
