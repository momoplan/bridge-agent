import { expect, test } from "vitest";
import { verifyPermissionImports } from "./verify-macos-permission-symbols.mjs";

test("rejects a Catalina crash symbol in either universal architecture", () => {
  for (const symbol of ["PostEvent", "ListenEvent", "ScreenCapture"]) {
    expect(() => verifyPermissionImports(
      `app (for architecture x86_64):\n_AXIsProcessTrusted\n_CGPreflight${symbol}Access\napp (for architecture arm64):\n_dlsym\n`,
      "app",
    )).toThrow(/directly imports/);
  }
  for (const symbol of ["PostEvent", "ScreenCapture"]) {
    expect(() => verifyPermissionImports(`U _CGRequest${symbol}Access`, "app")).toThrow(/directly imports/);
  }
});

test("allows stable accessibility imports and dynamic lookup", () => {
  verifyPermissionImports("_AXIsProcessTrusted\n_AXIsProcessTrustedWithOptions\n_dlsym\n", "app");
});
