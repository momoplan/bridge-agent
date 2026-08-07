import { readFileSync } from "node:fs";

import { describe, expect, test } from "vitest";

const workflow = readFileSync(
  ".github/workflows/release-bridge-agent.yml",
  "utf8",
);

function jobBody(jobName, nextJobName) {
  const startMarker = `  ${jobName}:\n`;
  const start = workflow.indexOf(startMarker);
  if (start < 0) {
    throw new Error(`workflow job not found: ${jobName}`);
  }

  const end =
    nextJobName === undefined
      ? workflow.length
      : workflow.indexOf(`  ${nextJobName}:\n`, start + startMarker.length);
  if (end < 0) {
    throw new Error(`next workflow job not found: ${nextJobName}`);
  }

  return workflow.slice(start, end);
}

describe("release workflow repository script availability", () => {
  test("publish-update-service checks out the dispatched workflow commit", () => {
    const body = jobBody("publish-update-service");
    const checkoutIndex = body.indexOf("uses: actions/checkout@v4");
    const helperIndex = body.indexOf(
      "node .github/scripts/release-service-url.mjs",
    );

    expect(checkoutIndex).toBeGreaterThanOrEqual(0);
    expect(body).toContain(
      "ref: ${{ github.event_name == 'workflow_dispatch' && github.sha || github.ref }}",
    );
    expect(helperIndex).toBeGreaterThan(checkoutIndex);
  });

  test("release upload never expands an empty array under Bash nounset", () => {
    expect(workflow).not.toContain("prerelease_flag=()");
    expect(workflow).toContain("release_create_args=(");
    expect(workflow).toContain(
      'retry_gh gh release create "${release_create_args[@]}"',
    );
  });

  test("Windows quality gate runs workspace tests and real PATH registry writes", () => {
    const body = jobBody("windows-quality-gate", "release");
    expect(body).toContain("cargo test --locked --workspace");
    expect(body).toContain(
      "managed_tool::tests::windows_registry_path_registration_round_trips",
    );
    expect(body).toContain("cargo test --locked --manifest-path src-tauri/Cargo.toml");
  });

  test("metadata repair can atomically raise the minimum supported client version", () => {
    const body = jobBody("publish-update-service");

    expect(workflow).toContain("minimum_supported_version:");
    expect(workflow).toContain("force_update_message:");
    expect(body).toContain("Update minimum supported client version");
    expect(body).toContain("inputs.minimum_supported_version != ''");
    expect(body).toContain('"$api/release-policy"');
    expect(body).toContain('current_policy="$(curl');
    expect(body).toContain(".releasePageUrl");
    expect(body).toContain("releasePageUrl: $releasePageUrl");
    expect(body).toContain(".releasePageUrl == $release_page_url");
    expect(body).toContain("forceUpdate: false");
    expect(body).toContain(
      "Minimum supported version $minimum_version exceeds release version $version",
    );
    expect(body).toContain(".forceUpdate == true");
    expect(body).toContain(".forceUpdate == false");
    expect(body).toContain(
      '"${latest_api}?currentVersion=${MINIMUM_SUPPORTED_VERSION}"',
    );
  });
});
