import { createMemo, createSignal, For, Show } from "solid-js";
import { Button } from "../../../components/Button";
import type { Models, Snapshot, Settings } from "../../../helpers/api";
import type { Words } from "../../../helpers/words";
import styles from "./OpenRouterForm.module.css";

const modelFields = ["transcription", "chat", "speech", "voice"] as const;
const modelLabels = { transcription: "transcription", chat: "chatModel", speech: "speech", voice: "voice" } as const;

export function OpenRouterForm(props: {
  t: Words;
  snapshot: Snapshot;
  pending: boolean;
  error: string;
  save: (settings: Settings, key?: string) => Promise<boolean>;
}) {
  const savedModels = createMemo(() => props.snapshot.settings.models, {
    equals: (previous, next) => modelFields.every((field) => previous[field] === next[field]),
  });
  const savedSensitivity = createMemo(() => props.snapshot.settings.sensitivity);
  const [models, setModels] = createSignal<Models>(savedModels);
  const [sensitivity, setSensitivity] = createSignal(savedSensitivity);
  const [key, setKey] = createSignal("");
  const [removeKey, setRemoveKey] = createSignal(false);
  const [saved, setSaved] = createSignal(false);

  async function save() {
    setSaved(false);
    const settings = { ...props.snapshot.settings, models: models(), sensitivity: sensitivity() };
    if (await props.save(settings, removeKey() ? "" : key().trim() || undefined)) {
      setKey("");
      setRemoveKey(false);
      setSaved(true);
    }
  }

  return <section aria-labelledby="provider-title">
    <h2 id="provider-title">{props.t.provider}</h2>
    <form class={styles.form} onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <label class={styles.field}>
        {props.t.key}
        <input type="password" autocomplete="off" maxlength={512} placeholder={props.snapshot.key_configured ? props.t.replaceKey : undefined}
          value={key()} disabled={removeKey()} onInput={(event) => { setKey(event.currentTarget.value); setSaved(false); }} />
        <span class={styles.hint}>{props.snapshot.key_configured ? props.t.keySaved : props.t.keyEmpty}</span>
      </label>
      <Show when={props.snapshot.key_configured}>
        <label class={styles.removeKey}>
          <input type="checkbox" checked={removeKey()} onChange={(event) => { setRemoveKey(event.currentTarget.checked); setSaved(false); }} />
          {props.t.removeKey}
        </label>
      </Show>
      <details class={styles.advanced}>
        <summary>{props.t.advanced}</summary>
        <div class={styles.modelFields}>
          <For each={modelFields}>{(name) => <label class={styles.field}>
            {props.t[modelLabels[name]]}
            <input required maxlength={160} value={models()[name]} onInput={(event) => {
              const value = event.currentTarget.value;
              setModels((previous) => ({ ...previous, [name]: value }));
              setSaved(false);
            }} />
          </label>}</For>
          <label class={styles.field}>
            {props.t.sensitivity} <output>{Math.round(sensitivity() * 100)}%</output>
            <input type="range" min={0} max={1} step={0.01} value={sensitivity()} onInput={(event) => {
              setSensitivity(event.currentTarget.valueAsNumber);
              setSaved(false);
            }} />
          </label>
        </div>
      </details>
      <div class={styles.actions}>
        <Button variant="primary" type="submit" disabled={props.pending}>{props.t.save}</Button>
        <Show when={saved() && !props.error}><span role="status" class={styles.saved}>{props.t.saved}</span></Show>
      </div>
    </form>
  </section>;
}

