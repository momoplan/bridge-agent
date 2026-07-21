import { describe, expect, it, vi } from "vitest";

import { prepareGiteeRelease } from "./prepare-gitee-release.mjs";

const release = {
  id: 110,
  tag_name: "bridge-agent-v0.1.110",
  created_at: "2026-07-21T18:19:00Z",
};

describe("prepareGiteeRelease", () => {
  it("creates a release when Gitee represents a missing tag as HTTP 200 null", async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(jsonResponse(null))
      .mockResolvedValueOnce(jsonResponse(release, 201))
      .mockResolvedValueOnce(jsonResponse([release]));
    const logger = { log: vi.fn() };

    await expect(
      prepareGiteeRelease({
        tagName: release.tag_name,
        version: "0.1.110",
        token: "test-token",
        fetchImpl,
        logger,
      }),
    ).resolves.toEqual(release);

    expect(fetchImpl).toHaveBeenCalledTimes(3);
    expect(fetchImpl.mock.calls[1][1]).toMatchObject({ method: "POST" });
    expect(JSON.parse(fetchImpl.mock.calls[1][1].body)).toMatchObject({
      tag_name: release.tag_name,
      name: `百积木 ${release.tag_name}`,
    });
    expect(logger.log).toHaveBeenCalledWith(`Created Gitee Release ${release.tag_name}`);
  });

  it("still creates a release when the missing tag endpoint returns HTTP 404", async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(jsonResponse({ message: "Not Found" }, 404))
      .mockResolvedValueOnce(jsonResponse(release, 201))
      .mockResolvedValueOnce(jsonResponse([release]));

    await expect(
      prepareGiteeRelease({
        tagName: release.tag_name,
        version: "0.1.110",
        token: "test-token",
        fetchImpl,
        logger: { log: vi.fn() },
      }),
    ).resolves.toEqual(release);

    expect(fetchImpl).toHaveBeenCalledTimes(3);
    expect(fetchImpl.mock.calls[1][1]).toMatchObject({ method: "POST" });
  });
});

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}
