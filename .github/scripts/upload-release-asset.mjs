import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";
import { basename } from "node:path";

const [apiBaseArg, tagName, version, target, filePath] = process.argv.slice(2);

if (!apiBaseArg || !tagName || !version || !target || !filePath) {
  console.error(
    "Usage: upload-release-asset.mjs <apiBase> <tagName> <version> <target> <filePath>",
  );
  process.exit(2);
}

const token = process.env.BRIDGE_AGENT_RELEASE_API_TOKEN;
if (!token) {
  console.error("Missing BRIDGE_AGENT_RELEASE_API_TOKEN");
  process.exit(2);
}

const apiBase = apiBaseArg.replace(/\/+$/, "");
const assetName = basename(filePath);
if (!isAllowedReleaseAsset(assetName, target)) {
  throw new Error(`Refusing to upload non-release bundle for ${target}: ${assetName}`);
}
const bytes = await readFile(filePath);
const { size } = await stat(filePath);
const sha256 = createHash("sha256").update(bytes).digest("hex");
const contentType = contentTypeFor(assetName);

const prepare = await postJson(`${apiBase}/releases/${encodeURIComponent(tagName)}/assets/prepare`, {
  tagName,
  version,
  target,
  name: assetName,
  sha256,
  contentType,
  sizeBytes: size,
});

const uploadUrl = prepare.uploadUrl ?? prepare.upload_url;
if (!uploadUrl) {
  throw new Error("Release service did not return uploadUrl");
}

const uploadMethod = prepare.method ?? "PUT";
const uploadHeaders = normalizeHeaders(prepare.headers);
if (!hasHeader(uploadHeaders, "content-type")) {
  uploadHeaders["content-type"] = contentType;
}

const uploadResponse = await fetch(uploadUrl, {
  method: uploadMethod,
  headers: uploadHeaders,
  body: bytes,
});
if (!uploadResponse.ok) {
  throw new Error(
    `OSS upload failed for ${assetName}: ${uploadResponse.status} ${await uploadResponse.text()}`,
  );
}

await postJson(`${apiBase}/releases/${encodeURIComponent(tagName)}/assets/complete`, {
  tagName,
  version,
  target,
  name: assetName,
  sha256,
  contentType,
  sizeBytes: size,
  objectKey: prepare.objectKey ?? prepare.object_key,
  downloadUrl: prepare.resourceUrl ?? prepare.resource_url ?? prepare.downloadUrl ?? prepare.download_url,
});

console.log(`Uploaded ${assetName} (${size} bytes, sha256:${sha256})`);

async function postJson(url, payload) {
  const response = await fetch(url, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(payload),
  });
  if (!response.ok) {
    throw new Error(`${url} failed: ${response.status} ${await response.text()}`);
  }
  return response.json();
}

function normalizeHeaders(headers) {
  if (!headers || typeof headers !== "object") {
    return {};
  }
  return Object.fromEntries(
    Object.entries(headers)
      .filter(([, value]) => value !== undefined && value !== null)
      .map(([key, value]) => [key, String(value)]),
  );
}

function hasHeader(headers, expectedName) {
  return Object.keys(headers).some((name) => name.toLowerCase() === expectedName);
}

function contentTypeFor(name) {
  if (name.endsWith(".dmg")) {
    return "application/x-apple-diskimage";
  }
  if (name.endsWith(".msi")) {
    return "application/x-msi";
  }
  if (name.endsWith(".exe")) {
    return "application/vnd.microsoft.portable-executable";
  }
  if (name.endsWith(".AppImage")) {
    return "application/octet-stream";
  }
  if (name.endsWith(".deb")) {
    return "application/vnd.debian.binary-package";
  }
  return "application/octet-stream";
}

function isAllowedReleaseAsset(name, target) {
  if (name.includes("/") || name.includes("\\")) {
    return false;
  }
  if (target === "macOS Universal") {
    return name.endsWith(".dmg");
  }
  if (target === "Windows x64") {
    return name.endsWith(".msi");
  }
  if (target === "Linux x64") {
    return name.endsWith(".AppImage") || name.endsWith(".deb");
  }
  return false;
}
