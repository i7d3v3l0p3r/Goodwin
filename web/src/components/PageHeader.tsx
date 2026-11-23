import { Show, type ParentProps } from "solid-js";
import { usePanel } from "../helpers/panel";
import { errorLabel } from "../helpers/words";
import { Button } from "./Button";
import { Icon } from "./icons";
import styles from "./PageHeader.module.css";

export function PageHeader(props: ParentProps<{ title: string }>) {
  const panel = usePanel();
  const muted = () => panel.snapshot()?.settings.microphone_muted === true;
  const paused = () => {
    const state = panel.snapshot();
    return muted() || !panel.reachable() || !state?.microphone || !state.settings.listening || !state.model;
  };

  return <header class={styles.header}>
    <h1>{props.title}</h1>
    <div class={styles.actions}>
      <span class={styles.status} role="status" data-paused={String(paused())}><i /><span>{panel.status()}</span></span>
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
