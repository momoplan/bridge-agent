import { existsSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { join } from "node:path";
const [root, binary] = process.argv.slice(2);
const sourceFile = join(root, "tools/baijimu-cli/INSTALL_SOURCE.json");
if (existsSync(sourceFile)) {
  const source = JSON.parse(readFileSync(sourceFile, "utf8"));
  const version = readFileSync(join(root, "tools/baijimu-cli/VERSION"), "utf8").trim();
  const appId = readFileSync(join(root, "tools/baijimu-cli/APP_ID"), "utf8").trim();
  if (source.kind !== "market" || !source.marketKey || !source.listingId ||
      source.version !== version || source.source?.version !== version ||
      source.source?.application?.appId !== appId || !source.source.application.environmentKey) {
    throw new Error("Bundled CLI provenance does not match its pinned identity");
  }
  writeFileSync(`${binary}.market.json`, JSON.stringify(source, null, 2) + "\n");
} else if (process.env.BAIJIMU_CLI_REQUIRE_MARKET_SOURCE === "true") {
  throw new Error("Verified bundled CLI market source is required for release");
} else {
  rmSync(`${binary}.market.json`, { force: true });
}
