import { createMemo, createSignal, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import type { Settings, Snapshot } from "../../../helpers/api";
import { protectDraft } from "../../../helpers/drafts";
import type { Words } from "../../../helpers/words";
import { isSettingsPath } from "../sections";
import styles from "./MicrophoneGainForm.module.css";

export function MicrophoneGainForm(props: {
  t: Words;
  snapshot: Snapshot;
  pending: boolean;
  showWakeGain: boolean;
  save: (settings: Settings) => Promise<boolean>;
}) {
  const wakeGain = createMemo(() => props.snapshot.settings.wake_gain);
  const transcriptionGain = createMemo(() => props.snapshot.settings.transcription_gain);
  const [saved, setSaved] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  const [edited, setEdited] = createSignal({ wake_gain: false, transcription_gain: false });
  const dirty = () => edited().wake_gain || edited().transcription_gain;
  protectDraft(dirty, isSettingsPath);

  function clearStatus() {
    setSaved(false);
    setFailed(false);
  }

  function edit(field: keyof ReturnType<typeof edited>, input: HTMLInputElement) {
    const saved = props.snapshot.settings[field];
    const value = input.value === "" ? null : input.valueAsNumber;
    setEdited(previous => ({ ...previous, [field]: input.validity.badInput || value !== saved }));
    clearStatus();
  }

  async function save(event: SubmitEvent & { currentTarget: HTMLFormElement }) {
    event.preventDefault();
    if (props.pending) return;
    clearStatus();
    const fields = new FormData(event.currentTarget);
    const gain = (name: string) => {
      const value = fields.get(name);
      return typeof value === "string" && value !== "" ? Number(value) : null;
    };
    const ok = await props.save({
      ...props.snapshot.settings,
      wake_gain: props.showWakeGain ? gain("wake_gain") : props.snapshot.settings.wake_gain,
      transcription_gain: gain("transcription_gain"),
    });
    if (ok) setEdited({ wake_gain: false, transcription_gain: false });
    setSaved(ok);
    setFailed(!ok);
  }

  return <section aria-labelledby="microphone-settings-title">
    <h2 id="microphone-settings-title">{props.t.audioInput}</h2>
    <form class={styles.form} onSubmit={event => void save(event)}>
      <fieldset class={styles.gains} disabled={props.pending}>
        <Show when={props.showWakeGain}><label class={styles.field}>
          {props.t.wakeGain}
          <input type="number" inputmode="decimal" name="wake_gain" min={0.1} max={64} step="any"
            value={wakeGain() ?? ""} placeholder={String(props.snapshot.default_microphone_gain)}
            aria-describedby="wake-gain-hint" onInput={event => edit("wake_gain", event.currentTarget)} />
          <span id="wake-gain-hint" class={styles.hint}>{props.t.defaultGainHint}</span>
        </label></Show>
        <label class={styles.field}>
          {props.t.transcriptionGain}
          <input type="number" inputmode="decimal" name="transcription_gain" min={0.1} max={64} step="any"
            value={transcriptionGain() ?? ""} placeholder={props.t.automaticGain}
            aria-describedby="transcription-gain-hint" onInput={event => edit("transcription_gain", event.currentTarget)} />
          <span id="transcription-gain-hint" class={styles.hint}>{props.t.automaticGainHint}</span>
        </label>
      </fieldset>
      <div class="gw-row">
        <Button variant="primary" type="submit" disabled={props.pending}>{props.t.save}</Button>
        <Show when={saved()}><span class={styles.saved} role="status">{props.t.saved}</span></Show>
        <Show when={failed()}><span class={styles.error} role="alert">{props.t.errors.save_failed}</span></Show>
        <Show when={dirty() && !failed()}><span class={styles.hint}>{props.t.unsaved}</span></Show>
      </div>
    </form>
  </section>;
}
