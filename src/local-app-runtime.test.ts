import { describe, expect, it } from "vitest";
import { embeddedLocalAppView } from "./local-app-runtime";

describe("embeddedLocalAppView", () => {
  it("mounts embedded UI only after the authoritative lifecycle is ready", () => {
    expect(embeddedLocalAppView("ready", false)).toBe("mounted");
    for (const state of ["absent", "stopped", "degraded", "failed"]) {
      expect(embeddedLocalAppView(state, false)).toBe("unavailable");
    }
  });

  it("keeps lifecycle transitions and uninstall isolated from the embedded UI", () => {
    expect(embeddedLocalAppView("starting", false)).toBe("transitioning");
    expect(embeddedLocalAppView("stopping", false)).toBe("transitioning");
    expect(embeddedLocalAppView("upgrading", false)).toBe("transitioning");
    expect(embeddedLocalAppView("ready", true)).toBe("uninstalling");
    expect(embeddedLocalAppView("uninstalling", false)).toBe("uninstalling");
  });
});
