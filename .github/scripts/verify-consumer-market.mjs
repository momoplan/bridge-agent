import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export async function verifyConsumerMarket({ baseUrl, workspaceId, token, selection, appId, version }, request = fetch) {
  const base = new URL(baseUrl);
  if (base.protocol !== "https:" || base.username || base.password || base.search || base.hash ||
      baseUrl.trim() !== baseUrl || !/^[1-9][0-9]*$/.test(workspaceId) ||
      !token || token.trim() !== token || /[\r\n]/.test(token)) throw new Error("Invalid consumer market access configuration");
  if (selection?.kind !== "market" || !selection.marketKey || !selection.listingId ||
      !selection.source?.application?.environmentKey || selection.source.application.appId !== appId ||
      selection.version !== version || selection.source.version !== version) throw new Error("Bundled tool requires an exact market source binding");
  async function get(path, after) {
    const url = new URL(`${base.href.replace(/\/$/, "")}/${path}`);
    url.searchParams.set("workspaceId", workspaceId);
    if (after) url.searchParams.set("after", after);
    const response = await request(url, { redirect: "error", signal: AbortSignal.timeout(60_000), headers: { Authorization: `Bearer ${token}` } });
    if (response.status !== 200) throw new Error(`Consumer service returned HTTP ${response.status}`);
    const envelope = await response.json();
    if (envelope.contractVersion !== "1.0.0" || envelope.errorCode !== "0" || envelope.data == null) throw new Error("Invalid consumer CModel response");
    return envelope.data;
  }
  if (await get("context") !== selection.marketKey) throw new Error("Consumer market binding changed");
  let after;
  let match;
  const cursors = new Set();
  do {
    const page = await get("listings", after);
    if (!Array.isArray(page.items)) throw new Error("Invalid market page");
    for (const listing of page.items) {
      if (listing.marketKey !== selection.marketKey) throw new Error("Unexpected market authority");
      if (listing.listingId === selection.listingId) {
        if (match) throw new Error("Duplicate catalog entry");
        match = listing;
      }
    }
    after = page.nextCursor;
    if (after && (cursors.has(after) || page.items.at(-1)?.listingId !== after)) throw new Error("Invalid catalog cursor");
    cursors.add(after);
  } while (after);
  const frozen = match?.frozenVersion;
  if (frozen?.source?.application?.environmentKey !== selection.source.application.environmentKey ||
      frozen?.source?.application?.appId !== appId || frozen?.source?.version !== version ||
      frozen?.content?.applicationType !== "managed_tool" || !frozen.content.artifacts?.length) {
    throw new Error("Bundled CLI differs from the current release of its bound market source");
  }
  return { listingId: selection.listingId, version };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const selection = JSON.parse(process.env.BAIJIMU_CLI_MARKET_SOURCE || "null");
    await verifyConsumerMarket({
      baseUrl: process.env.LOCAL_APP_CONSUMER_API_BASE_URL,
      workspaceId: process.env.LOCAL_APP_CONSUMER_WORKSPACE_ID,
      token: process.env.LOCAL_APP_CONSUMER_TOKEN,
      selection,
      appId: readFileSync("tools/baijimu-cli/APP_ID", "utf8").trim(),
      version: readFileSync("tools/baijimu-cli/VERSION", "utf8").trim(),
    });
    writeFileSync("tools/baijimu-cli/INSTALL_SOURCE.json", JSON.stringify(selection, null, 2) + "\n");
    console.log("Bundled CLI matches its consumer market source");
  } catch {
    // Never print request objects, configured endpoints or access credentials from transport errors.
    console.error("Consumer market release verification failed; check the configured source and service access");
    process.exitCode = 1;
  }
}
