import { sameMarketApplication } from "./market-identity";
import { compareVersions } from "./formatters";
import type { LocalAppItem, LocalAppUpdateStatus, MarketConnector } from "./types";

// Read-only derivations must not depend on action factories or their initialization order.
export function createLocalAppUpdateSelectors(marketConnectors: readonly MarketConnector[]) {
  function marketConnectorForLocalApp(app: LocalAppItem): MarketConnector | undefined {
    if (app.kind !== "connector" || !app.connector || app.connector.reviewStatus !== "PUBLISHED") {
      return undefined;
    }
    return marketConnectors.find((item) => sameMarketApplication(app.connector?.installSource, item.installSource));
  }

  function marketManagedToolForLocalApp(app: LocalAppItem): MarketConnector | undefined {
    if (app.kind !== "managed_tool" || !app.managedTool) return undefined;
    return marketConnectors.find((item) =>
      item.applicationType === "managed_tool" && sameMarketApplication(app.managedTool?.installSource, item.installSource));
  }

  function marketAppForLocalApp(app: LocalAppItem): MarketConnector | undefined {
    return app.kind === "managed_tool"
      ? marketManagedToolForLocalApp(app) : marketConnectorForLocalApp(app);
  }

  function localAppUpdateStatus(app: LocalAppItem): LocalAppUpdateStatus | undefined {
    const marketApp = marketAppForLocalApp(app);
    const currentVersion = app.managedTool?.installedVersion ?? app.connector?.version ?? null;
    if (!marketApp || !currentVersion) return undefined;
    return {
      appId: app.id, name: app.name, currentVersion, latestVersion: marketApp.version,
      updateAvailable: compareVersions(marketApp.version, currentVersion) > 0,
      source: marketApp.source
    };
  }

  return { marketConnectorForLocalApp, marketManagedToolForLocalApp, marketAppForLocalApp, localAppUpdateStatus };
}
