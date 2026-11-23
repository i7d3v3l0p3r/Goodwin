import { createSignal, Show } from "solid-js";
import { Button } from "../../../components/Button";
import { Icon } from "../../../components/icons";
import type { Words } from "../../../helpers/words";
import styles from "./SystemPromptForm.module.css";

export function SystemPromptForm(props: {
  t: Words;
  systemPrompt: string;
  pending: boolean;
  save: (prompt: string) => Promise<boolean>;
}) {
  const [draft, setDraft] = createSignal<string>();
  const [saving, setSaving] = createSignal(false);
  const [saved, setSaved] = createSignal(false);
  const prompt = () => draft() ?? props.systemPrompt;
  const canSave = () => !saving() && !props.pending && prompt() !== props.systemPrompt;

  async function save() {
    if (!canSave()) return;
    setSaved(false);
    setSaving(true);
    try {
      if (await props.save(prompt())) {
        setDraft(undefined);
        setSaved(true);
      }
    } finally {
      setSaving(false);
    }
  }

  return <details class={styles.prompt}>
    <summary class={styles.summary}>{props.t.systemPrompt}<Icon name="chevronDown" size={17} /></summary>
    <form class={styles.form} onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <label for="system-prompt" class={styles.hint}>{props.t.systemPromptHint}</label>
      <textarea id="system-prompt" rows={5} maxlength={4000}
        placeholder={props.t.systemPromptExample} value={prompt()} readonly={saving()}
        onInput={(event) => { setDraft(event.currentTarget.value); setSaved(false); }} />
      <div class={styles.actions}>
        <Button variant="primary" type="submit" disabled={!canSave()}>{props.t.savePrompt}</Button>
        <Show when={saved()}><span role="status" class={styles.saved}>{props.t.saved}</span></Show>
        <Show when={!saved() && prompt() !== props.systemPrompt}><span class={styles.hint}>{props.t.unsaved}</span></Show>
      </div>
    </form>
  </details>;
}
