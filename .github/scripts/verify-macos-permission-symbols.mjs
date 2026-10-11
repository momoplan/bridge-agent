import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// These functions are absent on some supported macOS releases. A deployment
// target alone cannot protect an executable that imports them directly.
const optionalSymbols = new Set([
  "_CGPreflightPostEventAccess",
  "_CGPreflightListenEventAccess",
  "_CGPreflightScreenCaptureAccess",
  "_CGRequestPostEventAccess",
  "_CGRequestScreenCaptureAccess",
]);

export function verifyPermissionImports(undefinedSymbols, label) {
  const imports = undefinedSymbols.split(/\s+/).filter((symbol) => optionalSymbols.has(symbol));
  if (imports.length > 0) {
    throw new Error(`${label} directly imports optional macOS permission APIs: ${[...new Set(imports)].join(", ")}`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const binary = process.argv[2];
  if (!binary) throw new Error("Usage: node verify-macos-permission-symbols.mjs <Mach-O binary>");
  const result = spawnSync("nm", ["-u", binary], { encoding: "utf8" });
  if (result.status !== 0) throw result.error ?? new Error(`nm failed: ${result.stderr}`);
  verifyPermissionImports(result.stdout, binary);
  console.log(`Optional macOS permission APIs are not directly imported: ${binary}`);
}
