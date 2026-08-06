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

  test("publishes content-addressed artifacts through anonymous Baijimu OSS", () => {
    expect(publisher).toContain('OSS_BUCKET="${OSS_BUCKET:-lowcode-common}"');
    expect(publisher).toContain(
      'OSS_PREFIX="${OSS_PREFIX:-local-app-artifacts/codex}"',
    );
    expect(publisher).toContain(
      'object_prefix="${OSS_PREFIX}/releases/v${version}/${checksum}"',
    );
    expect(publisher).toContain("anonymous OSS download checksum mismatch");
    expect(publisher).toContain("Cache-Control:public,max-age=31536000,immutable");
    expect(publisher).toContain("OSS_ACCESS_KEY_ID");
    expect(publisher).toContain("OSS_ACCESS_KEY_SECRET");
    expect(publisher).toContain("oss_validate_access");
    expect(pipeline.match(/credentialsId: 'jenkins-aliyun-ram'/g)).toHaveLength(2);
    expect(pipeline).toContain("usernameVariable: 'OSS_ACCESS_KEY_ID'");
    expect(pipeline).toContain("passwordVariable: 'OSS_ACCESS_KEY_SECRET'");
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
    expect(publisher).toContain("hostVersion=0.2.21");
    expect(publisher).toContain("hostCapabilities=connector.setup.v1");
    expect(publisher).toContain(".latestVersion.compatibility.compatible == true");
    expect(publisher).toContain(".latestVersion.manifest == $expected_manifest");
    expect(publisher).toContain(".latestVersion.source == $mac_source");
  });
});
