import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";
export function ApplicationIcon(props: {
  className: string;
  name: string;
  src?: string | null;
}) {
  const fallback = props.name.trim().slice(0, 1).toLocaleUpperCase() || "应";
  return (
    <span className={`application-icon ${props.className}`} aria-hidden="true">
      {props.src ? <img src={props.src} alt="" /> : fallback}
    </span>
  );
}

export function handleTabListKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>) {
  if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
    return;
  }
  const tabList = event.currentTarget.closest<HTMLElement>("[role='tablist']");
  const tabs = tabList
    ? Array.from(tabList.querySelectorAll<HTMLButtonElement>("[role='tab']:not(:disabled)"))
    : [];
  const currentIndex = tabs.indexOf(event.currentTarget);
  if (currentIndex < 0 || tabs.length === 0) {
    return;
  }
  event.preventDefault();
  const nextIndex =
    event.key === "Home"
      ? 0
      : event.key === "End"
        ? tabs.length - 1
        : (currentIndex + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
  tabs[nextIndex]?.focus();
  tabs[nextIndex]?.click();
}

export function Field(props: {
  label: string;
  children: JSX.Element;
  wide?: boolean;
  hint?: string;
}) {
  return (
    <label className={`field ${props.wide ? "field-wide" : ""}`}>
      <span>{props.label}</span>
      {props.children}
      {props.hint ? <small className="field-hint">{props.hint}</small> : null}
    </label>
  );
}

export function Card(props: {
  title: string;
  description?: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <section className="card">
      <div className="card-head">
        <div>
          <h2>{props.title}</h2>
          {props.description ? <p>{props.description}</p> : null}
        </div>
        {props.action ?? null}
      </div>
      {props.children}
    </section>
  );
}

export function InfoRow(props: {
  label: string;
  value: string;
  tone?: "normal" | "warning" | "danger";
}) {
  const valueClass =
    props.tone === "danger" ? "danger-text" : props.tone === "warning" ? "warning-text" : "";
  return (
    <div className="info-row">
      <span>{props.label}</span>
      <strong className={valueClass}>{props.value}</strong>
    </div>
  );
}
