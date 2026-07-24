import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { readFile, stat } from "node:fs/promises";
import { basename, join } from "node:path";
import { pathToFileURL } from "node:url";

import { uploadMultipartFile } from "./multipart-file-upload.mjs";

const defaultApiBase = "https://gitee.com/api/v5";
const defaultOwner = "zxflimit_admin";
const defaultRepository = "bridge-agent";
const maximumAttachmentBytes = 100_000_000;
const platformNames = ["macos-universal", "windows-x64", "linux-x64"];

export async function mirrorCliReleaseToGitee({
  tagName,
  version,
  targetCommitish,
  assetsDirectory,
  token,
  apiBase = defaultApiBase,
  owner = defaultOwner,
  repository = defaultRepository,
  retainedReleaseCount = 5,
  fetchImpl = fetch,
  uploadImpl = uploadMultipartFile,
  logger = console,
  sleepImpl = sleep,
}) {
  validateReleaseIdentity({ tagName, version, targetCommitish, assetsDirectory, token });

  const assets = [];
  for (const platform of platformNames) {
    const name = `baijimu-cli-${version}-${platform}.zip`;
    const filePath = join(assetsDirectory, name);
    const checksumPath = `${filePath}.sha256`;
    const checksumText = await readFile(checksumPath, "utf8");
    const checksum = parseChecksumFile(checksumText, name);
    const actualChecksum = await sha256File(filePath);
    if (actualChecksum !== checksum) {
      throw new Error(
        `${name} checksum mismatch before domestic mirroring: expected ${checksum}, got ${actualChecksum}`,
      );
    }
    const { size } = await stat(filePath);
    if (size <= 0 || size > maximumAttachmentBytes) {
      throw new Error(
        `${name} is ${size} bytes; Gitee release attachments must be between 1 and ${maximumAttachmentBytes} bytes`,
      );
    }
    assets.push({
      name,
      filePath,
      checksum,
      size,
      checksumName: basename(checksumPath),
      checksumPath,
      checksumText,
    });
  }

  const request = createRequester({ apiBase, token: token.trim(), fetchImpl, sleepImpl });
  const release = await prepareRelease({
    request,
    tagName,
    version,
    targetCommitish,
    owner,
    repository,
    retainedReleaseCount,
    logger,
  });
  const attachments = await request(
    `/repos/${owner}/${repository}/releases/${release.id}/attach_files?page=1&per_page=100`,
  );
  if (!Array.isArray(attachments)) {
    throw new Error(`Gitee attachments response for ${tagName} is not an array`);
  }

  const mirrored = [];
  for (const asset of assets) {
    const files = [
      {
        name: asset.name,
        path: asset.filePath,
        contentType: "application/zip",
        expectedSize: asset.size,
        expectedSha256: asset.checksum,
      },
      {
        name: asset.checksumName,
        path: asset.checksumPath,
        contentType: "text/plain",
        expectedBytes: Buffer.from(asset.checksumText),
      },
    ];
    for (const file of files) {
      const existing = attachments.filter((item) => item.name === file.name);
      if (existing.length > 1) {
        throw new Error(
          `Gitee CLI Release ${tagName} contains duplicate immutable attachment ${file.name}`,
        );
      }

      let reusable = existing[0];
      if (reusable) {
        const downloadUrl = reusable.browser_download_url ?? reusable.download_url;
        validateDownloadUrl(downloadUrl, owner, repository, tagName, file.name);
        try {
          await verifyAnonymousDownload({
            url: downloadUrl,
            file,
            fetchImpl,
            sleepImpl,
            logger,
          });
          logger.log(`Reused and verified immutable ${file.name}`);
        } catch (error) {
          if (error.retryable !== false) throw error;
          await request(
            `/repos/${owner}/${repository}/releases/${release.id}/attach_files/${reusable.id}`,
            { method: "DELETE" },
          );
          logger.warn?.(
            `Deleted incomplete mismatched Gitee attachment ${file.name}: ${error.message}`,
          );
          reusable = null;
        }
      }

      const uploaded =
        reusable ??
        (await retry(
          async () => {
            const response = await uploadImpl({
              url: `${apiBase}/repos/${owner}/${repository}/releases/${release.id}/attach_files`,
              headers: { Authorization: `Bearer ${token.trim()}` },
              filePath: file.path,
              fileName: file.name,
              contentType: file.contentType,
            });
            return decodeJsonResponse(response, `upload Gitee attachment ${file.name}`);
          },
          `Gitee upload for ${file.name}`,
          4,
          5_000,
          sleepImpl,
          logger,
        ));
      const downloadUrl = uploaded.browser_download_url ?? uploaded.download_url;
      validateDownloadUrl(downloadUrl, owner, repository, tagName, file.name);
      if (!reusable) {
        await verifyAnonymousDownload({
          url: downloadUrl,
          file,
          fetchImpl,
          sleepImpl,
          logger,
        });
        logger.log(`Mirrored and verified ${file.name}`);
      }

      if (file.name === asset.name) {
        mirrored.push({
          platform: platformForAsset(asset.name),
          name: asset.name,
          sha256: asset.checksum,
          sizeBytes: asset.size,
          downloadUrl,
          externalAssetId: String(uploaded.id),
        });
      }
    }
  }

  await removeLegacyPrivateRelease({ request, owner, version, logger });
  return { releaseId: String(release.id), tagName, version, assets: mirrored };
}

