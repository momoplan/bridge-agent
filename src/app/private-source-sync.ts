import type { ConnectorSummary, StartConnectorAppInstallRequest } from "./types";

// Strict SemVer 2.0.0, including the no-leading-zero rule for numeric prereleases.
const identifier = "(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)";
const versionPattern = new RegExp(`^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)(?:-${identifier}(?:\\.${identifier})*)?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$`);

export function privateSourceLabel(connector: ConnectorSummary): string | undefined {
  if (connector.bundleSource) {
    return connector.bundleSource.resourceLocator;
  }
  const selected = connector.installSource;
  if (selected?.kind === "environment") {
    const application = selected.source.application;
    return `${application.environmentKey} / ${application.appId}`;
  }
}

export function privateSourceSyncRequest(
  connector: ConnectorSummary,
  targetVersion: string
): StartConnectorAppInstallRequest {
  const bundle = connector.bundleSource;
  const selected = connector.installSource;
  if (!bundle && selected?.kind !== "environment") {
    throw new Error("当前安装缺少来源环境身份，请从来源环境重新选择安装版本。");
  }
  if ((bundle?.resourceLocator ?? selected?.source.application.appId) !== connector.appId) {
    throw new Error("安装记录中的来源应用身份不一致，无法同步。");
  }
  if (targetVersion.trim() !== targetVersion || !versionPattern.test(targetVersion)) {
    throw new Error("目标版本必须是严格 SemVer 2.0.0，例如 1.2.3。");
  }
  return {
    installSource: !bundle && selected?.kind === "environment" ? {
      kind: "environment",
      source: { application: { ...selected.source.application }, version: targetVersion }
    } : undefined,
    sourceIdentityMigration: false,
    operation: "sync",
    replace: true,
    appId: connector.appId,
    name: connector.name,
    version: targetVersion,
    acceptUnreviewed: true
  };
}
