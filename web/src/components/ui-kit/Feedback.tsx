import type { ParentProps } from "solid-js";

export function StatusIndicator(props: ParentProps<{ tone: "neutral" | "pending" | "success" | "error" }>) {
  return <span class="gw-status" data-tone={props.tone}>{props.children}</span>;
}

export function InlineError(props: ParentProps<{ announce?: boolean }>) {
  return <p class="gw-inline-error" role={props.announce === false ? undefined : "alert"}>{props.children}</p>;
}

export function EmptyState(props: ParentProps) {
  return <p class="gw-empty-state">{props.children}</p>;
}
