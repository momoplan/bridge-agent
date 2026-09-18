let parentBlocked = false;
try { void window.parent.document.body; } catch { parentBlocked = true; }
let nativeIpcBlocked = true;
if (window.__TAURI_INTERNALS__) {
  // WebView2 injects the API object into subframes, but the native sender/key must be absent.
  nativeIpcBlocked = !window.__TAURI_INTERNALS__.postMessage;
  for (const command of ['app_version', 'plugin:app|version']) {
    const outcome = await Promise.race([
      window.__TAURI_INTERNALS__.invoke(command).then(() => 'allowed', () => 'blocked'),
      new Promise((resolve) => setTimeout(() => resolve('timeout'), 1500)),
    ]);
    nativeIpcBlocked &&= outcome === 'blocked';
  }
}
const detail = { parentBlocked, nativeIpcBlocked, bridgeVersion: window.baijimuLocalApp?.version };
try {
  await window.baijimuLocalApp.invoke('smoke', detail);
  // The actual parent returns through the same bridge as production management operations.
  window.parent.postMessage({ type: 'native-ui-smoke-complete', ...detail }, '*');
} catch (error) {
  window.parent.postMessage({ type: 'native-ui-smoke-failed', error: String(error) }, '*');
}
