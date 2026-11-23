import { createMemo, createSignal } from "solid-js";
import type { Words } from "../../../helpers/words";
import styles from "./WakeWordSettings.module.css";

export function WakeWordSettings(props: {
  t: Words;
  listening: boolean;
  sensitivity: number;
  pending: boolean;
  toggleListening: () => void;
  changeSensitivity: (value: number) => void;
}) {
  const savedSensitivity = createMemo(() => props.sensitivity);
  const [sensitivity, setSensitivity] = createSignal(savedSensitivity);
  return <section aria-labelledby="wake-word-title">
    <h3 id="wake-word-title">{props.t.wakeWord}</h3>
    <div class={styles.row}>
      <span id="listening-label">{props.t.listening}</span>
      <button type="button" role="switch" class={styles.listeningSwitch} aria-labelledby="listening-label"
        aria-checked={props.listening ? "true" : "false"} disabled={props.pending}
        onClick={props.toggleListening}>
        <span />
      </button>
    </div>
    <label class={styles.sensitivity} for="sensitivity">
      <span>{props.t.sensitivity} <output aria-hidden="true">{Math.round(sensitivity() * 100)}%</output></span>
      <input id="sensitivity" type="range" min={0} max={1} step={0.01} value={sensitivity()} disabled={props.pending}
        aria-valuetext={`${Math.round(sensitivity() * 100)}%`}
        onInput={event => setSensitivity(event.currentTarget.valueAsNumber)}
        onChange={event => props.changeSensitivity(event.currentTarget.valueAsNumber)} />
    </label>
  </section>;
}
