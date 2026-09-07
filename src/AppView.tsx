import { invoke } from "@tauri-apps/api/core";


import { useEffect } from "react";



import { type LocalAppInstallTask } from "./local-app-install-tasks";










import { toUiConfig, fromUiConfig, defaultCapabilityArgumentsText } from "./app/config-conversion";
import { serviceSignature } from "./app/formatters";


import { SettingsSectionPanel } from "./app/settings-section";
import { ServiceRuntimeConfigPanel } from "./app/service-runtime-config-panel";
import { ServiceEditorPanel } from "./app/service-editor-panel";
import { CapabilityMethodConfigPanel } from "./app/capability-method-config-panel";
import { LocalAppAbilityList } from "./app/local-app-ability-list";
import { InstallLocalAppPanel } from "./app/install-local-app-panel";
import { LocalAppDetailDialog } from "./app/local-app-detail-dialog";
import { ForceUpdateOverlay, RecoveryUpdateStatusCard, StartupRecoveryPanel } from "./app/recovery-panels";
import { DetailPanelContent, DiagnosticsPage, SettingsPage, ToastStack } from "./app/diagnostics-and-settings";
import { LocalAppInstallProgress, LocalAppUpdateChanges, LocalAppUpgradeDialog } from "./app/local-app-upgrade-panels";
import { LocalAppCard, LocalAppListPanel } from "./app/local-app-list-panel";
import { AppAttentionPanel } from "./app/app-attention-panel";
import { useAppController } from "./use-app-controller";
import { ComputerPermissionPanel, ConnectorPermissionPanel, resolveServiceRuntimeView, ServiceDefinitionJsonPanel, ServiceRuntimePanel } from "./app/service-status-panels";
import { AuthorizationGatePanel, LocalAppRuntimePanel, LogMetadata, RuntimeConflictPanel } from "./app/runtime-view-panels";
import type { BrowserAuthPollResponse, ConnectorSummary, LocalAppItem, LogEntry, MarketConnector, UiMethodConfig, UiServiceConfig } from "./app/types";




