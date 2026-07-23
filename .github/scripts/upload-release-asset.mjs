import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { readFile, stat } from "node:fs/promises";
import { basename } from "node:path";

import { uploadMultipartFile } from "./multipart-file-upload.mjs";

const [apiBaseArg, tagName, version, target, filePath, signaturePath] = process.argv.slice(2);

if (!apiBaseArg || !tagName || !version || !target || !filePath) {
  console.error(
    "Usage: upload-release-asset.mjs <apiBase> <tagName> <version> <target> <filePath> [signaturePath]",
  );
  process.exit(2);
}

const releaseServiceToken = requiredEnv("BRIDGE_AGENT_RELEASE_API_TOKEN");
const giteeToken = requiredEnv("GITEE_ACCESS_TOKEN");
const apiBase = apiBaseArg.replace(/^http:\/\//, "https://").replace(/\/+$/, "");
const giteeApiBase = "https://gitee.com/api/v5";
const owner = "zxflimit_admin";
const repository = "bridge-agent";
const maximumAttachmentBytes = 100_000_000;

const assetName = basename(filePath);
if (!isAllowedReleaseAsset(assetName, target)) {
  throw new Error(`Refusing to upload non-release bundle for ${target}: ${assetName}`);
}

const { size } = await stat(filePath);
if (size > maximumAttachmentBytes) {
  throw new Error(
    `${assetName} is ${size} bytes and exceeds Gitee's 100 MB release attachment limit`,
  );
}
const sha256 = await sha256File(filePath);
const contentType = contentTypeFor(assetName);
const signature = signaturePath ? (await readFile(signaturePath, "utf8")).trim() : undefined;
if (signaturePath && !signature) {
  throw new Error(`Updater signature is empty: ${signaturePath}`);
}

const release = await giteeJson(
  `/repos/${owner}/${repository}/releases/tags/${encodeURIComponent(tagName)}`,
);
const releaseId = release.id;
if (!releaseId) {
  throw new Error(`Gitee release ${tagName} has no id`);
}

const filesToUpload = [{ name: assetName, path: filePath, type: contentType }];
if (signaturePath) {
  filesToUpload.push({
    name: basename(signaturePath),
    path: signaturePath,
    type: "text/plain",
  });
}

const attachments = await giteeJson(
  `/repos/${owner}/${repository}/releases/${releaseId}/attach_files`,
  { query: { page: "1", per_page: "100" } },
);
for (const file of filesToUpload) {
  for (const existing of attachments.filter((attachment) => attachment.name === file.name)) {
    await giteeJson(
      `/repos/${owner}/${repository}/releases/${releaseId}/attach_files/${existing.id}`,
      { method: "DELETE" },
    );
  }
}

let uploadedAsset;
for (const file of filesToUpload) {
  const uploaded = await uploadGiteeAttachment(releaseId, file);
  if (file.name === assetName) {
    uploadedAsset = uploaded;
  }
  console.log(`Uploaded ${file.name} to Gitee Release`);
}

const downloadUrl = uploadedAsset?.browser_download_url ?? uploadedAsset?.download_url;
if (!downloadUrl) {
  throw new Error(`Gitee did not return a public download URL for ${assetName}`);
}
validateGiteeDownloadUrl(downloadUrl);
await verifyPublicDownload(downloadUrl, size, assetName);

await postReleaseServiceJson(
  `${apiBase}/releases/${encodeURIComponent(tagName)}/assets/register`,
  {
    tagName,
    version,
    target,
    name: assetName,
    sha256,
    contentType,
    sizeBytes: size,
    provider: "gitee-release",
    externalAssetId: String(uploadedAsset.id),
    downloadUrl,
    signature,
  },
);

console.log(`Registered ${assetName} (${size} bytes, sha256:${sha256})`);

async function uploadGiteeAttachment(releaseId, file) {
  return retry(async () => {
    const response = await uploadMultipartFile({
      url: `${giteeApiBase}/repos/${owner}/${repository}/releases/${releaseId}/attach_files`,
      headers: { Authorization: `Bearer ${giteeToken}` },
      filePath: file.path,
      fileName: file.name,
      contentType: file.type,
    });
    return decodeJsonResponse(response, `upload Gitee attachment ${file.name}`);
  }, `Gitee upload for ${file.name}`, 4);
}

async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

async function giteeJson(path, options = {}) {
  return retry(async () => {
    const url = new URL(`${giteeApiBase}${path}`);
    for (const [name, value] of Object.entries(options.query ?? {})) {
      url.searchParams.set(name, value);
    }
    const { query: _query, ...fetchOptions } = options;
    const response = await fetch(url, {
      ...fetchOptions,
      headers: {
        Authorization: `Bearer ${giteeToken}`,
        ...fetchOptions.headers,
      },
    });
    if (options.method === "DELETE" && response.status === 204) {
      return {};
    }
    return decodeJsonResponse(response, `${options.method ?? "GET"} Gitee ${path}`);
  }, `Gitee API ${path}`);
}

async function postReleaseServiceJson(url, payload) {
  return retry(async () => {
    const response = await fetch(url, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${releaseServiceToken}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(payload),
      redirect: "follow",
    });
    return decodeJsonResponse(response, `POST ${url}`);
  }, `release metadata registration for ${assetName}`);
}

