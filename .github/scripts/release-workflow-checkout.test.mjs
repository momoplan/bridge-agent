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
});
