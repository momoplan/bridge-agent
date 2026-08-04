import { describe, expect, it } from "vitest";
import { describeLocalAppUpdate } from "./local-app-updates";

describe("describeLocalAppUpdate", () => {
  it("combines publisher notes with capability and permission differences", () => {
    const changes = describeLocalAppUpdate(
      {
        methodNames: ["message.list", "message.send"],
        eventNames: ["message.received"],
        permissions: [{ id: "network", title: "网络访问" }]
      },
      {
        releaseNotes: ["支持发送文件", "支持发送文件", "  "],
        methodNames: ["message.send", "file.send"],
        eventNames: ["message.received", "file.received"],
        permissions: [
          { id: "network", title: "网络访问" },
          { id: "filesystem", title: "文件读取" }
        ]
      }
    );

    expect(changes).toEqual({
      releaseNotes: ["支持发送文件"],
      addedCapabilities: ["事件：file.received", "方法：file.send"],
      removedCapabilities: ["方法：message.list"],
      addedPermissions: ["文件读取"],
      removedPermissions: [],
      hasSpecificChanges: true
    });
  });

  it("does not invent details when a publisher supplied no notes or structural changes", () => {
    const changes = describeLocalAppUpdate(
      { methodNames: ["ping"], eventNames: [], permissions: [] },
      { releaseNotes: [], methodNames: ["ping"], eventNames: [], permissions: [] }
    );

    expect(changes.hasSpecificChanges).toBe(false);
    expect(changes.releaseNotes).toEqual([]);
  });
});
