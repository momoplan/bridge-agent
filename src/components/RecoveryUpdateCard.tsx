import type { ReactNode } from "react";

export type RecoveryUpdateCheckState = "checking" | "ready" | "error";

interface RecoveryUpdateCardProps {
  checkState: RecoveryUpdateCheckState;
  checkError: string | null;
  currentVersion: string | null;
  targetVersion: string | null;
  updateAvailable: boolean;
  autoDownloadAvailable: boolean;
  releaseUrl: string | null;
  updateBusy: boolean;
  updateBusyLabel: string;
  progress?: ReactNode;
  onInstall: () => void;
  onOpenRelease: () => void;
  onCheck: () => void;
}

interface ConfigLoadFailurePanelProps {
  error: string;
  recoveryUpdateCard: ReactNode;
  recoveryBusy: boolean;
  onRetry: () => void;
  onRecoverDefaults: () => void;
}

function recoveryUpdateMessage(props: RecoveryUpdateCardProps): string {
  if (props.updateAvailable) {
    return `发现 ${props.targetVersion ?? "新版本"}，可直接升级；现有配置会保留并由新版本迁移。`;
  }
  if (props.checkState === "checking") {
    return "正在通过独立更新通道检查新版本，此过程不读取业务配置。";
  }
  if (props.checkState === "error") {
    return `检查失败：${props.checkError ?? "更新服务暂时不可用"}`;
  }
  return `当前版本 ${props.currentVersion ?? "-"}，未发现可用更新。`;
}

export function RecoveryUpdateCard(props: RecoveryUpdateCardProps) {
  const checking = props.checkState === "checking";

  return (
    <div className="startup-update-card config-recovery-update-card">
      <div>
        <strong>客户端更新</strong>
        <p>{recoveryUpdateMessage(props)}</p>
      </div>
      <div className="startup-update-actions">
        {props.updateAvailable && props.autoDownloadAvailable ? (
          <button className="primary" onClick={props.onInstall} disabled={props.updateBusy}>
            {props.updateBusy
              ? props.updateBusyLabel
              : `安装 ${props.targetVersion ?? "最新版本"}（保留配置）`}
          </button>
        ) : props.updateAvailable && props.releaseUrl ? (
          <button className="primary" onClick={props.onOpenRelease}>
            打开官方下载页
          </button>
        ) : null}
        <button
          className="secondary"
          onClick={props.onCheck}
          disabled={props.updateBusy || checking}
        >
          {checking ? "检查中" : "重新检查"}
        </button>
      </div>
      {props.progress}
    </div>
  );
}

export function ConfigLoadFailurePanel(props: ConfigLoadFailurePanelProps) {
  return (
    <section className="loading-panel">
      <p className="eyebrow">百积木</p>
      <h1>配置需要处理</h1>
      <p>读取配置和运行状态时发生错误。</p>
      <div className="alert error">{props.error}</div>
      <p className="loading-hint">
        配置读取失败不会阻止官方更新。请优先安装可用更新，更新完成后会保留并迁移原配置。
      </p>
      {props.recoveryUpdateCard}
      <div className="loading-actions">
        <button className="secondary" onClick={props.onRetry} disabled={props.recoveryBusy}>
          重试加载
        </button>
        <button
          className="secondary danger"
          onClick={props.onRecoverDefaults}
          disabled={props.recoveryBusy}
        >
          {props.recoveryBusy ? "恢复中" : "归档并恢复默认配置"}
        </button>
      </div>
      <p className="loading-hint">
        恢复默认配置只应在更新仍无法修复时使用；操作前会先归档当前配置文件。
      </p>
    </section>
  );
}
