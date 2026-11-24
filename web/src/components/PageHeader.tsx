import { createEffect, Show, type ParentProps } from "solid-js";
import { usePanel } from "../helpers/panel";
import { errorLabel } from "../helpers/words";
import { Button } from "./ui-kit/Button";
import { Icon } from "./icons";
import styles from "./PageHeader.module.css";

export function PageHeader(props: ParentProps<{ title: string; showRequestStatus?: boolean }>) {
  const panel = usePanel();
  const muted = () => panel.snapshot()?.settings.microphone_muted === true;
  createEffect(() => props.title, title => { document.title = `${title} · Goodwin`; });

  return <header class={styles.header}>
    <h1 id="panel-page-title" tabindex={-1}>{props.title}</h1>
    <span class={styles.status} role="status" data-paused={String(!panel.wakeReady())}
      data-local-status={props.showRequestStatus === false && panel.reachable() ? "" : undefined}>
      <i aria-hidden="true" /><span>{panel.status()}</span>
    </span>
    <div class={styles.actions}>
      {props.children}
      <div class={styles.microphoneControl}>
        <Button type="button" class={styles.microphone} aria-label={panel.t().muteMicrophone}
          aria-pressed={muted() ? "true" : "false"} title={muted() ? panel.t().unmuteMicrophone : panel.t().muteMicrophone}
          disabled={!panel.snapshot() || !panel.reachable() || panel.microphonePending()}
          onClick={() => void panel.setMicrophoneMuted(!muted())}>
          <Icon name={muted() ? "micOff" : "mic"} size={19} />
        </Button>
        <Show when={panel.microphoneError()}>
          <p class={styles.microphoneError} role="alert">{errorLabel(panel.t(), panel.microphoneError())}</p>
        </Show>
      </div>
    </div>
  </header>;
}
