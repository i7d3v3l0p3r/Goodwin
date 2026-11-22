import { createSignal, Show } from "solid-js";
import { Button } from "../../../components/Button";
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

  return <section aria-labelledby="system-prompt-title">
    <h2 id="system-prompt-title"><label for="system-prompt">{props.t.systemPrompt}</label></h2>
    <form class={styles.form} onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <p id="system-prompt-hint" class={styles.hint}>{props.t.systemPromptHint}</p>
      <textarea id="system-prompt" aria-describedby="system-prompt-hint" rows={5} maxlength={4000}
        placeholder={props.t.systemPromptExample} value={prompt()} readonly={saving()}
        onInput={(event) => { setDraft(event.currentTarget.value); setSaved(false); }} />
      <div class={styles.actions}>
        <Button variant="primary" type="submit" disabled={!canSave()}>{props.t.save}</Button>
        <Show when={saved()}><span role="status" class={styles.saved}>{props.t.saved}</span></Show>
      </div>
    </form>
  </section>;
}
