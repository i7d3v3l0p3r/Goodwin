import { createMemo, createSignal, For } from "solid-js";
import { SaveActions } from "../../../components/ui-kit/SaveActions";
import type { Capability, PipelineStage, Snapshot, Settings } from "../../../helpers/api";
import { protectDraft } from "../../../helpers/drafts";
import type { Language, Words } from "../../../helpers/words";
import { isSettingsPath } from "../sections";
import { PipelineStageForm, type StageDraft } from "./PipelineStageForm";
import styles from "./AssistantForm.module.css";

const capabilities = ["transcription", "chat", "speech"] as const;

function draftStage(stage: PipelineStage): StageDraft {
  const { voice = "", speed, ...parameters } = stage.selection.parameters ?? {};
  return {
    provider: stage.provider, model: stage.selection.model ?? "", voice, speed,
    parameters,
  };
}

function draftPipeline(settings: Settings): Record<Capability, StageDraft> {
  const { pipeline } = settings.assistant;
  return {
    transcription: draftStage(pipeline.transcription),
    chat: draftStage(pipeline.chat),
    speech: { ...draftStage(pipeline.speech), streaming: pipeline.speech_streaming ?? true },
  };
}

export function AssistantForm(props: {
  t: Words;
  language: Language;
  snapshot: Snapshot;
  pending: boolean;
  error: string;
  focusStage?: Capability;
  builtin?: boolean;
  save: (settings: Settings) => Promise<boolean>;
}) {
  const savedSettings = createMemo(() => draftPipeline(props.snapshot.settings), {
    equals: (previous, next) => JSON.stringify(previous) === JSON.stringify(next),
  });
  const [draft, setDraft] = createSignal(savedSettings);
  const [saved, setSaved] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const dirty = createMemo(() => JSON.stringify(draft()) !== JSON.stringify(savedSettings()));
  protectDraft(dirty, isSettingsPath);

  async function save() {
    if (props.pending || saving() || !dirty()) return;
    setSaved(false);
    const pipeline = { ...props.snapshot.settings.assistant.pipeline };
    for (const capability of capabilities) {
      const stage = draft()[capability];
      const parameters = { ...stage.parameters };
      if (capability === "speech") {
        if (stage.voice.trim()) parameters.voice = stage.voice.trim();
        if (stage.speed !== undefined) parameters.speed = stage.speed;
      }
      pipeline[capability] = {
        provider: stage.provider,
        selection: { model: stage.model.trim() || undefined, parameters },
      };
    }
    pipeline.speech_streaming = draft().speech.streaming !== false;
    const settings = { ...props.snapshot.settings, assistant: { engine: "pipeline" as const, pipeline } };
    setSaving(true);
    try {
      setSaved(await props.save(settings));
    } finally {
      setSaving(false);
    }
  }

  return <section aria-labelledby="assistant-title">
    <h2 id="assistant-title">{props.t.modelsAndVoice}</h2>
    <form class={styles.form} onSubmit={event => { event.preventDefault(); void save(); }}>
      <p class={styles.note}>{props.t.pipeline}</p>
      <fieldset class={styles.stages} disabled={props.pending} aria-labelledby="assistant-title">
        <For each={capabilities}>{capability =>
          <PipelineStageForm capability={capability} draft={draft()[capability]} plugins={props.snapshot.plugins}
            builtin={props.builtin}
            t={props.t} language={props.language}
            focusOnOpen={props.focusStage === capability}
            change={changes => {
              setDraft(previous => ({ ...previous, [capability]: { ...previous[capability], ...changes } }));
              setSaved(false);
            }} />
        }</For>
      </fieldset>
      <SaveActions labels={props.t} dirty={dirty()} saving={saving()} saved={saved() && !props.error}
        disabled={props.pending || !dirty()} />
    </form>
  </section>;
}