async function removeLegacyPrivateRelease({ request, owner, version, logger }) {
  const repository = "baijimu-cli-rs";
  const tagName = `v${version}`;
  let release;
  try {
    release = await request(
      `/repos/${owner}/${repository}/releases/tags/${encodeURIComponent(tagName)}`,
    );
  } catch (error) {
    if (error.status === 404) return;
    throw error;
  }
  if (release === null) return;
  if (
    !release?.id ||
    release.tag_name !== tagName ||
    release.name !== `Baijimu CLI ${version}`
  ) {
    throw new Error(
      `Refusing to delete unrecognized private CLI release metadata for ${tagName}`,
    );
  }
  await request(`/repos/${owner}/${repository}/releases/${release.id}`, {
    method: "DELETE",
  });
  logger.log(`Deleted legacy private CLI Release ${tagName}; source tag was preserved`);
}

async function prepareRelease({
  request,
  tagName,
  version,
  targetCommitish,
  owner,
  repository,
  retainedReleaseCount,
  logger,
}) {
  let currentRelease;
  try {
    currentRelease = await request(
      `/repos/${owner}/${repository}/releases/tags/${encodeURIComponent(tagName)}`,
    );
  } catch (error) {
    if (error.status !== 404) throw error;
    currentRelease = null;
  }
  if (currentRelease === null) {
    currentRelease = await request(`/repos/${owner}/${repository}/releases`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        tag_name: tagName,
        target_commitish: targetCommitish,
        name: `Baijimu CLI ${version}`,
        body: [
          "Baijimu CLI 国内镜像发布。",
          "",
          "制品由 GitHub Actions 构建、签名并同步到此 Gitee Release；GitHub Release 保留完整历史。",
        ].join("\n"),
        prerelease: false,
      }),
    });
    logger.log(`Created Gitee CLI Release ${tagName}`);
  }
  if (!currentRelease?.id) {
    throw new Error(`Gitee CLI Release ${tagName} has no id`);
  }

  const releases = await request(`/repos/${owner}/${repository}/releases?page=1&per_page=100`);
  if (!Array.isArray(releases)) {
    throw new Error("Gitee CLI releases response is not an array");
  }
  const managed = releases
    .filter((release) => /^baijimu-cli-v\d+\.\d+\.\d+$/.test(release.tag_name ?? ""))
    .sort((left, right) => releaseTime(right) - releaseTime(left));
  const keepIds = new Set(
    [currentRelease, ...managed.filter((release) => release.id !== currentRelease.id)]
      .slice(0, retainedReleaseCount)
      .map((release) => release.id),
  );
  for (const release of managed) {
    if (!keepIds.has(release.id)) {
      await request(`/repos/${owner}/${repository}/releases/${release.id}`, {
        method: "DELETE",
      });
      logger.log(`Deleted old Gitee CLI Release ${release.tag_name}; Git tag was preserved`);
    }
  }
  return currentRelease;
}

function createRequester({ apiBase, token, fetchImpl, sleepImpl }) {
  return async function request(path, options = {}) {
    return retry(
      async () => {
        const url = new URL(`${apiBase}${path}`);
        const response = await fetchImpl(url, {
          ...options,
          headers: {
            Authorization: `Bearer ${token}`,
            ...options.headers,
          },
        });
        if (options.method === "DELETE" && response.status === 204) {
          return {};
        }
        return decodeJsonResponse(
          response,
          `Gitee API ${options.method ?? "GET"} ${url.pathname}`,
        );
      },
      `Gitee API ${options.method ?? "GET"} ${path}`,
      4,
      5_000,
      sleepImpl,
      console,
      (error) => error.status !== 404,
    );
  };
}

