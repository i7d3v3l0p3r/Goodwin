import { createMemo, createSignal } from "solid-js";
import type { Language, Words } from "../../../helpers/words";
import styles from "./Preferences.module.css";

export function Preferences(props: {
  t: Words;
  language: Language;
  listening: boolean;
  sensitivity: number;
  pending: boolean;
  changeLanguage: (language: Language) => void;
  toggleListening: () => void;
  changeSensitivity: (value: number) => void;
}) {
  const savedSensitivity = createMemo(() => props.sensitivity);
  const [sensitivity, setSensitivity] = createSignal(savedSensitivity);
  return <section aria-labelledby="preferences-title">
    <h2 id="preferences-title">{props.t.preferences}</h2>
    <div class={styles.row}>
      <label for="language">{props.t.language}</label>
      <select id="language" value={props.language} onChange={(event) => props.changeLanguage(event.currentTarget.value as Language)}>
        <option value="en">English</option><option value="ru">Русский</option>
      </select>
    </div>
    <div class={styles.row}>
      <label for="sensitivity">{props.t.sensitivity} <output>{Math.round(sensitivity() * 100)}%</output></label>
      <input id="sensitivity" type="range" min={0} max={1} step={0.01} value={sensitivity()} disabled={props.pending}
        onInput={event => setSensitivity(event.currentTarget.valueAsNumber)}
        onChange={event => props.changeSensitivity(event.currentTarget.valueAsNumber)} />
    </div>
    <div class={styles.row}>
      <span id="listening-label">{props.t.listening}</span>
      <button type="button" role="switch" class={styles.listeningSwitch} aria-labelledby="listening-label"
        aria-checked={props.listening ? "true" : "false"} disabled={props.pending}
        onClick={props.toggleListening}>
        <span />
      </button>
    </div>
  </section>;
}
