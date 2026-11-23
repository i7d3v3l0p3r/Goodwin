import { createMemo, createSignal, For } from "solid-js";
import { SaveActions } from "../../../components/ui-kit/SaveActions";
import type { Capability, PipelineStage, Snapshot, Settings } from "../../../helpers/api";
import { protectDraft } from "../../../helpers/drafts";
import type { Language, Words } from "../../../helpers/words";
import { isSettingsPath } from "../sections";
import { PipelineStageForm, speechSpeed, type StageDraft } from "./PipelineStageForm";
import styles from "./AssistantForm.module.css";

const capabilities = ["transcription", "chat", "speech"] as const;

function draftStage(stage: PipelineStage, capability: Capability): StageDraft {
  const parameters = { ...stage.selection.parameters };
  const voice = capability === "speech" && typeof parameters.voice === "string" ? parameters.voice : "";
  if (capability === "speech" && typeof parameters.voice === "string") delete parameters.voice;
  const speed = capability === "speech" && typeof parameters.speed === "number"
    && parameters.speed >= speechSpeed.min && parameters.speed <= speechSpeed.max ? parameters.speed : undefined;
  if (speed !== undefined) delete parameters.speed;
  return {
    provider: stage.provider, model: stage.selection.model ?? "", voice, speed,
    parameters: Object.keys(parameters).length ? JSON.stringify(parameters, null, 2) : "",
  };
}

function draftPipeline(settings: Settings): Record<Capability, StageDraft> {
  const { pipeline } = settings.assistant;
  return {
    transcription: draftStage(pipeline.transcription, "transcription"),
    chat: draftStage(pipeline.chat, "chat"),
    speech: { ...draftStage(pipeline.speech, "speech"), streaming: pipeline.speech_streaming ?? true },
  };
}

export function AssistantForm(props: {
  t: Words;
  language: Language;
  snapshot: Snapshot;
  pending: boolean;
  error: string;
  save: (settings: Settings) => Promise<boolean>;
}) {
  const savedSettings = createMemo(() => draftPipeline(props.snapshot.settings), {
    equals: (previous, next) => JSON.stringify(previous) === JSON.stringify(next),
  });
  const [draft, setDraft] = createSignal(savedSettings);
  const [saved, setSaved] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [invalid, setInvalid] = createSignal<Capability>();
  const dirty = createMemo(() => JSON.stringify(draft()) !== JSON.stringify(savedSettings()));
  protectDraft(dirty, isSettingsPath);

  async function save() {
    if (props.pending || saving() || !dirty()) return;
    setSaved(false);
    setInvalid(undefined);
    const pipeline = { ...props.snapshot.settings.assistant.pipeline };
    for (const capability of capabilities) {
      const stage = draft()[capability];
      let parameters: Record<string, unknown>;
      try {
        const value: unknown = stage.parameters.trim() ? JSON.parse(stage.parameters) : {};
        if (!value || Array.isArray(value) || typeof value !== "object") throw new Error();
        parameters = value as Record<string, unknown>;
      } catch {
        setInvalid(capability);
        return;
      }
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
    <h3 id="assistant-title">{props.t.modelsAndVoice}</h3>
    <form class={styles.form} onSubmit={event => { event.preventDefault(); void save(); }}>
      <p class={styles.note}>{props.t.pipeline}</p>
      <fieldset class={styles.stages} disabled={props.pending} aria-labelledby="assistant-title">
        <For each={capabilities}>{capability =>
          <PipelineStageForm capability={capability} draft={draft()[capability]} plugins={props.snapshot.plugins}
            t={props.t} language={props.language} invalidParameters={invalid() === capability}
            change={changes => {
              setDraft(previous => ({ ...previous, [capability]: { ...previous[capability], ...changes } }));
              setSaved(false);
              if (invalid() === capability) setInvalid(undefined);
            }} />
        }</For>
      </fieldset>
      <SaveActions labels={props.t} dirty={dirty()} saving={saving()} saved={saved() && !props.error}
        disabled={props.pending || !dirty()} />
    </form>
  </section>;
}