async function verifyAnonymousDownload({ url, file, fetchImpl, sleepImpl, logger }) {
  await retry(
    async () => {
      const response = await fetchImpl(url, { redirect: "follow" });
      if (!response.ok) {
        const error = new Error(`anonymous download returned HTTP ${response.status}`);
        error.status = response.status;
        throw error;
      }
      const bytes = Buffer.from(await response.arrayBuffer());
      if (file.expectedBytes) {
        if (!bytes.equals(file.expectedBytes)) {
          throw deterministicMismatch(
            `anonymous checksum attachment content mismatch for ${file.name}`,
          );
        }
        return;
      }
      if (bytes.length !== file.expectedSize) {
        throw deterministicMismatch(
          `anonymous download size ${bytes.length} does not match ${file.expectedSize} for ${file.name}`,
        );
      }
      const checksum = createHash("sha256").update(bytes).digest("hex");
      if (checksum !== file.expectedSha256) {
        throw deterministicMismatch(
          `anonymous download checksum mismatch for ${file.name}: expected ${file.expectedSha256}, got ${checksum}`,
        );
      }
    },
    `anonymous Gitee download verification for ${file.name}`,
    6,
    10_000,
    sleepImpl,
    logger,
    (error) => error.retryable !== false,
  );
}

function deterministicMismatch(message) {
  const error = new Error(message);
  error.retryable = false;
  return error;
}

async function decodeJsonResponse(response, label) {
  const body = await response.text();
  if (!response.ok) {
    const error = new Error(`${label} failed: ${response.status} ${truncate(body)}`);
    error.status = response.status;
    throw error;
  }
  if (!body.trim()) return {};
  try {
    return JSON.parse(body);
  } catch (error) {
    throw new Error(`${label} returned invalid JSON: ${error.message}`);
  }
}

async function retry(
  operation,
  label,
  attempts,
  baseDelayMs,
  sleepImpl,
  logger,
  shouldRetry = () => true,
) {
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await operation();
    } catch (error) {
      lastError = error;
      if (attempt === attempts || !shouldRetry(error)) break;
      logger.warn?.(`${label} failed on attempt ${attempt}/${attempts}: ${error.message}`);
      await sleepImpl(Math.min(attempt * baseDelayMs, 60_000));
    }
  }
  throw lastError;
}

function validateReleaseIdentity({ tagName, version, targetCommitish, assetsDirectory, token }) {
  if (
    !/^\d+\.\d+\.\d+$/.test(version ?? "") ||
    tagName !== `baijimu-cli-v${version}`
  ) {
    throw new Error("CLI Gitee release tag must be baijimu-cli-v<semantic-version>");
  }
  if (!/^[0-9a-f]{40}$/.test(targetCommitish ?? "")) {
    throw new Error("CLI target commit must be an exact 40-character commit");
  }
  if (!assetsDirectory) throw new Error("CLI release assets directory is required");
  if (!token?.trim()) throw new Error("Missing GITEE_ACCESS_TOKEN");
}

function parseChecksumFile(value, expectedName) {
  const match = value.trim().match(/^([0-9a-fA-F]{64})  ([^/\\]+)$/);
  if (!match || match[2] !== expectedName) {
    throw new Error(`Invalid SHA-256 file for ${expectedName}`);
  }
  return match[1].toLowerCase();
}

function validateDownloadUrl(value, owner, repository, tagName, expectedName) {
  const url = new URL(value);
  const encodedName = encodeURIComponent(expectedName);
  const isReleaseDownload =
    url.pathname ===
    `/${owner}/${repository}/releases/download/${encodeURIComponent(tagName)}/${encodedName}`;
  const isLegacyAttachmentDownload =
    url.pathname.startsWith(`/${owner}/${repository}/attach_files/`) &&
    url.pathname.endsWith(`/download/${encodedName}`);
  if (
    url.protocol !== "https:" ||
    url.hostname !== "gitee.com" ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    (!isReleaseDownload && !isLegacyAttachmentDownload)
  ) {
    throw new Error(`Gitee returned an invalid permanent download URL for ${expectedName}`);
  }
}

function platformForAsset(name) {
  return platformNames.find((platform) => name.endsWith(`-${platform}.zip`));
}

async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

function releaseTime(release) {
  return Date.parse(release.created_at ?? release.published_at ?? 0) || 0;
}

function truncate(value, limit = 500) {
  return value.length <= limit ? value : `${value.slice(0, limit)}...`;
}

function sleep(delayMs) {
  return new Promise((resolve) => setTimeout(resolve, delayMs));
}

async function main() {
  const [tagName, version, targetCommitish, assetsDirectory] = process.argv.slice(2);
  const result = await mirrorCliReleaseToGitee({
    tagName,
    version,
    targetCommitish,
    assetsDirectory,
    token: process.env.GITEE_ACCESS_TOKEN,
  });
  console.log(JSON.stringify(result));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
