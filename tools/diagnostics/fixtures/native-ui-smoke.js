let parentBlocked = false;
try { void window.parent.document.body; } catch { parentBlocked = true; }
const detail = { parentBlocked, nativeIpcExposed: Boolean(window.__TAURI_INTERNALS__), bridgeVersion: window.baijimuLocalApp?.version };
try {
  await window.baijimuLocalApp.invoke('smoke', detail);
  // The actual parent returns through the same bridge as production management operations.
  window.parent.postMessage({ type: 'native-ui-smoke-complete', ...detail }, '*');
} catch (error) {
  window.parent.postMessage({ type: 'native-ui-smoke-failed', error: String(error) }, '*');
}
