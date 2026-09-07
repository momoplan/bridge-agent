import React, { Component, useEffect, type ErrorInfo, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles.css";
import "./design-system.css";
import "./desktop.css";

interface BoundaryProps { children: ReactNode; onFailure: (error: unknown) => void; }
export class BusinessBoundary extends Component<BoundaryProps, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(error: Error, _info: ErrorInfo) { this.props.onFailure(error); }
  render() { return this.state.failed ? null : this.props.children; }
}
function Mounted({ onReady }: { onReady: () => void }) {
  useEffect(onReady, [onReady]);
  return <App />;
}
export function mountBusiness(root: HTMLElement, ready: () => void, fail: (error: unknown) => void) {
  const renderer = createRoot(root);
  renderer.render(
    <React.StrictMode>
      <BusinessBoundary onFailure={fail}><Mounted onReady={ready} /></BusinessBoundary>
    </React.StrictMode>
  );
  return () => renderer.unmount();
}
