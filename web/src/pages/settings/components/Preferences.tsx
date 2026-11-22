import type { Language, Words } from "../../../helpers/words";
import styles from "./Preferences.module.css";

export function Preferences(props: {
  t: Words;
  language: Language;
  listening: boolean;
  pending: boolean;
  changeLanguage: (language: Language) => void;
  toggleListening: () => void;
}) {
  return <section aria-labelledby="preferences-title">
    <h2 id="preferences-title">{props.t.preferences}</h2>
    <div class={styles.row}>
      <label for="language">{props.t.language}</label>
      <select id="language" value={props.language} onChange={(event) => props.changeLanguage(event.currentTarget.value as Language)}>
        <option value="en">English</option><option value="ru">Русский</option>
      </select>
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
