import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type MutableRefObject, type ReactNode } from "react";
import { toUiConfig } from "./config-conversion";
import { readError } from "./formatters";
import { GeneralSettings } from "./general-settings";
import { settingsChanges, settingsSections, type SettingsPageId } from "./settings-fields";
import { SettingsSectionPanel } from "./settings-section";
import type { ConfigDocument, EventStorageStats, PythonRuntimeStatus, UiAgentConfig } from "./types";

export type SettingsLeaveGuard = () => Promise<boolean>;
interface Props {
  config: UiAgentConfig;
  configPath: string;
  leaveGuard: MutableRefObject<SettingsLeaveGuard | null>;
  onSaved: (document: ConfigDocument) => void;
  beginBrowserAuth: () => Promise<void>;
  openExternalUrl: (url: string) => Promise<void>;
  renderAbout: () => ReactNode;
}
export function SettingsWorkspace({ config, configPath, leaveGuard, onSaved, beginBrowserAuth, openExternalUrl, renderAbout }: Props) {
  const latestConfig = useRef(config);
  latestConfig.current = config;
  const [section, setSection] = useState<SettingsPageId>("general");
  const [original, setOriginal] = useState(config);
  const [draft, setDraft] = useState(config);
  const [busy, setBusy] = useState(false);
  const saving = useRef(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [showAdvancedSettings, setShowAdvancedSettings] = useState(false);
  const [pythonStatus, setPythonStatus] = useState<PythonRuntimeStatus | null>(null);
  const [pythonCheckBusy, setPythonCheckBusy] = useState(false);
  const [pendingLeave, setPendingLeave] = useState<((allowed: boolean) => void) | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const pendingRef = useRef<((allowed: boolean) => void) | null>(null);
  const changes = settingsChanges(original, draft, section);
  const dirty = Object.keys(changes).length > 0;
  const editable = section !== "general" && section !== "about";
  const selected = settingsSections.find(item => item.id === section)!;
  useEffect(() => {
    if (!dirty && !saving.current) { setOriginal(config); setDraft(config); }
  }, [config]);
  useEffect(() => {
    leaveGuard.current = async () => {
      if (saving.current || pendingRef.current) return false;
      if (!dirty) return true;
      return new Promise<boolean>(resolve => {
        pendingRef.current = resolve;
        setPendingLeave(() => resolve);
      });
    };
    const unload = (event: BeforeUnloadEvent) => { if (dirty) { event.preventDefault(); event.returnValue = ""; } };
    window.addEventListener("beforeunload", unload);
    return () => { leaveGuard.current = null; window.removeEventListener("beforeunload", unload); };
  }, [dirty, leaveGuard]);
  useEffect(() => () => { pendingRef.current?.(false); }, []);
  useEffect(() => {
    if (!pendingLeave) return;
    const previousFocus = document.activeElement as HTMLElement | null;
    const dialog = dialogRef.current;
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !saving.current) { event.preventDefault(); resolveLeave(false); }
      if (event.key !== "Tab") return;
      const buttons = Array.from(dialog?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
      const first = buttons[0], last = buttons[buttons.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    };
    document.addEventListener("keydown", keydown);
    return () => { document.removeEventListener("keydown", keydown); previousFocus?.focus(); };
  }, [pendingLeave]);
  async function save() {
    if (saving.current) return false;
    saving.current = true; setBusy(true); setError(""); setMessage("");
    try {
      if (section === "events") {
        const preview = await invoke<EventStorageStats>("event_storage_statistics", { policy: draft.runtime.event_delivery });
        if (preview.expiring_events > 0 && !window.confirm(`保存后将清理 ${preview.expiring_events} 条过期事件，停止这些事件的重试。是否继续？`)) return false;
      }
      const document = await invoke<ConfigDocument>("save_settings_section", { section, changes });
      const saved = toUiConfig(document.config);
      latestConfig.current = saved;
      setOriginal(saved); setDraft(saved); onSaved(document);
      setMessage(`${selected.label}已保存`);
      return true;
    } catch (err) { setError(readError(err)); return false; }
    finally { saving.current = false; setBusy(false); }
  }
  function resolveLeave(allowed: boolean) {
    if (allowed) { setDraft(original); setError(""); setMessage(""); }
    pendingRef.current?.(allowed); pendingRef.current = null; setPendingLeave(null);
  }
  async function navigate(next: SettingsPageId) {
    if (next === section) return;
    if (await leaveGuard.current?.()) {
      setOriginal(latestConfig.current); setDraft(latestConfig.current); setSection(next); setError(""); setMessage(""); setShowAdvancedSettings(false);
    }
  }
  function update<G extends "device" | "platform" | "relay" | "runtime" | "upload", K extends keyof UiAgentConfig[G]>(group: G, key: K, value: UiAgentConfig[G][K]) {
    setMessage("");
    setDraft(current => ({ ...current, [group]: { ...current[group], [key]: value } }));
  }
  async function refreshPythonRuntime(pythonPath?: string | null) {
    setPythonCheckBusy(true); setError("");
    try { setPythonStatus(await invoke<PythonRuntimeStatus>("python_runtime_status", { pythonPath })); }
    catch (err) { setError(readError(err)); }
    finally { setPythonCheckBusy(false); }
  }
  return <div className="settings-workspace">
    <nav className="settings-navigation" aria-label="设置分类">
      <h2>客户端设置</h2>
      {settingsSections.map(item => <button key={item.id} className={item.id === section ? "active" : ""} aria-current={item.id === section ? "page" : undefined} disabled={busy} onClick={() => void navigate(item.id)}>{item.label}</button>)}
    </nav>
    <section className="settings-content" aria-labelledby="settings-heading">
      <header className="settings-heading"><h2 id="settings-heading">{selected.label}</h2><p>{selected.description}</p></header>
      {section === "general" ? <GeneralSettings /> : section === "about" ? renderAbout() : <>
        {section === "identity" ? <div className="settings-preference-row"><div><strong>账号授权</strong><p>{config.platform.workspace_id ? `当前工作区：${config.platform.workspace_id}` : "尚未授权"}</p></div><button className="secondary" disabled={busy} onClick={async () => { if (await leaveGuard.current?.()) await beginBrowserAuth(); }}>浏览器授权</button></div> : null}
        <fieldset disabled={busy} className="settings-fields">
          <SettingsSectionPanel activeSettingsSection={section} config={draft} configPath={configPath} openExternalUrl={openExternalUrl}
            pythonCheckBusy={pythonCheckBusy} pythonStatus={pythonStatus} refreshPythonRuntime={refreshPythonRuntime} setPythonStatus={setPythonStatus}
            setShowAdvancedSettings={setShowAdvancedSettings} showAdvancedSettings={showAdvancedSettings}
            updateDevice={(key, value) => update("device", key, value)} updatePlatform={(key, value) => update("platform", key, value)}
            updateRelay={(key, value) => update("relay", key, value)} updateRuntime={(key, value) => update("runtime", key, value)} updateUpload={(key, value) => update("upload", key, value)}
            useDetectedPython={() => { if (pythonStatus?.detectedPath) update("runtime", "python_path", pythonStatus.detectedPath); }} />
        </fieldset>
      </>}
      {error ? <div role="alert" className="alert error">{error}</div> : null}
      {message ? <p role="status">{message}</p> : null}
      {editable ? <footer className="settings-footer"><span>{dirty ? "有未保存的更改" : "所有更改已保存"}</span><button className="secondary" disabled={!dirty || busy} onClick={() => { setDraft(original); setError(""); }}>放弃更改</button><button className="primary" disabled={!dirty || busy} onClick={() => void save()}>{busy ? "保存中…" : "保存更改"}</button></footer> : null}
    </section>
    {pendingLeave ? <div className="settings-dialog-backdrop"><div ref={dialogRef} className="settings-leave-dialog" role="dialog" aria-modal="true" aria-labelledby="unsaved-heading">
      <h2 id="unsaved-heading">保存当前页面的更改？</h2><p>“{selected.label}”有未保存的更改，请保存或放弃后继续。</p>
      <div className="service-actions"><button autoFocus className="secondary" disabled={busy} onClick={() => resolveLeave(false)}>继续编辑</button><button className="secondary" disabled={busy} onClick={() => resolveLeave(true)}>放弃并继续</button><button className="primary" disabled={busy} onClick={async () => { if (await save()) { pendingRef.current?.(true); pendingRef.current = null; setPendingLeave(null); } }}>保存并继续</button></div>
      {error ? <p role="alert">{error}</p> : null}
    </div></div> : null}
  </div>;
}
