import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const read = (path) => JSON.parse(readFileSync(new URL(`../${path}`, import.meta.url), "utf8"));
const compatibility = read("config/client-compatibility.json");
assert.equal(read("src-tauri/tauri.conf.json").bundle.macOS.minimumSystemVersion, compatibility.macos.x86_64,
  "Tauri packaging must use the versioned Intel macOS baseline");
for (const version of Object.values(compatibility.macos)) {
  assert.match(version, /^\d+\.\d+(?:\.\d+)?$/, "macOS requirements must use numeric product versions");
}
assert(compatibility.webview.javascript.includes(compatibility.webview.css), "CSS and JavaScript must share the oldest WebView target");
console.log("Client packaging and WebView compatibility baseline verified.");