export function AppView({ controller }: { controller: ReturnType<typeof useAppController> }) {
  // Readiness is acknowledged only after a successful commit with loaded business config.
  useEffect(() => {
    if (controller.config) void invoke("mark_frontend_ready").catch(() => {});
  }, [controller.config != null]);
  const {
    configPath,
    manifestPreview,
    config,
    setConfig,
    savedServiceSignatures,
    setSavedServiceSignatures,
    runtime,
    logServiceFilter,
    setLogServiceFilter,
    localAppQuery,
    setLocalAppQuery,
    localAppKindFilter,
    setLocalAppKindFilter,
    refreshing,
    busy,
    message,
    setMessage,
    error,
    setError,
    runtimeConflict,
    setRuntimeConflict,
    browserAuth,
    setBrowserAuth,
    appVersion,
    appUpdate,
    desktopPermissions,
    registeredServiceStatuses,
    appUpdateCheckState,
    appUpdateError,
    updateBusy,
    appUpdateProgress,
    startupHealth,
    startupRecoveryBusy,
    serviceStartBusy,
    connectorBusy,
    connectorUninstalling,
    connectorUpdateBusy,
    managedToolBusy,
    serviceNotices,
    serviceJsonDrafts,
    serviceJsonErrors,
    capabilityTestDrafts,
    setCapabilityTestDrafts,
    capabilityTestResults,
    capabilityTestBusy,
    desktopPermissionBusy,
    showAdvancedSettings,
    setShowAdvancedSettings,
    activeSettingsSection,
    setActiveSettingsSection,
    activePage,
    setActivePage,
    activeDetailPanel,
    setActiveDetailPanel,
    setExpandedServiceIndex,
    setSelectedLocalAppId,
    setPendingUpgradeAppId,
    activeLocalAppDetailTab,
    setActiveLocalAppDetailTab,
    installPanelOpen,
    setInstallPanelOpen,
    installSourceMode,
    setInstallSourceMode,
    selectedMarketAppId,
    setSelectedMarketAppId,
    marketAppQuery,
    setMarketAppQuery,
    marketLoading,
    marketLoadError,
    registeredInstallAppId,
    setRegisteredInstallAppId,
    registeredInstallVersion,
    setRegisteredInstallVersion,
    customInstallConfirmed,
    setCustomInstallConfirmed,
    installBusy,
    pythonStatus,
    setPythonStatus,
    pythonCheckBusy,
    startupUpdateGate,
    startupConfigGateReady,
    startupConfigMigrationFailure,
    installableMarketConnectors,
    visibleMarketConnectors,
    authorizationState,
    needsAuthorization,
    statusLabel,
    startActionLocked,
    runtimeCanStop,
    startActionLabel,
    logServiceOptions,
    filteredLogs,
    localApps,
    selectedLocalApp,
    pendingUpgradeApp,
    availableLocalAppUpdates,
    appVersionLabel,
    appUpdateStatusLabel,
    appUpdateTone,
    hasDesktopPermissionGap,
    buildCapabilityTestKey,
    isMethodAdvancedOpen,
    toggleMethodAdvanced,
    openLocalAppCapabilityConfig,
    handleCommandError,
    refreshAll,
    refreshPythonRuntime,
    useDetectedPython,
    applyRuntimeSnapshot,
    refreshRegisteredServiceStatuses,
    refreshMarketConnectorApps,
    startRegisteredService,
    stopRegisteredService,
    installLocalApp,
    marketAppForLocalApp,
    localAppUpdateStatus,
    upgradeLocalAppVersion,
    checkLocalAppVersion,
    upgradeManagedTool,
    rollbackManagedTool,
    connectorSyncSource,
    connectorSourceKind,
    syncLocalApp,
    startLocalApp,
    stopLocalApp,
    testCapability,
    testLocalAppCapability,
    uninstallLocalApp,
    checkAppUpdate,
    checkStartupAppUpdate,
    openAppUninstaller,
    renderAppUpdateProgress,
    installAppUpdate,
    upgradeClientForMarketApp,
    restartInNormalMode,
    openStartupLog,
    saveConfig,
    saveService,
    deleteSavedService,
    startAgent,
    stopConflictingRuntimeAndStart,
    stopAgent,
    resetExampleConfig,
    recoverInvalidConfig,
    clearLogs,
    openExternalUrl,
    openAppUpdateReleasePage,
    openConsole,
    openExternalUrlInEdge,
    copyText,
    requestDesktopPermission,
    openDesktopPermissionSettings,
    beginBrowserAuth,
    updateRelay,
    updatePlatform,
    updateUpload,
    updateDevice,
    updateRuntime,
    updateService,
    updateServiceHealthCheck,
    updateServiceStartCommand,
    updateServiceStopCommand,
    updateServiceJsonDraft,
    resetServiceJsonDraft,
    applyServiceJsonDraft,
    removeService,
    removeMethod,
    updateMethod,
    grantFullShellAccess,
    restoreSafeShellAccess,
    formatLocalAppKind,
    countLocalAppCapabilities,
    localAppLifecycle,
    hasLocalAppStopCommand,
    hasLocalAppStartCommand,
  } = controller;


  function renderSettingsSection() {
    return (
      <SettingsSectionPanel
        activeSettingsSection={activeSettingsSection}
        config={config}
        configPath={configPath}
        openExternalUrl={openExternalUrl}
        pythonCheckBusy={pythonCheckBusy}
        pythonStatus={pythonStatus}
        refreshPythonRuntime={refreshPythonRuntime}
        setPythonStatus={setPythonStatus}
        setShowAdvancedSettings={setShowAdvancedSettings}
        showAdvancedSettings={showAdvancedSettings}
        updateDevice={updateDevice}
        updatePlatform={updatePlatform}
        updateRelay={updateRelay}
        updateRuntime={updateRuntime}
        updateUpload={updateUpload}
        useDetectedPython={useDetectedPython}
      />
    );
  }

  function renderRuntimeConflictPanel() {
    return <RuntimeConflictPanel busy={busy} runtimeConflict={runtimeConflict} setRuntimeConflict={setRuntimeConflict} stopConflictingRuntimeAndStart={stopConflictingRuntimeAndStart} />;
  }

  function renderDeviceAuthorizationGate() {
    return <AuthorizationGatePanel authorizationState={authorizationState} beginBrowserAuth={beginBrowserAuth} browserAuth={browserAuth} busy={busy} config={config} copyText={copyText} openExternalUrl={openExternalUrl} openExternalUrlInEdge={openExternalUrlInEdge} setActivePage={setActivePage} />;
  }

  function serviceRuntimeView(service: UiServiceConfig) {
    return resolveServiceRuntimeView(service, registeredServiceStatuses);
  }

  function renderServiceRuntimePanel(service: UiServiceConfig, serviceIndex: number, isSystem: boolean) {
    return <ServiceRuntimePanel isSystem={isSystem} refreshRegisteredServiceStatuses={refreshRegisteredServiceStatuses} registeredServiceStatuses={registeredServiceStatuses} service={service} serviceIndex={serviceIndex} serviceStartBusy={serviceStartBusy} startRegisteredService={startRegisteredService} stopRegisteredService={stopRegisteredService} updateService={updateService} />;
  }

  function renderServiceRuntimeConfig(service: UiServiceConfig, serviceIndex: number, isSystem: boolean) {
    return (
      <ServiceRuntimeConfigPanel
        isSystem={isSystem}
        service={service}
        serviceIndex={serviceIndex}
        updateService={updateService}
        updateServiceHealthCheck={updateServiceHealthCheck}
        updateServiceStartCommand={updateServiceStartCommand}
        updateServiceStopCommand={updateServiceStopCommand}
      />
    );
  }

  function renderComputerPermissionPanel(service: UiServiceConfig) {
    return <ComputerPermissionPanel desktopPermissionBusy={desktopPermissionBusy} desktopPermissions={desktopPermissions} openDesktopPermissionSettings={openDesktopPermissionSettings} requestDesktopPermission={requestDesktopPermission} service={service} />;
  }

  function renderConnectorPermissionPanel(connector: ConnectorSummary) {
    return <ConnectorPermissionPanel connector={connector} desktopPermissions={desktopPermissions} openDesktopPermissionSettings={openDesktopPermissionSettings} />;
  }

  function renderServiceDefinitionJson(service: UiServiceConfig, serviceIndex: number) {
    return <ServiceDefinitionJsonPanel applyServiceJsonDraft={applyServiceJsonDraft} resetServiceJsonDraft={resetServiceJsonDraft} service={service} serviceIndex={serviceIndex} serviceJsonDrafts={serviceJsonDrafts} serviceJsonErrors={serviceJsonErrors} updateServiceJsonDraft={updateServiceJsonDraft} />;
  }

  function renderServiceEditor(service: UiServiceConfig, serviceIndex: number) {
    return (
      <ServiceEditorPanel
        busy={busy}
        deleteSavedService={deleteSavedService}
        grantFullShellAccess={grantFullShellAccess}
        isMethodAdvancedOpen={isMethodAdvancedOpen}
        removeMethod={removeMethod}
        removeService={removeService}
        renderComputerPermissionPanel={renderComputerPermissionPanel}
        renderServiceDefinitionJson={renderServiceDefinitionJson}
        renderServiceRuntimeConfig={renderServiceRuntimeConfig}
        renderServiceRuntimePanel={renderServiceRuntimePanel}
        restoreSafeShellAccess={restoreSafeShellAccess}
        saveService={saveService}
        savedServiceSignatures={savedServiceSignatures}
        service={service}
        serviceIndex={serviceIndex}
        serviceNotices={serviceNotices}
        toggleMethodAdvanced={toggleMethodAdvanced}
        updateMethod={updateMethod}
        updateService={updateService}
      />
    );
  }

  function renderCapabilityMethodConfig(method: UiMethodConfig, serviceIndex: number, methodIndex: number) {
    return (
      <CapabilityMethodConfigPanel
        grantFullShellAccess={grantFullShellAccess}
        isMethodAdvancedOpen={isMethodAdvancedOpen}
        method={method}
        methodIndex={methodIndex}
        restoreSafeShellAccess={restoreSafeShellAccess}
        serviceIndex={serviceIndex}
        updateMethod={updateMethod}
      />
    );
  }

  function renderLocalAppAbilityList(app: LocalAppItem, canShowConfig: boolean) {
    return (
      <LocalAppAbilityList
        app={app}
        buildCapabilityTestKey={buildCapabilityTestKey}
        busy={busy}
        canShowConfig={canShowConfig}
        capabilityTestBusy={capabilityTestBusy}
        capabilityTestDrafts={capabilityTestDrafts}
        capabilityTestResults={capabilityTestResults}
        config={config}
        defaultCapabilityArgumentsText={defaultCapabilityArgumentsText}
        isMethodAdvancedOpen={isMethodAdvancedOpen}
        openLocalAppCapabilityConfig={openLocalAppCapabilityConfig}
        renderCapabilityMethodConfig={renderCapabilityMethodConfig}
        saveService={saveService}
        savedServiceSignatures={savedServiceSignatures}
        serviceSignature={serviceSignature}
        setCapabilityTestDrafts={setCapabilityTestDrafts}
        testCapability={testCapability}
        testLocalAppCapability={testLocalAppCapability}
      />
    );
  }

  function renderLocalAppRuntime(app: LocalAppItem) {
    return <LocalAppRuntimePanel app={app} config={config} localAppLifecycle={localAppLifecycle} serviceRuntimeView={serviceRuntimeView} />;
  }

  function renderInstallLocalAppPanel() {
    return (
      <InstallLocalAppPanel
        appUpdate={appUpdate}
        appUpdateCheckState={appUpdateCheckState}
        appUpdateProgress={appUpdateProgress}
        customInstallConfirmed={customInstallConfirmed}
        installBusy={installBusy}
        installLocalApp={installLocalApp}
        installPanelOpen={installPanelOpen}
        installSourceMode={installSourceMode}
        installableMarketConnectors={installableMarketConnectors}
        marketAppQuery={marketAppQuery}
        marketLoadError={marketLoadError}
        marketLoading={marketLoading}
        refreshMarketConnectorApps={refreshMarketConnectorApps}
        registeredInstallAppId={registeredInstallAppId}
        registeredInstallVersion={registeredInstallVersion}
        selectedMarketAppId={selectedMarketAppId}
        setCustomInstallConfirmed={setCustomInstallConfirmed}
        setInstallPanelOpen={setInstallPanelOpen}
        setInstallSourceMode={setInstallSourceMode}
        setMarketAppQuery={setMarketAppQuery}
        setRegisteredInstallAppId={setRegisteredInstallAppId}
        setRegisteredInstallVersion={setRegisteredInstallVersion}
        setSelectedMarketAppId={setSelectedMarketAppId}
        updateBusy={updateBusy}
        upgradeClientForMarketApp={upgradeClientForMarketApp}
        visibleMarketConnectors={visibleMarketConnectors}
      />
    );
  }

  function renderAppAttentionPanel() {
    return <AppAttentionPanel appUpdate={appUpdate} appUpdateProgress={appUpdateProgress} busy={busy} config={config} hasDesktopPermissionGap={hasDesktopPermissionGap} installAppUpdate={installAppUpdate} openAppUpdateReleasePage={openAppUpdateReleasePage} runtime={runtime} runtimeCanStop={runtimeCanStop} setActiveLocalAppDetailTab={setActiveLocalAppDetailTab} setActivePage={setActivePage} setExpandedServiceIndex={setExpandedServiceIndex} setSelectedLocalAppId={setSelectedLocalAppId} startActionLabel={startActionLabel} startActionLocked={startActionLocked} startAgent={startAgent} statusLabel={statusLabel} stopAgent={stopAgent} updateBusy={updateBusy} />;
  }

  function renderLocalAppPanel() {
    return <LocalAppListPanel availableLocalAppUpdates={availableLocalAppUpdates} busy={busy} config={config} formatLocalAppKind={formatLocalAppKind} localAppKindFilter={localAppKindFilter} localAppQuery={localAppQuery} localApps={localApps} needsAuthorization={needsAuthorization} openConsole={openConsole} refreshAll={refreshAll} refreshing={refreshing} refreshMarketConnectorApps={refreshMarketConnectorApps} renderAppAttentionPanel={renderAppAttentionPanel} renderLocalAppCard={renderLocalAppCard} setCustomInstallConfirmed={setCustomInstallConfirmed} setInstallPanelOpen={setInstallPanelOpen} setInstallSourceMode={setInstallSourceMode} setLocalAppKindFilter={setLocalAppKindFilter} setLocalAppQuery={setLocalAppQuery} setMarketAppQuery={setMarketAppQuery} setSelectedLocalAppId={setSelectedLocalAppId} />;
  }

  function renderLocalAppCard(app: LocalAppItem) {
    return <LocalAppCard key={app.id} app={app} config={config} countLocalAppCapabilities={countLocalAppCapabilities} formatLocalAppKind={formatLocalAppKind} hasLocalAppStartCommand={hasLocalAppStartCommand} localAppLifecycle={localAppLifecycle} localAppUpdateStatus={localAppUpdateStatus} marketAppForLocalApp={marketAppForLocalApp} renderLocalAppInstallProgress={renderLocalAppInstallProgress} setActiveLocalAppDetailTab={setActiveLocalAppDetailTab} setExpandedServiceIndex={setExpandedServiceIndex} setSelectedLocalAppId={setSelectedLocalAppId} />;
  }

  function renderLocalAppInstallProgress(task: LocalAppInstallTask, compact = false) {
    return <LocalAppInstallProgress compact={compact} task={task} />;
  }

  function renderLocalAppUpdateChanges(app: LocalAppItem, marketApp: MarketConnector) {
    return <LocalAppUpdateChanges app={app} marketApp={marketApp} />;
  }

  function renderLocalAppUpgradeDialog(app: LocalAppItem) {
    return <LocalAppUpgradeDialog app={app} connectorUpdateBusy={connectorUpdateBusy} localAppUpdateStatus={localAppUpdateStatus} managedToolBusy={managedToolBusy} marketAppForLocalApp={marketAppForLocalApp} renderLocalAppUpdateChanges={renderLocalAppUpdateChanges} setPendingUpgradeAppId={setPendingUpgradeAppId} upgradeLocalAppVersion={upgradeLocalAppVersion} />;
  }

  function renderLocalAppDetailDialog(app: LocalAppItem) {
    return (
      <LocalAppDetailDialog
        activeLocalAppDetailTab={activeLocalAppDetailTab}
        app={app}
        checkLocalAppVersion={checkLocalAppVersion}
        config={config}
        connectorBusy={connectorBusy}
        connectorSourceKind={connectorSourceKind}
        connectorSyncSource={connectorSyncSource}
        connectorUninstalling={connectorUninstalling}
        connectorUpdateBusy={connectorUpdateBusy}
        countLocalAppCapabilities={countLocalAppCapabilities}
        formatLocalAppKind={formatLocalAppKind}
        hasLocalAppStartCommand={hasLocalAppStartCommand}
        hasLocalAppStopCommand={hasLocalAppStopCommand}
        localAppLifecycle={localAppLifecycle}
        localAppUpdateStatus={localAppUpdateStatus}
        managedToolBusy={managedToolBusy}
        marketAppForLocalApp={marketAppForLocalApp}
        renderComputerPermissionPanel={renderComputerPermissionPanel}
        renderConnectorPermissionPanel={renderConnectorPermissionPanel}
        renderLocalAppAbilityList={renderLocalAppAbilityList}
        renderLocalAppInstallProgress={renderLocalAppInstallProgress}
        renderLocalAppRuntime={renderLocalAppRuntime}
        renderLocalAppUpdateChanges={renderLocalAppUpdateChanges}
        renderServiceEditor={renderServiceEditor}
        rollbackManagedTool={rollbackManagedTool}
        setActiveLocalAppDetailTab={setActiveLocalAppDetailTab}
        setPendingUpgradeAppId={setPendingUpgradeAppId}
        setSelectedLocalAppId={setSelectedLocalAppId}
        showAdvancedSettings={showAdvancedSettings}
        startLocalApp={startLocalApp}
        stopLocalApp={stopLocalApp}
        syncLocalApp={syncLocalApp}
        uninstallLocalApp={uninstallLocalApp}
        upgradeManagedTool={upgradeManagedTool}
      />
    );
  }

  function renderAppsPage() {
    return (
      <div className="service-editor-panel">
        {renderLocalAppPanel()}
        {selectedLocalApp ? renderLocalAppDetailDialog(selectedLocalApp) : null}
        {pendingUpgradeApp ? renderLocalAppUpgradeDialog(pendingUpgradeApp) : null}
      </div>
    );
  }

  function renderLogMetadata(entry: LogEntry) {
    return <LogMetadata entry={entry} />;
  }

  function renderDetailPanel() {
    return <DetailPanelContent activeDetailPanel={activeDetailPanel} activeSettingsSection={activeSettingsSection} appUpdate={appUpdate} appUpdateCheckState={appUpdateCheckState} appUpdateProgress={appUpdateProgress} appUpdateStatusLabel={appUpdateStatusLabel} appUpdateTone={appUpdateTone} appVersion={appVersion} appVersionLabel={appVersionLabel} beginBrowserAuth={beginBrowserAuth} busy={busy} checkAppUpdate={checkAppUpdate} clearLogs={clearLogs} config={config} configPath={configPath} filteredLogs={filteredLogs} installAppUpdate={installAppUpdate} logServiceFilter={logServiceFilter} logServiceOptions={logServiceOptions} manifestPreview={manifestPreview} needsAuthorization={needsAuthorization} openAppUninstaller={openAppUninstaller} openAppUpdateReleasePage={openAppUpdateReleasePage} renderAppUpdateProgress={renderAppUpdateProgress} renderLogMetadata={renderLogMetadata} renderSettingsSection={renderSettingsSection} runtime={runtime} saveConfig={saveConfig} setActiveSettingsSection={setActiveSettingsSection} setLogServiceFilter={setLogServiceFilter} statusLabel={statusLabel} updateBusy={updateBusy} />;
  }

  function renderDiagnosticsPage() {
    return <DiagnosticsPage activeDetailPanel={activeDetailPanel} busy={busy} refreshAll={refreshAll} refreshing={refreshing} renderDetailPanel={renderDetailPanel} resetExampleConfig={resetExampleConfig} setActiveDetailPanel={setActiveDetailPanel} />;
  }

  function renderSettingsPage() {
    return <SettingsPage activeSettingsSection={activeSettingsSection} beginBrowserAuth={beginBrowserAuth} busy={busy} config={config} refreshAll={refreshAll} refreshing={refreshing} renderSettingsSection={renderSettingsSection} runtime={runtime} saveConfig={saveConfig} setActiveSettingsSection={setActiveSettingsSection} />;
  }

  function renderToastStack() {
    return <ToastStack error={error} message={message} setError={setError} setMessage={setMessage} />;
  }

  useEffect(() => {
    if (!browserAuth || !config) {
      return;
    }
    let active = true;
    let timer: number | null = null;
    const intervalMs = Math.max(browserAuth.interval, 1) * 1000;
    const session = browserAuth;
    const authConfig = fromUiConfig(config);

    async function poll() {
      try {
        const result = await invoke<BrowserAuthPollResponse>("poll_browser_auth", {
          config: authConfig,
          deviceCode: session.deviceCode
        });
        if (!active) {
          return;
        }
        if (result.status === "authorized" && result.config) {
          const uiConfig = toUiConfig(result.config);
          setConfig(uiConfig);
          setSavedServiceSignatures(uiConfig.services.map(serviceSignature));
          if (result.runtime) {
            applyRuntimeSnapshot(result.runtime);
          }
          setBrowserAuth(null);
          setMessage("浏览器授权成功，Agent 已使用新凭证重启");
          return;
        }
        if (result.status === "denied" || result.status === "expired") {
          setBrowserAuth(null);
          setError(result.message);
          return;
        }
        timer = window.setTimeout(() => void poll(), intervalMs);
      } catch (err) {
        if (active) {
          setBrowserAuth(null);
          handleCommandError(err);
        }
      }
    }

    timer = window.setTimeout(() => void poll(), intervalMs);
    return () => {
      active = false;
      if (timer != null) {
        window.clearTimeout(timer);
      }
    };
  }, [browserAuth, config]);

  const degradedStartupComponents =
    startupHealth?.components.filter((component) => component.status === "degraded") ?? [];

  function renderRecoveryUpdateCard() {
    return <RecoveryUpdateStatusCard appUpdate={appUpdate} appUpdateCheckState={appUpdateCheckState} appUpdateError={appUpdateError} appUpdateProgress={appUpdateProgress} appVersion={appVersion} checkStartupAppUpdate={checkStartupAppUpdate} installAppUpdate={installAppUpdate} openAppUpdateReleasePage={openAppUpdateReleasePage} renderAppUpdateProgress={renderAppUpdateProgress} updateBusy={updateBusy} />;
  }

  function renderStartupRecoveryPanel() {
    return <StartupRecoveryPanel busy={busy} openStartupLog={openStartupLog} recoverInvalidConfig={recoverInvalidConfig} renderRecoveryUpdateCard={renderRecoveryUpdateCard} restartInNormalMode={restartInNormalMode} startupHealth={startupHealth} startupRecoveryBusy={startupRecoveryBusy} />;
  }

  if (!config) {
    return <StartupLoadingView busy={busy} error={error} openStartupLog={openStartupLog} recoverInvalidConfig={recoverInvalidConfig} refreshAll={refreshAll} renderRecoveryUpdateCard={renderRecoveryUpdateCard} renderStartupRecoveryPanel={renderStartupRecoveryPanel} renderToastStack={renderToastStack} restartInNormalMode={restartInNormalMode} startupConfigGateReady={startupConfigGateReady} startupConfigMigrationFailure={startupConfigMigrationFailure} startupHealth={startupHealth} startupRecoveryBusy={startupRecoveryBusy} startupUpdateGate={startupUpdateGate} />;
  }

  function renderForceUpdateOverlay() {
    return <ForceUpdateOverlay appUpdate={appUpdate} appUpdateCheckState={appUpdateCheckState} appUpdateProgress={appUpdateProgress} checkAppUpdate={checkAppUpdate} installAppUpdate={installAppUpdate} openAppUpdateReleasePage={openAppUpdateReleasePage} renderAppUpdateProgress={renderAppUpdateProgress} updateBusy={updateBusy} />;
  }

  return <DesktopApplicationShell activeDetailPanel={activeDetailPanel} activePage={activePage} appUpdate={appUpdate} appVersion={appVersion} authorizationState={authorizationState} config={config} degradedStartupComponents={degradedStartupComponents} needsAuthorization={needsAuthorization} openStartupLog={openStartupLog} renderAppsPage={renderAppsPage} renderDeviceAuthorizationGate={renderDeviceAuthorizationGate} renderDiagnosticsPage={renderDiagnosticsPage} renderForceUpdateOverlay={renderForceUpdateOverlay} renderInstallLocalAppPanel={renderInstallLocalAppPanel} renderRuntimeConflictPanel={renderRuntimeConflictPanel} renderSettingsPage={renderSettingsPage} renderStartupRecoveryPanel={renderStartupRecoveryPanel} renderToastStack={renderToastStack} runtime={runtime} setActiveDetailPanel={setActiveDetailPanel} setActivePage={setActivePage} setSelectedLocalAppId={setSelectedLocalAppId} startupHealth={startupHealth} statusLabel={statusLabel} />;
}

import { DesktopApplicationShell, StartupLoadingView } from "./app/app-shell";
