import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const files = {
  jenkins: fs.readFileSync("Jenkinsfile.wecom-local-app-release", "utf8"),
  workflow: fs.readFileSync(".github/workflows/release-wecom-local-app.yml", "utf8"),
  publisher: fs.readFileSync("tools/release/publish-wecom-local-app-market.sh", "utf8"),
  job: fs.readFileSync("deploy/jenkins-wecom-local-app-release.xml", "utf8"),
};

test("WeCom uses an independent local-app release identity", () => {
  for (const content of Object.values(files)) {
    assert.doesNotMatch(content, /bridge-agent-v/);
    assert.doesNotMatch(content, /baijimu-cp|Control Plane/i);
  }
  assert.match(files.jenkins, /wecom-local-app-v\$\{CONNECTOR_VERSION\}/);
  assert.match(files.workflow, /release-wecom-local-app/);
  assert.match(files.job, /Jenkinsfile\.wecom-local-app-release/);
});

test("publisher is scoped to the exact WeCom app and immutable version", () => {
  assert.match(files.publisher, /app\.id='wecom'/);
  assert.match(files.publisher, /com\.baijimu\.connector\.wecom/);
  assert.match(files.publisher, /该版本已经存在且内容与当前流水线不一致/);
  assert.doesNotMatch(files.publisher, /ON DUPLICATE KEY UPDATE/);
});

test("release tests the supported Python range and publishes three platform archives", () => {
  for (const version of ["3.10", "3.11", "3.12", "3.13", "3.14"]) {
    assert.match(files.workflow, new RegExp(`- "${version.replace(".", "\\.")}"`));
  }
  for (const platform of ["macos-universal", "windows-universal", "linux-universal"]) {
    assert.match(files.workflow, new RegExp(`- name: ${platform}`));
  }
  assert.match(
    files.workflow,
    /baijimu-wecom-local-app-\$\{CONNECTOR_VERSION\}-\$\{\{ matrix\.name \}\}\.zip/,
  );
});
