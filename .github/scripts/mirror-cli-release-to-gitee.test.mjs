import { createHash } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import { mirrorCliReleaseToGitee } from "./mirror-cli-release-to-gitee.mjs";

const cleanups = [];
const version = "0.1.17";
const tagName = `baijimu-cli-v${version}`;
const targetCommitish = "a".repeat(40);
const release = {
  id: 117,
  tag_name: tagName,
  created_at: "2026-07-24T14:00:00Z",
};

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()));
});

describe("mirrorCliReleaseToGitee", () => {
  it("creates the CLI release, uploads all artifacts and verifies anonymous bytes", async () => {
    const { directory, files } = await createReleaseAssets();
    const uploads = [];
    const releaseRequests = [];
    const fetchImpl = vi.fn(async (input, options = {}) => {
      const url = new URL(input);
      if (url.hostname === "gitee.com" && url.pathname.includes("/releases/download/")) {
        const name = decodeURIComponent(url.pathname.split("/").at(-1));
        return new Response(files.get(name), { status: 200 });
      }
      if (url.pathname.endsWith(`/releases/tags/${tagName}`)) {
        return jsonResponse(null);
      }
      if (url.pathname.endsWith(`/baijimu-cli-rs/releases/tags/v${version}`)) {
        return jsonResponse(null);
      }
      if (url.pathname.endsWith("/releases") && options.method === "POST") {
        releaseRequests.push(JSON.parse(options.body));
        return jsonResponse(release, 201);
      }
      if (url.pathname.endsWith("/releases") && !options.method) {
        return jsonResponse([release]);
      }
      if (url.pathname.endsWith(`/${release.id}/attach_files`)) {
        return jsonResponse([]);
      }
      throw new Error(`Unexpected fetch: ${options.method ?? "GET"} ${url}`);
    });
    const uploadImpl = vi.fn(async ({ fileName }) => {
      uploads.push(fileName);
      return jsonResponse(
        {
          id: uploads.length,
          browser_download_url:
            `https://gitee.com/zxflimit_admin/bridge-agent/releases/download/${tagName}/` +
            encodeURIComponent(fileName),
        },
        201,
      );
    });

    const result = await mirrorCliReleaseToGitee({
      tagName,
      version,
      targetCommitish,
      assetsDirectory: directory,
      token: "test-token",
      fetchImpl,
      uploadImpl,
      logger: { log: vi.fn(), warn: vi.fn() },
      sleepImpl: vi.fn(),
    });

    expect(releaseRequests).toEqual([
      expect.objectContaining({
        tag_name: tagName,
        target_commitish: targetCommitish,
        name: `Baijimu CLI ${version}`,
      }),
    ]);
    expect(uploads).toHaveLength(6);
    expect(uploads).toEqual(
      expect.arrayContaining([
        `baijimu-cli-${version}-macos-universal.zip`,
        `baijimu-cli-${version}-windows-x64.zip`,
        `baijimu-cli-${version}-linux-x64.zip`,
        `baijimu-cli-${version}-macos-universal.zip.sha256`,
        `baijimu-cli-${version}-windows-x64.zip.sha256`,
        `baijimu-cli-${version}-linux-x64.zip.sha256`,
      ]),
    );
    expect(result).toMatchObject({
      releaseId: String(release.id),
      tagName,
      version,
      assets: [
        { platform: "macos-universal" },
        { platform: "windows-x64" },
        { platform: "linux-x64" },
      ],
    });
  });

  it("rejects a mismatched checksum before touching Gitee", async () => {
    const { directory } = await createReleaseAssets();
    const name = `baijimu-cli-${version}-windows-x64.zip`;
    await writeFile(join(directory, `${name}.sha256`), `${"0".repeat(64)}  ${name}\n`);
    const fetchImpl = vi.fn();

    await expect(
      mirrorCliReleaseToGitee({
        tagName,
        version,
        targetCommitish,
        assetsDirectory: directory,
        token: "test-token",
        fetchImpl,
        uploadImpl: vi.fn(),
      }),
    ).rejects.toThrow("checksum mismatch before domestic mirroring");
    expect(fetchImpl).not.toHaveBeenCalled();
  });

  it("reuses matching immutable attachments without changing their URLs", async () => {
    const { directory, files } = await createReleaseAssets();
    const attachments = [...files.keys()].map((name, index) => ({
      id: index + 1,
      name,
      browser_download_url:
        `https://gitee.com/zxflimit_admin/bridge-agent/releases/download/${tagName}/` +
        encodeURIComponent(name),
    }));
    const deletedReleases = [];
    const fetchImpl = vi.fn(async (input, options = {}) => {
      const url = new URL(input);
      if (url.hostname === "gitee.com" && url.pathname.includes("/releases/download/")) {
        const name = decodeURIComponent(url.pathname.split("/").at(-1));
        return new Response(files.get(name), { status: 200 });
      }
      if (url.pathname.endsWith(`/releases/tags/${tagName}`)) {
        return jsonResponse(release);
      }
      if (url.pathname.endsWith(`/baijimu-cli-rs/releases/tags/v${version}`)) {
        return jsonResponse({
          id: 999,
          tag_name: `v${version}`,
          name: `Baijimu CLI ${version}`,
        });
      }
      if (
        url.pathname.endsWith("/baijimu-cli-rs/releases/999") &&
        options.method === "DELETE"
      ) {
        deletedReleases.push(999);
        return new Response(null, { status: 204 });
      }
      if (url.pathname.endsWith("/releases")) {
        return jsonResponse([release]);
      }
      if (url.pathname.endsWith(`/${release.id}/attach_files`)) {
        return jsonResponse(attachments);
      }
      throw new Error(`Unexpected fetch: ${url}`);
    });
    const uploadImpl = vi.fn();

    const result = await mirrorCliReleaseToGitee({
      tagName,
      version,
      targetCommitish,
      assetsDirectory: directory,
      token: "test-token",
      fetchImpl,
      uploadImpl,
      logger: { log: vi.fn(), warn: vi.fn() },
      sleepImpl: vi.fn(),
    });

    expect(uploadImpl).not.toHaveBeenCalled();
    expect(result.assets[0].downloadUrl).toBe(attachments[0].browser_download_url);
    expect(deletedReleases).toEqual([999]);
  });

  it("replaces a mismatched attachment left by an incomplete recovery", async () => {
    const { directory, files } = await createReleaseAssets();
    const name = `baijimu-cli-${version}-macos-universal.zip`;
    const staleAttachment = {
      id: 77,
      name,
      browser_download_url:
        `https://gitee.com/zxflimit_admin/bridge-agent/releases/download/${tagName}/` +
        encodeURIComponent(name),
    };
    let stale = true;
    const deletedAttachments = [];
    const fetchImpl = vi.fn(async (input, options = {}) => {
      const url = new URL(input);
      if (url.pathname.includes("/releases/download/")) {
        const requestedName = decodeURIComponent(url.pathname.split("/").at(-1));
        if (requestedName === name && stale) {
          return new Response(Buffer.from("stale ZIP bytes"), { status: 200 });
        }
        return new Response(files.get(requestedName), { status: 200 });
      }
      if (url.pathname.endsWith(`/releases/tags/${tagName}`)) {
        return jsonResponse(release);
      }
      if (url.pathname.endsWith(`/baijimu-cli-rs/releases/tags/v${version}`)) {
        return jsonResponse(null);
      }
      if (
        url.pathname.endsWith(`/${release.id}/attach_files/${staleAttachment.id}`) &&
        options.method === "DELETE"
      ) {
        stale = false;
        deletedAttachments.push(staleAttachment.id);
        return new Response(null, { status: 204 });
      }
      if (url.pathname.endsWith("/releases")) {
        return jsonResponse([release]);
      }
      if (url.pathname.endsWith(`/${release.id}/attach_files`)) {
        return jsonResponse([staleAttachment]);
      }
      throw new Error(`Unexpected fetch: ${options.method ?? "GET"} ${url}`);
    });
    const uploadedNames = [];
    const uploadImpl = vi.fn(async ({ fileName }) => {
      uploadedNames.push(fileName);
      return jsonResponse(
        {
          id: 100 + uploadedNames.length,
          browser_download_url:
            `https://gitee.com/zxflimit_admin/bridge-agent/releases/download/${tagName}/` +
            encodeURIComponent(fileName),
        },
        201,
      );
    });

    await mirrorCliReleaseToGitee({
      tagName,
      version,
      targetCommitish,
      assetsDirectory: directory,
      token: "test-token",
      fetchImpl,
      uploadImpl,
      logger: { log: vi.fn(), warn: vi.fn() },
      sleepImpl: vi.fn(),
    });

    expect(deletedAttachments).toEqual([staleAttachment.id]);
    expect(uploadedNames).toContain(name);
  });
});

async function createReleaseAssets() {
  const directory = await mkdtemp(join(tmpdir(), "baijimu-cli-release-test-"));
  cleanups.push(() => rm(directory, { recursive: true, force: true }));
  const files = new Map();
  for (const platform of ["macos-universal", "windows-x64", "linux-x64"]) {
    const name = `baijimu-cli-${version}-${platform}.zip`;
    const bytes = Buffer.from(`signed CLI release for ${platform}`);
    const checksum = createHash("sha256").update(bytes).digest("hex");
    const checksumName = `${name}.sha256`;
    const checksumBytes = Buffer.from(`${checksum}  ${name}\n`);
    await writeFile(join(directory, name), bytes);
    await writeFile(join(directory, checksumName), checksumBytes);
    files.set(name, bytes);
    files.set(checksumName, checksumBytes);
  }
  return { directory, files };
}

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}
