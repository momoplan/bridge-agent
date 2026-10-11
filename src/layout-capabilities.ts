// CSS.supports("gap", "1px") also succeeds for Grid on Catalina. Measure Flex.
export function initializeLayoutCapabilities(document: Document) {
  const probe = document.createElement("div");
  probe.style.cssText = "position:absolute;visibility:hidden;display:flex;flex-direction:column;row-gap:1px;padding:0;border:0;";
  for (let index = 0; index < 2; index += 1) {
    const child = document.createElement("div");
    child.style.cssText = "height:1px;flex:none;margin:0;padding:0;border:0;";
    probe.appendChild(child);
  }
  document.body.appendChild(probe);
  document.documentElement.classList.toggle("flex-gap-not-supported", probe.scrollHeight !== 3);
  probe.remove();
}
