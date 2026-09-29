import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Field } from "./ui-primitives";
import type { EventStoragePolicy, EventStorageStats } from "./types/runtime";

const mib = 1024 * 1024;
const bytes = (value: number) => `${(value / mib).toFixed(2)} MiB`;
const date = (value: number | null) => value === null ? "—" : new Date(value * 1000).toLocaleString();

export function EventStorageSettings({ policy, onChange }: {
  policy: EventStoragePolicy;
  onChange: (value: EventStoragePolicy) => void;
}) {
  const [stats, setStats] = useState<EventStorageStats | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    const refresh = async () => {
      try {
        const next = await invoke<EventStorageStats>("event_storage_statistics", { policy });
        if (active) { setStats(next); setError(""); }
      } catch (err) { if (active) setError(String(err)); }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => { active = false; window.clearInterval(timer); };
  }, [policy]);
  const update = <K extends keyof EventStoragePolicy>(key: K, value: EventStoragePolicy[K]) => onChange({ ...policy, [key]: value });
  return <div className="form-grid">
    <Field label="自动清理" hint="默认开启。已确认事件立即删除正文；未确认事件保留到所设期限，过期后停止重试。" wide>
      <label className="checkbox-row"><input type="checkbox" checked={policy.automatic_cleanup}
        onChange={e => update("automatic_cleanup", e.target.checked)} />自动清理过期事件</label>
    </Field>
    <Field label="未确认事件保留天数" hint="从本机接收时间开始计算。关闭自动清理后，未确认事件持续重试，直到队列容量用完。">
      <input type="number" min={1} max={3650} value={policy.retention_days} disabled={!policy.automatic_cleanup}
        onChange={e => update("retention_days", Number(e.target.value))} />
    </Field>
    <Field label="事件队列容量（MiB）" hint="达到容量后暂停接收新事件，保留已有未过期事件。">
      <input type="number" min={1} max={102400} value={policy.capacity_bytes / mib}
        onChange={e => update("capacity_bytes", Math.round(Number(e.target.value) * mib))} />
    </Field>
    <Field label="诊断摘要保留天数" hint="摘要不包含事件正文，最多保留最近 10,000 条。日志仍使用运行设置中的轮转规则。">
      <input type="number" min={1} max={3650} value={policy.diagnostic_retention_days}
        onChange={e => update("diagnostic_retention_days", Number(e.target.value))} />
    </Field>
    {error && <p role="alert">无法读取事件存储：{error}</p>}
    {stats && <div className="field-wide" aria-live="polite">
      <p>待确认 {stats.pending_events} 条 · 队列占用 {bytes(stats.logical_bytes)} · 磁盘占用 {bytes(stats.physical_bytes)}</p>
      <p>最早接收：{date(stats.oldest_received_at)}</p>
      <p>上次清理：{date(stats.last_cleanup_at)}，过期清理 {stats.last_cleanup_expired} 条</p>
      <p>保存此设置后，将清理 {stats.expiring_events} 条已过期事件（{bytes(stats.expiring_bytes)}）。</p>
      {stats.logical_bytes > policy.capacity_bytes && <p>当前积压超过所设容量。保存后暂停接收新事件，已有事件继续投递。</p>}
      <p>数据库会复用已释放空间；磁盘占用包含索引和日志，可能高于队列占用。</p>
    </div>}
  </div>;
}
