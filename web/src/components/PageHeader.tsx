import type { ParentProps } from "solid-js";
import { usePanel } from "../helpers/panel";
import styles from "./PageHeader.module.css";

export function PageHeader(props: ParentProps<{ title: string }>) {
  const panel = usePanel();
  const paused = () => {
    const state = panel.snapshot();
    return !panel.reachable() || !state?.microphone || !state.settings.listening || !state.model;
  };

  return <header class={styles.header}>
    <h1>{props.title}</h1>
    <div class={styles.actions}>
      <span class={styles.status} role="status" data-paused={String(paused())}><i /><span>{panel.status()}</span></span>
      {props.children}
    </div>
  </header>;
}
