import { basename } from "node:path";
import { pathToFileURL } from "node:url";

import { normalizeReleaseApiBase } from "./release-service-url.mjs";

export function buildManifest(entries) {
  if (entries.length === 0) {
    throw new Error("Release manifest must contain at least one asset");
  }
  const identities = new Set();
  return {
    assets: entries.map((entry) => {
      const [target, filePath, signatureRequired] = entry.split("::");
      if (!target || !filePath || !["true", "false"].includes(signatureRequired)) {
        throw new Error(`Invalid manifest entry: ${entry}`);
      }
      const name = basename(filePath);
      const identity = `${target}\u0000${name}`;
      if (identities.has(identity)) {
        throw new Error(`Duplicate manifest asset: ${target}/${name}`);
      }
      identities.add(identity);
      return { target, name, signatureRequired: signatureRequired === "true" };
    }),
  };
}

async function main() {
  const [apiBaseArg, tagName, ...entries] = process.argv.slice(2);
  if (!apiBaseArg || !tagName) {
    throw new Error(
      "Usage: register-release-manifest.mjs <apiBase> <tagName> <target::file::signatureRequired>...",
    );
  }
  const token = process.env.BRIDGE_AGENT_RELEASE_API_TOKEN?.trim();
  if (!token) throw new Error("Missing BRIDGE_AGENT_RELEASE_API_TOKEN");
  const apiBase = normalizeReleaseApiBase(apiBaseArg);
  const response = await fetch(
    `${apiBase}/releases/${encodeURIComponent(tagName)}/manifest`,
    {
      method: "PUT",
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
        "X-Bridge-Release-Actor": "github-actions",
      },
      body: JSON.stringify(buildManifest(entries)),
      signal: AbortSignal.timeout(60_000),
    },
  );
  if (!response.ok) {
    throw new Error(
      `release service returned HTTP ${response.status}: ${(await response.text()).slice(0, 512)}`,
    );
  }
  console.log(`Registered ${entries.length} expected assets for ${tagName}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
