import { describe, expect, test } from "vitest";

import {
  completionPayload,
  contentTypeFor,
  transferTimeoutMsForSize,
  validatePrepareResponse,
} from "./upload-oss-release-asset.mjs";

const objectKey =
  "lowcode/direct-uploads/bridge-agent-release/20260729/anonymous/123-Baijimu_0.2.11_amd64.AppImage";

describe("OSS release asset contract", () => {
  test("accepts only the canonical immutable object and signed receipt", () => {
    expect(() =>
      validatePrepareResponse(
        {
          uploadUrl:
            "https://baijimu-lowcode-public-20260420.oss-cn-beijing-internal.aliyuncs.com/object?Expires=1&Signature=x",
          objectKey,
          downloadUrl: `https://baijimu-lowcode-public-20260420.oss-cn-beijing.aliyuncs.com/${objectKey}`,
          uploadReceipt: "payload.signature",
        },
        "Baijimu_0.2.11_amd64.AppImage",
      ),
    ).not.toThrow();

    expect(() =>
      validatePrepareResponse(
        {
          uploadUrl:
            "https://baijimu-lowcode-public-20260420.oss-cn-beijing-internal.aliyuncs.com/object?Expires=1&Signature=x",
          objectKey,
          downloadUrl: `https://example.com/${objectKey}`,
          uploadReceipt: "payload.signature",
        },
        "Baijimu_0.2.11_amd64.AppImage",
      ),
    ).toThrow(/non-canonical public OSS/);

    expect(() =>
      validatePrepareResponse(
        {
          uploadUrl:
            "https://baijimu-lowcode-public-20260420.oss-cn-beijing-internal.aliyuncs.com/object?Expires=1&Signature=x",
          objectKey,
          downloadUrl: `https://baijimu-lowcode-public-20260420.oss-cn-beijing-internal.aliyuncs.com/${objectKey}`,
          uploadReceipt: "payload.signature",
        },
        "Baijimu_0.2.11_amd64.AppImage",
      ),
    ).toThrow(/non-canonical public OSS/);
  });

  test("maps every supported release bundle to a stable content type", () => {
    expect(contentTypeFor("Baijimu_0.2.11_universal.dmg")).toBe(
      "application/x-apple-diskimage",
    );
    expect(contentTypeFor("Baijimu_0.2.11_x64_en-US.msi")).toBe(
      "application/x-msi",
    );
    expect(contentTypeFor("Baijimu_0.2.11_amd64.deb")).toBe(
      "application/vnd.debian.binary-package",
    );
    expect(contentTypeFor("Baijimu_0.2.11_amd64.AppImage")).toBe(
      "application/octet-stream",
    );
    expect(contentTypeFor("Baijimu_0.2.11_universal.app.tar.gz")).toBe(
      "application/gzip",
    );
  });

  test("completes with the signed permanent identity only", () => {
    const metadata = {
      tagName: "bridge-agent-v0.2.11",
      version: "0.2.11",
      target: "Linux x64",
      name: "Baijimu_0.2.11_amd64.AppImage",
      sha256: "a".repeat(64),
      contentType: "application/octet-stream",
      sizeBytes: 42,
    };
    const payload = completionPayload(metadata, {
      objectKey,
      downloadUrl: `https://baijimu-lowcode-public-20260420.oss-cn-beijing.aliyuncs.com/${objectKey}`,
      resourceUrl: "https://downloads.baijimu.com/bridge-agent/linux",
      uploadReceipt: "payload.signature",
    });

    expect(payload).toEqual({
      ...metadata,
      objectKey,
      downloadUrl: `https://baijimu-lowcode-public-20260420.oss-cn-beijing.aliyuncs.com/${objectKey}`,
      uploadReceipt: "payload.signature",
    });
    expect(payload).not.toHaveProperty("resourceUrl");
  });

  test("scales OSS transfer timeouts for large release bundles", () => {
    expect(transferTimeoutMsForSize(0)).toBe(20 * 60_000);
    expect(transferTimeoutMsForSize(10_000_000)).toBe(20 * 60_000);
    expect(transferTimeoutMsForSize(89_364_984)).toBe(9_056_499);
    expect(transferTimeoutMsForSize(2_000_000_000)).toBe(3 * 60 * 60_000);

    expect(() => transferTimeoutMsForSize(-1)).toThrow(/Invalid transfer size/);
  });
});
