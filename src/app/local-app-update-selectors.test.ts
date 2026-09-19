import { expect, it } from "vitest";
import { createLocalAppUpdateSelectors } from "./local-app-update-selectors";
import type { InstallSource } from "./market-identity";
import type { LocalAppItem, MarketConnector } from "./types";

function source(version: string, environmentKey = "official-source"): InstallSource {
  return { kind: "market", marketKey: "official-market", listingId: "cli-listing", version,
    source: { application: { environmentKey, appId: "fixture-cli" }, version } };
}

function installed(version: string, updateSource?: InstallSource): LocalAppItem {
  return { id: "fixture-cli", name: "CLI", description: "", kind: "managed_tool", serviceIndexes: [],
    managedTool: { id: "fixture-cli", name: "CLI", description: "", state: "ready",
      installedVersion: version, installSource: null, updateSource: updateSource ?? null, activePath: "/cli",
      launcherPath: "/bin/cli", pathConfigured: true, restartRequired: false, canRollback: true, detail: "" } };
}

function market(version: string, environmentKey?: string): MarketConnector {
  return { applicationType: "managed_tool", version, installSource: source(version, environmentKey) } as MarketConnector;
}

it("offers an orphaned CLI the newer release from its authoritative update subscription", () => {
  const app = installed("0.3.0", source("0.2.0"));
  const wrongSource = market("0.5.0", "another-source");
  const release = market("0.4.0");
  const selectors = createLocalAppUpdateSelectors([wrongSource, release]);
  expect(selectors.marketManagedToolForLocalApp(app)).toBe(release);
  expect(selectors.localAppUpdateStatus(app)).toMatchObject({
    currentVersion: "0.3.0", latestVersion: "0.4.0", updateAvailable: true
  });
  expect(app.managedTool?.installSource).toBeNull();
});

it("compares updates with the executable version, not the older subscription version", () => {
  const selectors = createLocalAppUpdateSelectors([market("0.3.0")]);
  expect(selectors.localAppUpdateStatus(installed("0.4.0", source("0.2.0"))))
    .toMatchObject({ currentVersion: "0.4.0", updateAvailable: false });
});

it("does not infer an update source from an app ID alone", () => {
  const selectors = createLocalAppUpdateSelectors([market("0.4.0")]);
  expect(selectors.localAppUpdateStatus(installed("0.3.0"))).toBeUndefined();
});

it("chooses the newest version for a source application across distribution routes", () => {
  const previousRoute = market("0.4.0");
  const newerRoute = {
    ...market("0.5.0"),
    installSource: {
      ...source("0.5.0"),
      marketKey: "replacement-market",
      listingId: "replacement-listing"
    }
  } as MarketConnector;
  const selectors = createLocalAppUpdateSelectors([previousRoute, newerRoute]);
  expect(selectors.localAppUpdateStatus(installed("0.3.0", source("0.2.0"))))
    .toMatchObject({ latestVersion: "0.5.0", updateAvailable: true });
});
