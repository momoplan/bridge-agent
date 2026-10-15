import { useState } from "react";
import type { LocalAppItem } from "./types";

export function PrivateSourceSyncPanel({ app, busy, syncLocalApp }: {
  app: LocalAppItem;
  busy: boolean;
  syncLocalApp: (app: LocalAppItem, targetVersion: string) => Promise<void>;
}) {
  const [version, setVersion] = useState(app.connector?.version ?? "");
  const source = app.connector?.installSource;
  if (source?.kind !== "environment") {
    return <p role="alert">当前安装缺少来源环境身份，请从来源环境重新选择安装版本。</p>;
  }
  return (
    <form className="market-permission-card" aria-label="选择来源环境版本" onSubmit={(event) => {
      event.preventDefault();
      void syncLocalApp(app, version);
    }}>
      <div>
        <strong>同步或升级来源环境版本</strong>
        <p>来源：{source.source.application.environmentKey} / {source.source.application.appId}</p>
        <p>当前版本：{app.connector!.version}。请输入要安装的已发布版本；同步当前版本会重新安装该版本。</p>
        <label>
          目标版本
          <input required value={version} disabled={busy} onChange={(event) => setVersion(event.target.value)} autoComplete="off" spellCheck={false} />
        </label>
        <p>此来源未公开审核。安装时会校验所选版本的授权、清单与平台兼容性。</p>
        <button className="secondary" type="submit" disabled={busy}>
          {busy ? "处理中…" : "确认所选版本并同步"}
        </button>
      </div>
    </form>
  );
}