async function decodeJsonResponse(response, label) {
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`${label} failed: ${response.status} ${truncate(body)}`);
  }
  if (!body.trim()) {
    return {};
  }
  try {
    return JSON.parse(body);
  } catch (error) {
    throw new Error(`${label} returned invalid JSON: ${error.message}`);
  }
}

async function verifyPublicDownload(url, expectedSize, name) {
  await retry(async () => {
    const response = await fetch(url, {
      headers: { Range: "bytes=0-0" },
      redirect: "follow",
    });
    try {
      if (response.status !== 200 && response.status !== 206) {
        throw new Error(`anonymous download returned HTTP ${response.status}`);
      }
      const contentRange = response.headers.get("content-range");
      const contentLength = response.headers.get("content-length");
      if (contentRange) {
        const match = contentRange.match(/\/(\d+)$/);
        if (match && Number(match[1]) !== expectedSize) {
          throw new Error(`content-range size ${match[1]} does not match ${expectedSize}`);
        }
      } else if (contentLength && Number(contentLength) !== expectedSize) {
        throw new Error(`content-length ${contentLength} does not match ${expectedSize}`);
      }
    } finally {
      await response.body?.cancel();
    }
  }, `public download verification for ${name}`, 6, 10_000);
  console.log(`Verified anonymous public download for ${name}`);
}

function validateGiteeDownloadUrl(value) {
  const url = new URL(value);
  if (
    url.protocol !== "https:" ||
    url.hostname !== "gitee.com" ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  ) {
    throw new Error(`Gitee returned a non-permanent download URL for ${assetName}`);
  }
  const prefix = `/${owner}/${repository}/`;
  if (!url.pathname.startsWith(prefix)) {
    throw new Error(`Gitee download URL does not belong to ${owner}/${repository}`);
  }
}

async function retry(operation, label, attempts = 3, baseDelayMs = 5_000) {
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await operation();
    } catch (error) {
      lastError = error;
      if (attempt === attempts) {
        break;
      }
      const delayMs = Math.min(attempt * baseDelayMs, 60_000);
      console.warn(`${label} failed on attempt ${attempt}/${attempts}: ${error.message}`);
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
  }
  throw lastError;
}

function requiredEnv(name) {
  const value = process.env[name]?.trim();
  if (!value) {
    throw new Error(`Missing ${name}`);
  }
  return value;
}

function truncate(value, limit = 500) {
  return value.length <= limit ? value : `${value.slice(0, limit)}...`;
}

function contentTypeFor(name) {
  if (name.endsWith(".dmg")) return "application/x-apple-diskimage";
  if (name.endsWith(".app.tar.gz")) return "application/gzip";
  if (name.endsWith(".msi")) return "application/x-msi";
  if (name.endsWith(".AppImage")) return "application/octet-stream";
  if (name.endsWith(".deb")) return "application/vnd.debian.binary-package";
  return "application/octet-stream";
}

function isAllowedReleaseAsset(name, target) {
  if (name.includes("/") || name.includes("\\")) return false;
  if (target === "macOS Universal") {
    return name.endsWith(".dmg") || name.endsWith(".app.tar.gz");
  }
  if (target === "Windows x64") return name.endsWith(".msi");
  if (target === "Linux x64") return name.endsWith(".AppImage") || name.endsWith(".deb");
  return false;
}
