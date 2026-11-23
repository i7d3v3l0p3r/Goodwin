import { createMemo, createSignal, For, Show } from "solid-js";
import { Button } from "../../../components/Button";
import type { Capability, PipelineStage, Snapshot, Settings } from "../../../helpers/api";
import type { Language, Words } from "../../../helpers/words";
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
  const [invalid, setInvalid] = createSignal<Capability>();

  async function save() {
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
    setSaved(await props.save(settings));
  }

  return <section aria-labelledby="assistant-title">
    <h2 id="assistant-title">{props.t.assistant}</h2>
    <form class={styles.form} onSubmit={event => { event.preventDefault(); void save(); }}>
      <label class={styles.field}>
        {props.t.engine}
        <select value="pipeline">
          <option value="pipeline">{props.t.pipeline}</option>
          <option value="live" disabled>{props.t.liveSoon}</option>
        </select>
      </label>
      <For each={capabilities}>{capability =>
        <PipelineStageForm capability={capability} draft={draft()[capability]} plugins={props.snapshot.plugins}
          t={props.t} language={props.language} invalidParameters={invalid() === capability}
          change={changes => {
            setDraft(previous => ({ ...previous, [capability]: { ...previous[capability], ...changes } }));
            setSaved(false);
            if (invalid() === capability) setInvalid(undefined);
          }} />
      }</For>
      <div class="gw-row">
        <Button variant="primary" type="submit" disabled={props.pending}>{props.t.save}</Button>
        <Show when={saved() && !props.error}><span role="status" class={styles.saved}>{props.t.saved}</span></Show>
      </div>
    </form>
  </section>;
}
