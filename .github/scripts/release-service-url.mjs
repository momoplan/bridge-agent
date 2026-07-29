import { pathToFileURL } from "node:url";

const releaseApiPath = "/api/bridge-agent";
const updateApiPath = `${releaseApiPath}/releases/latest`;
const publicOrigin = "https://updates.baijimu.com";

export function normalizeReleaseApiBase(value) {
  return normalizeUrl(value, releaseApiPath, "release API");
}

export function normalizeUpdateApiUrl(value) {
  return normalizeUrl(value, updateApiPath, "update API");
}

function normalizeUrl(value, expectedPath, label) {
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`Missing ${label} URL`);
  }

  const canonical = `${publicOrigin}${expectedPath}`;
  const input = value.trim();
  if (input !== canonical && input !== `${canonical}/`) {
    throw new Error(
      `${label} URL must be exactly ${canonical}`,
    );
  }

  return canonical;
}

function main() {
  const [kind, value] = process.argv.slice(2);
  if (!kind || !value) {
    console.error(
      "Usage: release-service-url.mjs <release|update> <url>",
    );
    process.exitCode = 2;
    return;
  }

  if (kind === "release") {
    console.log(normalizeReleaseApiBase(value));
    return;
  }
  if (kind === "update") {
    console.log(normalizeUpdateApiUrl(value));
    return;
  }
  throw new Error(`Unknown release service URL kind: ${kind}`);
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main();
}
