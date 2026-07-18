import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDir, "..");

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    env: process.env,
    stdio: "inherit",
    shell: false,
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
}

run(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "build:web"]);

if (process.platform === "win32") {
  const cargoTargetDir = path.join(repositoryRoot, "src-tauri", "target");
  run("cargo.exe", ["build", "--release", "--bin", "bridge-agent-service"], {
    env: {
      ...process.env,
      CARGO_TARGET_DIR: cargoTargetDir,
    },
  });

  const serviceExecutable = path.join(
    cargoTargetDir,
    "release",
    "bridge-agent-service.exe",
  );
  if (!existsSync(serviceExecutable)) {
    throw new Error(`Windows service executable was not built: ${serviceExecutable}`);
  }

  run("powershell.exe", [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    path.join(repositoryRoot, "src-tauri", "scripts", "sign-windows-artifact.ps1"),
    serviceExecutable,
  ]);
}
