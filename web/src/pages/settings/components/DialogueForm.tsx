import { createSignal, Show } from "solid-js";
import { Field } from "../../../components/ui-kit/Field";
import { SaveActions } from "../../../components/ui-kit/SaveActions";
import type { DialogueSettings, Settings } from "../../../helpers/api";
import { protectDraft } from "../../../helpers/drafts";
import type { Words } from "../../../helpers/words";
import { isSettingsPath } from "../sections";

type Mode = "wake_word" | "push_to_talk" | "follow_up";

export function DialogueForm(props: {
  t: Words;
  settings: Settings;
  dialogue: DialogueSettings;
  activation: string[];
  pending: boolean;
  save: (settings: Settings) => Promise<boolean>;
}) {
  const [modeDraft, setMode] = createSignal<Mode>();
  const [secondsDraft, setSeconds] = createSignal<string>();
  const [saved, setSaved] = createSignal(false);
  const hasButton = () => props.activation.includes("push_to_talk");
  const hasWake = () => props.activation.includes("wake_word");
  const storedMode = (): Mode => props.dialogue.mode === "follow_up" ? "follow_up"
    : hasButton() && (!hasWake() || !props.settings.listening) ? "push_to_talk" : "wake_word";
  const mode = () => modeDraft() ?? storedMode();
  const seconds = () => secondsDraft() ?? String(props.dialogue.timeout_seconds);
  const timeout = () => mode() === "follow_up" ? Number(seconds()) : props.dialogue.timeout_seconds;
  const valid = () => Number.isInteger(timeout()) && timeout() >= 1 && timeout() <= 60;
  const dirty = () => mode() !== storedMode() || timeout() !== props.dialogue.timeout_seconds;
  protectDraft(dirty, isSettingsPath);

  async function save() {
    if (props.pending || !dirty() || !valid()) return;
    const selected = mode();
    if (await props.save({
      ...props.settings,
      listening: selected === "follow_up" ? props.settings.listening : selected === "wake_word",
      dialogue: { mode: selected === "follow_up" ? "follow_up" : "single_turn", timeout_seconds: timeout() },
    })) {
      setMode(undefined);
      setSeconds(undefined);
      setSaved(true);
    }
  }

  return <section aria-labelledby="voice-dialogue-title">
    <h2 id="voice-dialogue-title">{props.t.voiceDialogue}</h2>
    <form class="gw-stack" onSubmit={event => { event.preventDefault(); void save(); }}>
      <Field label={props.t.dialogueMode}>{control =>
        <select {...control} value={mode()} disabled={props.pending}
          onChange={event => { setMode(event.currentTarget.value as Mode); setSaved(false); }}>
          <Show when={hasWake()}><option value="wake_word" selected={mode() === "wake_word"}>{props.t.singleWake}</option></Show>
          <Show when={hasButton()}><option value="push_to_talk" selected={mode() === "push_to_talk"}>{props.t.singleButton}</option></Show>
          <option value="follow_up" selected={mode() === "follow_up"}>{props.t.continuousDialogue}</option>
        </select>}
      </Field>
      <Show when={mode() === "follow_up"}>
        <Field label={props.t.replyTimeout} error={valid() ? undefined : props.t.timeoutRange}>{control =>
          <input {...control} type="number" inputmode="numeric" min={1} max={60} step={1} required
            value={seconds()} disabled={props.pending}
            onInput={event => { setSeconds(event.currentTarget.value); setSaved(false); }} />}
        </Field>
      </Show>
      <SaveActions labels={props.t} dirty={dirty()} saved={saved() && !dirty()} saving={props.pending && dirty()}
        disabled={props.pending || !valid() || !dirty()} />
    </form>
  </section>;
}
