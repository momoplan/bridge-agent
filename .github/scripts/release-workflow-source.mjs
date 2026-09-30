import { readFileSync } from "node:fs";

export function readExpandedReleaseWorkflow(workflowPath) {
  let workflow = readFileSync(workflowPath, "utf8").replace(/\r\n/g, "\n");
  workflow = workflow.replace(
    /^        run: bash (\.github\/scripts\/release-steps\/[^\n]+)$/gm,
    (_match, scriptPath) => inlineScript(scriptPath),
  );
  workflow = workflow.replace(
    /^        run: \|\n          & \.\/(\.github\/scripts\/release-steps\/[^\n]+)$/gm,
    (_match, scriptPath) => inlineScript(scriptPath),
  );
  return workflow;
}

function inlineScript(scriptPath) {
  const body = readFileSync(scriptPath, "utf8")
    .replace(/\r\n/g, "\n")
    .trimEnd()
    .split("\n")
    .map((line) => `          ${line}`)
    .join("\n");
  return `        run: |\n${body}`;
}
