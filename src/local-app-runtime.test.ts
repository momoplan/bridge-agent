import { describe, expect, it } from "vitest";
import { embeddedLocalAppView } from "./local-app-runtime";

describe("embeddedLocalAppView", () => {
  it("mounts embedded UI only after the runtime is running", () => {
    expect(embeddedLocalAppView("running", false)).toBe("mounted");
    for (const state of ["installed", "stopped", "start_failed", "unknown", "broken"]) {
      expect(embeddedLocalAppView(state, false)).toBe("unavailable");
    }
  });

  it("keeps lifecycle transitions and uninstall isolated from the embedded UI", () => {
    expect(embeddedLocalAppView("starting", false)).toBe("transitioning");
    expect(embeddedLocalAppView("stopping", false)).toBe("transitioning");
    expect(embeddedLocalAppView("running", true)).toBe("uninstalling");
  });
});
