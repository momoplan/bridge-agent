import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDir, "..");
const skipSigningForQualityGate = process.argv.includes("--unsigned-for-quality-gate");
const buildFrontend = process.argv.includes("--with-frontend");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    env: process.env,
    stdio: "inherit",
    shell: false,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
}

if (process.env.TAURI_ENV_PLATFORM !== "windows") {
  process.exit(0);
}

if (buildFrontend) {
  run(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "build:tauri"]);
}

const buildRoot = path.join(repositoryRoot, "src-tauri", "target", "windows-uninstaller");
run("cargo", [
  "build",
  "--locked",
  "--release",
  "--manifest-path",
  "tools/windows-uninstaller/Cargo.toml",
  "--target-dir",
  buildRoot,
]);

const rustc = spawnSync("rustc", ["-vV"], {
  cwd: repositoryRoot,
  env: process.env,
  encoding: "utf8",
  shell: false,
});
if (rustc.error) throw rustc.error;
if (rustc.status !== 0) throw new Error(`rustc -vV exited with status ${rustc.status}`);
const host = rustc.stdout.match(/^host:\s*(\S+)$/m)?.[1];
if (!host || !host.endsWith("-pc-windows-msvc")) {
  throw new Error(`unsupported Windows Rust host: ${host ?? "unknown"}`);
}

const source = path.join(buildRoot, "release", "bridge-agent-uninstaller.exe");
const binariesDir = path.join(repositoryRoot, "src-tauri", "binaries");
const destination = path.join(binariesDir, `bridge-agent-uninstaller-${host}.exe`);
mkdirSync(binariesDir, { recursive: true });
copyFileSync(source, destination);

if (!skipSigningForQualityGate) {
  run("powershell", [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    "src-tauri/scripts/sign-windows-artifact.ps1",
    destination,
  ]);
}
