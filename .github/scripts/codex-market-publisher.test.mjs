import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

const publisher = readFileSync(
  "tools/release/publish-codex-local-app-market.sh",
  "utf8",
);
const pipeline = readFileSync(
  "Jenkinsfile.codex-local-app-release",
  "utf8",
);

describe("Codex local app market publisher", () => {
  test("publishes only through Baijimu CLI without database access", () => {
    expect(publisher).toContain("local-app publish codex");
    expect(publisher).toContain("LOCAL_APP_MARKET_PUBLISH_TOKEN");
    expect(publisher).toContain("BAIJIMU_AUTH_FILE");
    expect(publisher).not.toContain("GetNacosConfig");
    expect(publisher).not.toContain("MYSQL_PWD");
    expect(publisher).not.toMatch(/(^|\s)mysql(\s|$)/m);
    expect(publisher).not.toContain("INSERT INTO local_app");
  });

  test("publishes GitHub Actions OSS artifacts through anonymous Baijimu OSS", () => {
    expect(publisher).toContain("manifest_asset=");
    expect(publisher).toContain(
      "https://lowcode-common.oss-cn-beijing.aliyuncs.com/local-app-artifacts/codex/releases/v",
    );
    expect(publisher).toContain("anonymous OSS download checksum mismatch");
    expect(publisher).toContain("GITHUB_TOKEN is required");
    expect(pipeline).not.toContain("jenkins-aliyun-ram");
    expect(pipeline).toContain("REUSE_RELEASE");
    expect(publisher).not.toContain(
      'release_base="https://github.com/momoplan/bridge-agent/releases/download/',
    );
  });

  test("preserves the complete Connector 2.0 manifest and adds OSS artifacts", () => {
    expect(publisher).toContain('connector_manifest_path="$2"');
    expect(publisher).toContain('select(.schemaVersion == "2.0")');
    expect(publisher).toContain("$connector + {");
    expect(publisher).toContain(".id == \"com.baijimu.connector.codex\"");
    expect(publisher).toContain(".version == $version");
    expect(publisher).toContain("(.transport | type) == \"object\"");
    expect(publisher).toContain("((.methods // []) | length)");
    expect(publisher).toContain("[.remoteCapabilities[]?.name]");
    expect(publisher).not.toContain("runtime: $connector.runtime.type");
    expect(publisher).not.toContain("management: ($connector.management != null)");
    expect(publisher).not.toContain('capabilities: ["connector.setup.v1"]');
    expect(publisher).toContain("hostVersion=0.2.40");
    expect(publisher).toContain(
      "hostCapabilities=connector.setup.v1,connector.process.host-managed.v1",
    );
    expect(publisher).toContain('.runtime.processOwnership == "host"');
    expect(publisher).toContain('.runtime.args == ["start"]');
    expect(publisher).toContain(
      '.hostRequirements.minimumVersion == "0.2.40"',
    );
    expect(publisher).toContain(
      'index("connector.process.host-managed.v1") != null',
    );
    expect(publisher).toContain(".latestVersion.compatibility.compatible == true");
    expect(publisher).toContain(".latestVersion.manifest == $expected_manifest");
    expect(publisher).toContain(".latestVersion.source == $mac_source");
  });
});
