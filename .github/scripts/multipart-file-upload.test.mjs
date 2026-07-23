import { createServer } from "node:http";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { uploadMultipartFile } from "./multipart-file-upload.mjs";

const cleanups = [];

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()));
});

describe("uploadMultipartFile", () => {
  it("streams a multipart file with an exact content length and returns the response", async () => {
    const directory = await mkdtemp(join(tmpdir(), "bridge-agent-upload-test-"));
    cleanups.push(() => rm(directory, { recursive: true, force: true }));
    const filePath = join(directory, "Baijimu_0.1.111_universal.dmg");
    const fileBytes = Buffer.from("signed desktop release bundle");
    await writeFile(filePath, fileBytes);

    let received;
    const server = createServer((request, response) => {
      const chunks = [];
      request.on("data", (chunk) => chunks.push(chunk));
      request.on("end", () => {
        received = {
          authorization: request.headers.authorization,
          contentLength: request.headers["content-length"],
          contentType: request.headers["content-type"],
          body: Buffer.concat(chunks),
        };
        response.writeHead(201, { "content-type": "application/json" });
        response.end(JSON.stringify({ id: 42, browser_download_url: "https://example.test" }));
      });
    });
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    cleanups.push(() => new Promise((resolve) => server.close(resolve)));
    const address = server.address();

    const response = await uploadMultipartFile({
      url: `http://127.0.0.1:${address.port}/release-assets`,
      headers: { Authorization: "Bearer test-token" },
      filePath,
      fileName: "Baijimu_0.1.111_universal.dmg",
      contentType: "application/x-apple-diskimage",
      inactivityTimeoutMs: 5_000,
    });

    expect(response.status).toBe(201);
    expect(JSON.parse(await response.text())).toMatchObject({ id: 42 });
    expect(received.authorization).toBe("Bearer test-token");
    expect(Number(received.contentLength)).toBe(received.body.length);
    expect(received.contentType).toMatch(/^multipart\/form-data; boundary=/);
    expect(received.body.includes(fileBytes)).toBe(true);
    expect(received.body.toString("utf8")).toContain(
      'filename="Baijimu_0.1.111_universal.dmg"',
    );
  });

  it("rejects unsafe multipart file names", async () => {
    await expect(
      uploadMultipartFile({
        url: "https://gitee.com/api/v5/releases/assets",
        filePath: "/unused",
        fileName: 'bad"name.dmg',
        contentType: "application/octet-stream",
      }),
    ).rejects.toThrow("Unsafe multipart file name");
  });
});
