import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { validateBundledSkill, verifyUpstreamSkill } from "./check-bundled-platform-skill.mjs";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
function fixture() {
  const contents = Buffer.from("---\nname: baijimu-platform\ndescription: fixture\n---\nhttps://docs.baijimu.com/\n");
  const archive = Buffer.from("published fixture archive");
  const provenance = {
    repository: "https://github.com/momoplan/baijimu-platform-skill",
    release: "v7.3.2",
    skillSha256: hash(contents),
    archiveSha256: hash(archive),
  };
  const checksum = Buffer.from(`${hash(archive)}  baijimu-platform.zip\n`);
  const fetcher = async (url) => new Response(url.endsWith("SKILL.md") ? contents : url.endsWith(".sha256") ? checksum : archive);
  return { contents, provenance, fetcher };
}

describe("bundled platform skill provenance", () => {
  it("verifies the pinned source, archive and published checksum", async () => {
    const { contents, provenance, fetcher } = fixture();
    await expect(verifyUpstreamSkill(contents, provenance, fetcher)).resolves.toBeUndefined();
  });

  it("rejects altered local bytes and mutable or unofficial sources", () => {
    const { contents, provenance } = fixture();
    expect(() => validateBundledSkill(Buffer.concat([contents, Buffer.from("changed")]), provenance)).toThrow("checksum mismatch");
    expect(() => validateBundledSkill(contents, { ...provenance, release: "latest" })).toThrow("official stable release");
    expect(() => validateBundledSkill(contents, { ...provenance, repository: "https://example.com/skill" })).toThrow("official stable release");
  });

  it("rejects a separately edited client copy even if its local hash was updated", async () => {
    const { contents, provenance, fetcher } = fixture();
    const edited = Buffer.concat([contents, Buffer.from("independent client instructions")]);
    await expect(verifyUpstreamSkill(edited, { ...provenance, skillSha256: hash(edited) }, fetcher)).rejects.toThrow("pinned upstream source");
  });

  it("rejects archive corruption and a provenance hash that differs from the release checksum", async () => {
    const { contents, provenance, fetcher } = fixture();
    const altered = Buffer.from("altered archive");
    const alteredFetcher = async (url) => url.endsWith(".zip") ? new Response(altered) : fetcher(url);
    await expect(verifyUpstreamSkill(contents, provenance, alteredFetcher)).rejects.toThrow("archive checksum mismatch");
    await expect(verifyUpstreamSkill(contents, { ...provenance, archiveSha256: hash(altered) }, alteredFetcher)).rejects.toThrow("published release checksum");
  });

  it("fails explicitly when the upstream release cannot be read", async () => {
    const { contents, provenance } = fixture();
    await expect(verifyUpstreamSkill(contents, provenance, async () => new Response(null, { status: 404 }))).rejects.toThrow("HTTP 404");
  });
});
