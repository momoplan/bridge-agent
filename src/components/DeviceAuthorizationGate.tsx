import { Activity, ExternalLink, ShieldCheck } from "lucide-react";
import type { DeviceAuthorizationState } from "../device-authorization-state";

export interface PendingBrowserAuthorization {
  userCode: string;
  verificationUriComplete: string;
}

interface DeviceAuthorizationGateProps {
  state: Exclude<DeviceAuthorizationState, "authorized">;
  workspaceId: string;
  pendingAuthorization: PendingBrowserAuthorization | null;
  busy: boolean;
  onAuthorize: () => void;
  onCopyAuthorizationUrl: () => void;
  onOpenAuthorizationUrl: () => void;
  onOpenAuthorizationUrlInEdge: () => void;
  onOpenDiagnostics: () => void;
}

const STATE_COPY = {
  unauthorized: {
    eyebrow: "设备尚未授权",
    title: "授权后才能使用百积木本地能力",
    description:
      "当前设备没有工作区身份或 Relay 凭据。授权完成前不会启动 Agent，也不会开放应用安装、运行或工作区调用。",
    action: "去授权"
  },
  authorizing: {
    eyebrow: "正在授权",
    title: "请在浏览器中完成设备授权",
    description:
      "客户端会持续轮询授权结果；只有平台返回有效工作区和设备凭据后，才会解除能力锁定并启动 Agent。",
    action: "重新打开授权页"
  },
  reauthorization_required: {
    eyebrow: "授权已失效",
    title: "需要重新授权这台设备",
    description:
      "Relay 已拒绝当前设备凭据，自动重连已停止。重新授权会轮换凭据并恢复工作区连接。",
    action: "重新授权"
  }
} as const;

export function DeviceAuthorizationGate({
  state,
  workspaceId,
  pendingAuthorization,
  busy,
  onAuthorize,
  onCopyAuthorizationUrl,
  onOpenAuthorizationUrl,
  onOpenAuthorizationUrlInEdge,
  onOpenDiagnostics
}: DeviceAuthorizationGateProps) {
  const copy = STATE_COPY[state];

  return (
    <section className={`device-authorization-gate state-${state}`} aria-labelledby="authorization-title">
      <div className="device-authorization-icon" aria-hidden="true">
        <ShieldCheck size={30} strokeWidth={1.7} />
      </div>
      <div className="device-authorization-copy">
        <span>{copy.eyebrow}</span>
        <h2 id="authorization-title">{copy.title}</h2>
        <p>{copy.description}</p>
        {state === "reauthorization_required" && workspaceId ? (
          <small>上次授权工作区：#{workspaceId}</small>
        ) : null}
      </div>

      {state === "authorizing" && pendingAuthorization ? (
        <div className="device-authorization-session" role="status">
          <div>
            <span>用户码</span>
            <strong>{pendingAuthorization.userCode}</strong>
          </div>
          <input
            aria-label="授权链接"
            readOnly
            value={pendingAuthorization.verificationUriComplete}
            onFocus={(event) => event.currentTarget.select()}
          />
          <div className="device-authorization-actions">
            <button className="primary" onClick={onOpenAuthorizationUrl} disabled={busy}>
              <ExternalLink size={15} aria-hidden="true" />
              默认浏览器打开
            </button>
            <button className="secondary" onClick={onOpenAuthorizationUrlInEdge} disabled={busy}>
              用 Edge 打开
            </button>
            <button className="secondary" onClick={onCopyAuthorizationUrl} disabled={busy}>
              复制链接
            </button>
          </div>
        </div>
      ) : (
        <div className="device-authorization-actions">
          <button className="primary" onClick={onAuthorize} disabled={busy}>
            <ShieldCheck size={15} aria-hidden="true" />
            {copy.action}
          </button>
          <button className="secondary" onClick={onOpenDiagnostics}>
            <Activity size={15} aria-hidden="true" />
            查看诊断
          </button>
        </div>
      )}

      <p className="device-authorization-boundary">
        “未授权”表示没有可用的平台身份；“连接异常”只会在授权有效但网络或 Relay 不可用时显示。
      </p>
    </section>
  );
}
