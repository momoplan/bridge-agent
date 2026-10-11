import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { compatibleFlexSpacing } from "./scripts/compatible-flex-spacing.mjs";
import { readFileSync } from "node:fs";

const compatibility = JSON.parse(readFileSync(new URL("./config/client-compatibility.json", import.meta.url), "utf8"));

export default defineConfig({
  // Catalina 10.15.5 ships WebKit/Safari 13.1.1.
  build: { target: compatibility.webview.javascript, cssTarget: compatibility.webview.css },
  css: { postcss: { plugins: [
    // The polyfill prefixes a whole selector list only once. Split lists first
    // so compatibility styles never leak into browsers with native Flex gap.
    { postcssPlugin: "split-layout-selectors", Once(root) {
      root.walkRules((rule) => {
        if (rule.selectors.length < 2) return;
        for (const selector of rule.selectors) rule.cloneBefore({ selector });
        rule.remove();
      });
    } },
    compatibleFlexSpacing()
  ] } },
  plugins: [react()],
  server: {
    port: 1420,
    strictPort: true
  },
  clearScreen: false
});
