import { createSignal } from "solid-js";
import { Disclosure } from "../../../components/ui-kit/Disclosure";
import { Field } from "../../../components/ui-kit/Field";
import { SaveActions } from "../../../components/ui-kit/SaveActions";
import { protectDraft } from "../../../helpers/drafts";
import type { Words } from "../../../helpers/words";
import { isSettingsPath } from "../sections";

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
  const dirty = () => prompt() !== props.systemPrompt;
  const canSave = () => !saving() && !props.pending && dirty();
  protectDraft(dirty, isSettingsPath);

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

  return <Disclosure title={props.t.systemPrompt}>
    <form class="gw-stack" onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <Field id="system-prompt" label={props.t.systemPrompt} hideLabel hint={props.t.systemPromptHint}>
        {control => <textarea {...control} rows={5} maxlength={4000}
          placeholder={props.t.systemPromptExample} value={prompt()} readonly={saving()}
          onInput={event => { setDraft(event.currentTarget.value); setSaved(false); }} />}
      </Field>
      <SaveActions labels={props.t} label={props.t.savePrompt} dirty={dirty()} saving={saving()} saved={saved()}
        disabled={!canSave()} />
    </form>
  </Disclosure>;
}
