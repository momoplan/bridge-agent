import { describe, expect, test } from "vitest";

import {
  contentTypeFor,
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
            "https://lowcode-common.oss-cn-beijing.aliyuncs.com/object?Expires=1&Signature=x",
          objectKey,
          downloadUrl: `https://lowcode-common.oss-cn-beijing.aliyuncs.com/${objectKey}`,
          uploadReceipt: "payload.signature",
        },
        "Baijimu_0.2.11_amd64.AppImage",
      ),
    ).not.toThrow();

    expect(() =>
      validatePrepareResponse(
        {
          uploadUrl:
            "https://lowcode-common.oss-cn-beijing.aliyuncs.com/object?Expires=1&Signature=x",
          objectKey,
          downloadUrl: `https://example.com/${objectKey}`,
          uploadReceipt: "payload.signature",
        },
        "Baijimu_0.2.11_amd64.AppImage",
      ),
    ).toThrow(/non-canonical OSS/);
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
});
