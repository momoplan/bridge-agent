import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export function prepareBundledMarketSource(selection, appId, version) {
  const nonempty = value => typeof value === "string" && value.trim() === value && value.length > 0;
  if (selection?.kind !== "market" || !nonempty(selection.marketKey) || !nonempty(selection.listingId) ||
      !nonempty(selection.source?.application?.environmentKey) ||
      selection.source.application.appId !== appId || selection.version !== version ||
      selection.source.version !== version) throw new Error("Bundled CLI provenance does not match its pinned identity");
  return { kind: "market", marketKey: selection.marketKey, listingId: selection.listingId,
    source: { application: { environmentKey: selection.source.application.environmentKey, appId }, version }, version };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const source = prepareBundledMarketSource(JSON.parse(process.env.BAIJIMU_CLI_MARKET_SOURCE || "null"),
    readFileSync("tools/baijimu-cli/APP_ID", "utf8").trim(),
    readFileSync("tools/baijimu-cli/VERSION", "utf8").trim());
  writeFileSync("tools/baijimu-cli/INSTALL_SOURCE.json", JSON.stringify(source, null, 2) + "\n");
  console.log("Bundled CLI pinned provenance validated");
}
