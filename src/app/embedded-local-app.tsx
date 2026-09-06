import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { clientInfo } from "../client-logger";
import { readError } from "./formatters";
import type { LocalAppDetailTab, LocalAppItem } from "./types";
export function reportLocalAppUiHandshake(
  appId: string,
  event: string,
  detail: Record<string, string | number | boolean | null> = {},
) {
  clientInfo("local_app_ui", { appId, event, ...detail });
}

export function LocalAppEmbeddedUi(props: { appId: string; title: string }) {
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const iframeLoadedRef = useRef(false);
  const uiOriginRef = useRef("");
  const [uiUrl, setUiUrl] = useState("");
  const [loadError, setLoadError] = useState("");
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let active = true;
    setUiUrl("");
    iframeLoadedRef.current = false;
    uiOriginRef.current = "";
    setLoadError("");
    setReady(false);
    reportLocalAppUiHandshake(props.appId, "url_request_started");
    void invoke<string>("connector_app_ui_url", { id: props.appId })
      .then((url) => {
        if (!active) return;
        const parsed = new URL(url);
        uiOriginRef.current = parsed.origin;
        setUiUrl(parsed.toString());
        reportLocalAppUiHandshake(props.appId, "url_resolved", { origin: parsed.origin });
      })
      .catch((error) => {
        if (active) {
          const message = readError(error);
          reportLocalAppUiHandshake(props.appId, "url_request_failed");
          setLoadError(message);
        }
      });
    return () => {
      active = false;
    };
  }, [props.appId]);

  useEffect(() => {
    const handleMessage = (event: MessageEvent<unknown>) => {
      const target = iframeRef.current?.contentWindow;
      const uiOrigin = uiOriginRef.current;
      const sourceMatches = Boolean(target && event.source === target);
      const originMatches = Boolean(uiOrigin && event.origin === uiOrigin);
      const messageType = readLocalAppBridgeMessageType(event.data);
      if (sourceMatches || originMatches || messageType !== "unknown") {
        reportLocalAppUiHandshake(props.appId, "message_received", {
          actualOrigin: event.origin,
          expectedOrigin: uiOrigin || null,
          sourcePresent: event.source !== null,
          sourceMatches,
          originMatches,
          messageType,
        });
      }
      if (!target || !uiOrigin || !sourceMatches || !originMatches) return;
      if (!isLocalAppBridgeMessage(event.data)) return;
      if (event.data.type === "baijimu:local-app:ready") {
        reportLocalAppUiHandshake(props.appId, "ready_accepted");
        setReady(true);
        return;
      }
      const request = event.data;
      void invoke<unknown>("invoke_connector_management", {
        id: props.appId,
        operation: request.operation,
        payload: request.payload ?? null
      })
        .then((data) => {
          target.postMessage(
            {
              type: "baijimu:local-app:response",
              version: 1,
              requestId: request.requestId,
              ok: true,
              data
            },
            uiOrigin
          );
        })
        .catch((error) => {
          target.postMessage(
            {
              type: "baijimu:local-app:response",
              version: 1,
              requestId: request.requestId,
              ok: false,
              error: readError(error)
            },
            uiOrigin
          );
        });
    };
    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, [props.appId]);

  useEffect(() => {
    if (!uiUrl || ready) return;
    const timer = window.setTimeout(() => {
      reportLocalAppUiHandshake(props.appId, "ready_timeout", {
        expectedOrigin: uiOriginRef.current || null,
        iframePresent: iframeRef.current !== null,
        iframeLoaded: iframeLoadedRef.current,
      });
      setLoadError(
        iframeLoadedRef.current
          ? "应用界面已加载，但通信桥接未就绪。请在“诊断 > 日志”中查看 local_app_ui 记录。"
          : "应用界面未能从本机 UI 服务加载。请在“诊断 > 日志”中查看 local_app_ui 记录。",
      );
    }, 10000);
    return () => window.clearTimeout(timer);
  }, [ready, uiUrl]);

  if (loadError) {
    return <div className="error-banner">加载应用界面失败：{loadError}</div>;
  }
  if (!uiUrl) {
    return <div className="empty-state">正在加载应用界面…</div>;
  }
  return (
    <div className="embedded-local-app-frame-shell">
      {!ready ? <div className="embedded-local-app-loading">正在启动 {props.title}…</div> : null}
      <iframe
        ref={iframeRef}
        className="embedded-local-app-frame"
        src={uiUrl}
        title={props.title}
        sandbox="allow-forms allow-same-origin allow-scripts"
        referrerPolicy="no-referrer"
        onLoad={() => {
          iframeLoadedRef.current = true;
          setLoadError("");
          const target = iframeRef.current?.contentWindow;
          const uiOrigin = uiOriginRef.current;
          reportLocalAppUiHandshake(props.appId, "iframe_loaded", {
            expectedOrigin: uiOrigin || null,
            targetPresent: target !== null && target !== undefined,
          });
          if (target && uiOrigin) {
            target.postMessage({ type: "baijimu:local-app:hello", version: 1 }, uiOrigin);
            reportLocalAppUiHandshake(props.appId, "hello_sent", { targetOrigin: uiOrigin });
          }
        }}
      />
    </div>
  );
}

type LocalAppBridgeMessage =
  | { type: "baijimu:local-app:ready"; version: 1 }
  | {
      type: "baijimu:local-app:invoke";
      version: 1;
      requestId: string;
      operation: string;
      payload?: unknown;
    };

export function isLocalAppBridgeMessage(value: unknown): value is LocalAppBridgeMessage {
  if (!value || typeof value !== "object") return false;
  const message = value as Partial<LocalAppBridgeMessage> & Record<string, unknown>;
  if (message.version !== 1) return false;
  if (message.type === "baijimu:local-app:ready") return true;
  return (
    message.type === "baijimu:local-app:invoke" &&
    typeof message.requestId === "string" &&
    message.requestId.length > 0 &&
    message.requestId.length <= 128 &&
    typeof message.operation === "string" &&
    /^[A-Za-z0-9._-]{1,128}$/.test(message.operation)
  );
}

export function readLocalAppBridgeMessageType(value: unknown): string {
  if (!value || typeof value !== "object") return "unknown";
  const message = value as Record<string, unknown>;
  return typeof message.type === "string" && message.type.startsWith("baijimu:local-app:")
    ? message.type.slice("baijimu:local-app:".length, 48)
    : "unknown";
}

export function defaultLocalAppDetailTab(app: LocalAppItem): LocalAppDetailTab {
  return app.connector?.ui?.type === "embedded" && app.connector.ui.defaultView
    ? "app"
    : "overview";
}
