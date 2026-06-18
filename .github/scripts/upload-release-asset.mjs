import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";
import http from "node:http";
import https from "node:https";
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

const apiBase = apiBaseArg.replace(/^http:\/\//, "https://").replace(/\/+$/, "");
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

const uploadResponse = await retry(
  () => uploadBinary(uploadUrl, uploadMethod, uploadHeaders, bytes),
  `OSS upload for ${assetName}`,
);
if (uploadResponse.status < 200 || uploadResponse.status >= 300) {
  throw new Error(`OSS upload failed for ${assetName}: ${uploadResponse.status} ${uploadResponse.body}`);
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
  const body = Buffer.from(JSON.stringify(payload));
  const response = await retry(
    () => requestJson(url, "POST", {
      Authorization: `Bearer ${token}`,
      "content-type": "application/json",
      "content-length": body.length,
    }, body),
    `POST ${url}`,
  );
  if (response.status < 200 || response.status >= 300) {
    throw new Error(`${url} failed: ${response.status} ${response.body}`);
  }
  return JSON.parse(response.body);
}

async function retry(operation, label, attempts = 3) {
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await operation();
    } catch (error) {
      lastError = error;
      if (attempt === attempts) {
        break;
      }
      const delayMs = attempt * 5000;
      console.warn(`${label} failed on attempt ${attempt}/${attempts}: ${error.message}`);
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
  }
  throw lastError;
}

async function uploadBinary(url, method, headers, body) {
  return requestRaw(url, method, {
    ...headers,
    "content-length": body.length,
  }, body, 20 * 60 * 1000);
}

async function requestJson(url, method, headers, body) {
  return requestRaw(url, method, headers, body, 60 * 1000);
}

async function requestRaw(url, method, headers, body, timeoutMs) {
  const parsed = new URL(url);
  const client = parsed.protocol === "http:" ? http : https;
  return new Promise((resolve, reject) => {
    const request = client.request(
      parsed,
      {
        method,
        headers,
      },
      (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () => {
          resolve({
            status: response.statusCode ?? 0,
            body: Buffer.concat(chunks).toString("utf8"),
          });
        });
      },
    );

    request.setTimeout(timeoutMs, () => {
      request.destroy(new Error(`${method} ${url} timed out after ${Math.round(timeoutMs / 1000)}s`));
    });
    request.on("error", reject);
    request.end(body);
  });
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
