import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { readFile, stat } from "node:fs/promises";
import { basename } from "node:path";
import { pathToFileURL } from "node:url";

import { normalizeReleaseApiBase } from "./release-service-url.mjs";

async function main() {
  const [apiBaseArg, tagName, version, target, filePath, signaturePath] =
    process.argv.slice(2);
  if (!apiBaseArg || !tagName || !version || !target || !filePath) {
    console.error(
      "Usage: upload-oss-release-asset.mjs <apiBase> <tagName> <version> <target> <filePath> [signaturePath]",
    );
    process.exitCode = 2;
    return;
  }

  const apiBase = normalizeReleaseApiBase(apiBaseArg);
  const assetName = basename(filePath);
  const fileStat = await stat(filePath);
  const sha256 = await sha256File(filePath);
  const contentType = contentTypeFor(assetName);
  const signature = signaturePath
    ? (await readFile(signaturePath, "utf8")).trim()
    : undefined;
  if (signaturePath && !signature) {
    throw new Error(`Updater signature is empty: ${signaturePath}`);
  }

  const metadata = {
    tagName,
    version,
    target,
    name: assetName,
    sha256,
    contentType,
    sizeBytes: fileStat.size,
    signature,
  };
  const prepared = await releaseServiceJson(
    `${apiBase}/releases/${encodeURIComponent(tagName)}/assets/prepare`,
    metadata,
  );
  validatePrepareResponse(prepared, assetName);

  const uploadResponse = await fetch(prepared.uploadUrl, {
    method: prepared.method || "PUT",
    headers: prepared.headers,
    body: createReadStream(filePath),
    duplex: "half",
    signal: AbortSignal.timeout(15 * 60_000),
  });
  if (!uploadResponse.ok) {
    throw new Error(`OSS upload failed with HTTP ${uploadResponse.status}`);
  }

  await verifyPublicObject(
    prepared.downloadUrl,
    fileStat.size,
    sha256,
    assetName,
  );
  await releaseServiceJson(
    `${apiBase}/releases/${encodeURIComponent(tagName)}/assets/complete`,
    completionPayload(metadata, prepared),
  );
  console.log(
    `Registered immutable OSS asset ${assetName} (${fileStat.size} bytes, sha256:${sha256})`,
  );
}

async function releaseServiceJson(url, body) {
  const releaseServiceToken = requiredEnv("BRIDGE_AGENT_RELEASE_API_TOKEN");
  const response = await fetch(url, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${releaseServiceToken}`,
      "Content-Type": "application/json",
      "X-Bridge-Release-Actor": "github-actions",
    },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(60_000),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(
      `release service returned HTTP ${response.status}: ${text.slice(0, 512)}`,
    );
  }
  return JSON.parse(text);
}

export function validatePrepareResponse(prepared, expectedName) {
  const downloadUrl = new URL(prepared.downloadUrl);
  if (
    downloadUrl.protocol !== "https:" ||
    !isPublicAliyunOssObjectHost(downloadUrl.hostname) ||
    downloadUrl.search ||
    downloadUrl.hash
  ) {
    throw new Error(
      "release service returned a non-canonical public OSS download URL",
    );
  }
  if (
    !prepared.objectKey?.startsWith(
      "lowcode/direct-uploads/bridge-agent-release/",
    ) ||
    !prepared.objectKey.endsWith(`-${expectedName}`) ||
    downloadUrl.pathname !== `/${prepared.objectKey}`
  ) {
    throw new Error("release service returned an invalid immutable object key");
  }
  if (
    typeof prepared.uploadReceipt !== "string" ||
    !prepared.uploadReceipt.includes(".")
  ) {
    throw new Error("release service returned an invalid upload receipt");
  }
  const uploadUrl = new URL(prepared.uploadUrl);
  if (uploadUrl.protocol !== "https:" || !uploadUrl.hostname.endsWith(".aliyuncs.com")) {
    throw new Error("release service returned an invalid OSS upload URL");
  }
}

function isPublicAliyunOssObjectHost(hostname) {
  return (
    /^[a-z0-9](?:[a-z0-9-]{1,61}[a-z0-9])?\.oss-[a-z0-9-]+\.aliyuncs\.com$/.test(
      hostname,
    ) && !hostname.includes("-internal.aliyuncs.com")
  );
}

export function completionPayload(metadata, prepared) {
  return {
    ...metadata,
    objectKey: prepared.objectKey,
    downloadUrl: prepared.downloadUrl,
    uploadReceipt: prepared.uploadReceipt,
  };
}

async function verifyPublicObject(url, expectedSize, expectedSha256, name) {
  const response = await fetch(url, {
    headers: { "Accept-Encoding": "identity" },
    signal: AbortSignal.timeout(15 * 60_000),
  });
  if (!response.ok || !response.body) {
    throw new Error(`anonymous OSS download returned HTTP ${response.status}`);
  }
  const hash = createHash("sha256");
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    hash.update(chunk);
  }
  const actualSha256 = hash.digest("hex");
  if (size !== expectedSize) {
    throw new Error(
      `anonymous OSS download size mismatch for ${name}: expected ${expectedSize}, got ${size}`,
    );
  }
  if (actualSha256 !== expectedSha256) {
    throw new Error(
      `anonymous OSS download checksum mismatch for ${name}: expected ${expectedSha256}, got ${actualSha256}`,
    );
  }
  console.log(`Verified anonymous OSS download for ${name}`);
}

async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

export function contentTypeFor(name) {
  if (name.endsWith(".msi")) return "application/x-msi";
  if (name.endsWith(".dmg")) return "application/x-apple-diskimage";
  if (name.endsWith(".deb")) return "application/vnd.debian.binary-package";
  if (name.endsWith(".AppImage")) return "application/octet-stream";
  if (name.endsWith(".app.tar.gz")) return "application/gzip";
  throw new Error(`Unsupported release asset: ${name}`);
}

function requiredEnv(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`Missing ${name}`);
  return value;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  await main();
}
