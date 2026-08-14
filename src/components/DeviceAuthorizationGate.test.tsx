import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { DeviceAuthorizationGate } from "./DeviceAuthorizationGate";

const handlers = {
  onAuthorize: vi.fn(),
  onCopyAuthorizationUrl: vi.fn(),
  onOpenAuthorizationUrl: vi.fn(),
  onOpenAuthorizationUrlInEdge: vi.fn(),
  onOpenDiagnostics: vi.fn()
};

describe("DeviceAuthorizationGate", () => {
  it("renders initial authorization as a locked product state", () => {
    const markup = renderToStaticMarkup(
      <DeviceAuthorizationGate
        state="unauthorized"
        workspaceId=""
        pendingAuthorization={null}
        busy={false}
        {...handlers}
      />
    );

    expect(markup).toContain("设备尚未授权");
    expect(markup).toContain("不会开放应用安装、运行或工作区调用");
    expect(markup).not.toContain("Agent 连接异常");
  });

  it("renders credential rejection separately from connectivity failures", () => {
    const markup = renderToStaticMarkup(
      <DeviceAuthorizationGate
        state="reauthorization_required"
        workspaceId="433"
        pendingAuthorization={null}
        busy={false}
        {...handlers}
      />
    );

    expect(markup).toContain("授权已失效");
    expect(markup).toContain("自动重连已停止");
    expect(markup).toContain("上次授权工作区：#433");
  });

  it("explains that polling, not the activation link, establishes authorization", () => {
    const markup = renderToStaticMarkup(
      <DeviceAuthorizationGate
        state="authorizing"
        workspaceId=""
        pendingAuthorization={{
          userCode: "ABCD-EFGH",
          verificationUriComplete: "https://example.com/device?user_code=ABCD-EFGH"
        }}
        busy={false}
        {...handlers}
      />
    );

    expect(markup).toContain("客户端会持续轮询授权结果");
    expect(markup).toContain("只有平台返回有效工作区和设备凭据后");
    expect(markup).toContain("ABCD-EFGH");
  });
});
