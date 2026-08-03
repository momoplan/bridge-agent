export async function loadSynchronizedLocalAppCatalog<TApp, TDocument>(
  listInstalledApps: () => Promise<TApp[]>,
  loadConfig: () => Promise<TDocument>
): Promise<{ apps: TApp[]; document: TDocument }> {
  // Listing installed Connectors synchronizes their manifests into the saved Agent config.
  // Loading the config must happen afterwards so the UI never renders the pre-sync snapshot.
  const apps = await listInstalledApps();
  const document = await loadConfig();
  return { apps, document };
}
