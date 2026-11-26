import { createMemo, createSignal, For } from "solid-js";
import codes from "../../../../../shared/language-codes.txt?raw";
import { Disclosure } from "../../../components/ui-kit/Disclosure";
import { Field } from "../../../components/ui-kit/Field";
import { SaveActions } from "../../../components/ui-kit/SaveActions";
import type { AssistantLanguages, Settings } from "../../../helpers/api";
import { protectDraft } from "../../../helpers/drafts";
import type { Language, Words } from "../../../helpers/words";
import { isSettingsPath } from "../sections";

export function AssistantLanguagesForm(props: {
  t: Words; language: Language; settings: Settings; pending: boolean;
  save: (settings: Settings) => Promise<boolean>;
}) {
  const options = createMemo(() => {
    const names = new Intl.DisplayNames([props.language], { type: "language" });
    const sort = new Intl.Collator(props.language);
    return codes.trim().split(/\s+/).map(code => ({ code, name: names.of(code) ?? code }))
      .sort((a, b) => sort.compare(a.name, b.name));
  });
  const [draft, setDraft] = createSignal<Partial<AssistantLanguages>>({});
  const [saved, setSaved] = createSignal(false);
  const value = (field: keyof AssistantLanguages) => draft()[field] ?? props.settings.languages[field];
  const dirty = () => value("transcription") !== props.settings.languages.transcription
    || value("reply") !== props.settings.languages.reply;
  protectDraft(dirty, isSettingsPath);

  async function save() {
    if (props.pending || !dirty()) return;
    if (await props.save({ ...props.settings, languages: { transcription: value("transcription"), reply: value("reply") } })) {
      setDraft({});
      setSaved(true);
    }
  }

  return <Disclosure title={props.t.assistantLanguages}>
    <form class="gw-stack" onSubmit={event => { event.preventDefault(); void save(); }}>
      <For each={["transcription", "reply"] as const}>{field =>
        <Field label={field === "reply" ? props.t.replyLanguage : props.t.transcriptionLanguage}
          hint={field === "reply" ? props.t.replyLanguageHint : undefined}>
          {control => <select {...control} value={value(field)} disabled={props.pending}
            onChange={event => {
              const code = event.currentTarget.value;
              setDraft(previous => ({ ...previous, [field]: code }));
              setSaved(false);
            }}>
            <For each={options()}>{option =>
              <option value={option.code} selected={value(field) === option.code}>{option.name}</option>
            }</For>
          </select>}
        </Field>
      }</For>
      <SaveActions labels={props.t} dirty={dirty()} saving={props.pending && dirty()} saved={saved() && !dirty()}
        disabled={props.pending || !dirty()} />
    </form>
  </Disclosure>;
}
