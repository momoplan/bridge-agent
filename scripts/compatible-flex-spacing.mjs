import flexGapPolyfill from "flex-gap-polyfill";

export function compatibleFlexSpacing() {
  return { postcssPlugin: "compatible-flex-spacing", Once(root, helpers) {
    const flex = new Set(), otherLayouts = new Set(), margins = new Set();
    root.walkRules((rule) => {
      for (const node of rule.nodes) {
        if (node.prop === "display") {
          const set = ["flex", "inline-flex"].includes(node.value) ? flex : otherLayouts;
          for (const selector of rule.selectors) set.add(selector);
        }
        if (node.prop?.startsWith("margin")) for (const selector of rule.selectors) margins.add(selector);
      }
    });
    // Grid already supports gap on Catalina. Never rewrite its gaps or let
    // inherited Flex flags disable them, including Grid nested inside Flex.
    const only = [...new Set([...flex, ...margins])].filter((selector) => !otherLayouts.has(selector));
    const plugin = flexGapPolyfill({ only, flexGapNotSupported: ".flex-gap-not-supported" });
    plugin.Once(root, helpers);
  } };
}
