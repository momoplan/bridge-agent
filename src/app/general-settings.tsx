import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { readError } from "./formatters";

export function GeneralSettings() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const operation = useRef(false);
  async function readStatus() {
    if (operation.current) return;
    operation.current = true;
    setBusy(true);
    try {
      setEnabled(await invoke<boolean>("get_desktop_autostart"));
      setError("");
    } catch (err) {
      setEnabled(null);
      setError(readError(err));
    } finally { operation.current = false; setBusy(false); }
  }
  useEffect(() => {
    void readStatus();
    const refresh = () => { void readStatus(); };
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, []);
  async function toggle() {
    if (operation.current || enabled == null) return;
    operation.current = true;
    setBusy(true); setError(""); setMessage("");
    try {
      const actual = await invoke<boolean>("set_desktop_autostart", { enabled: !enabled });
      setEnabled(actual);
      setMessage(actual ? "已开启开机自启动" : "已关闭开机自启动");
    } catch (err) {
      setError(readError(err));
      // A system write can succeed while verification fails. Never display an assumed value.
      try { setEnabled(await invoke<boolean>("get_desktop_autostart")); }
      catch { setEnabled(null); }
    } finally { operation.current = false; setBusy(false); }
  }
  return <>
    <div className="settings-preference-row">
      <div><label id="autostart-label">开机自启动</label><p id="autostart-description">登录电脑后在后台运行百积木，不自动打开主窗口。</p></div>
      <button type="button" role="switch" aria-labelledby="autostart-label" aria-describedby="autostart-description" aria-checked={enabled === true}
        className={`settings-switch ${enabled ? "is-on" : ""}`} disabled={busy || enabled == null} onClick={() => void toggle()}><span /></button>
    </div>
    <p className="settings-help">更改立即生效。关闭后，重新打开或升级客户端不会自动开启。</p>
    {busy ? <p role="status">正在读取或更新系统设置…</p> : null}
    {message ? <p role="status">{message}</p> : null}
    {error ? <div className="alert error" role="alert"><span>{error}</span><button className="secondary" disabled={busy} onClick={() => void readStatus()}>重新读取</button></div> : null}
  </>;
}
