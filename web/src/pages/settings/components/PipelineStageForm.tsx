import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { api, type Capability, type ModelCatalog, type Plugin } from "../../../helpers/api";
import type { Language, Words } from "../../../helpers/words";
import styles from "./AssistantForm.module.css";
import switchStyles from "./Preferences.module.css";

export interface StageDraft {
  provider: string;
  model: string;
  voice: string;
  speed?: number;
  streaming?: boolean;
  parameters: string;
}

export const speechSpeed = { min: 0.5, max: 2, step: 0.05, default: 1 } as const;

const headings = { transcription: "transcriptionProvider", chat: "chatProvider", speech: "speechProvider" } as const;
const manualModel = "manual";

export function PipelineStageForm(props: {
  capability: Capability;
  draft: StageDraft;
  plugins: Plugin[];
  t: Words;
  language: Language;
  invalidParameters: boolean;
  change: (changes: Partial<StageDraft>) => void;
}) {
  const providers = createMemo(() => props.plugins.filter(plugin => plugin.providers.includes(props.capability)));
  const current = () => providers().find(plugin => plugin.id === props.draft.provider);
  const revision = createMemo(() => current()?.model_catalog ? current()?.revision : undefined);
  const [catalog, setCatalog] = createSignal<ModelCatalog>();
  const [loading, setLoading] = createSignal(false);
  const [manual, setManual] = createSignal(false);
  const [reloadCount, setReloadCount] = createSignal(0);
  let details: HTMLDetailsElement | undefined;

  createEffect(() => [revision(), reloadCount()] as const, ([revision]) => {
    setCatalog(undefined);
    setLoading(Boolean(revision));
    if (!revision) return;
    let current = true;
    void api<ModelCatalog>(`plugin-models/${encodeURIComponent(props.draft.provider)}/${props.capability}`)
      .then(response => { if (current) setCatalog(response); })
      .catch(() => {})
      .finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  });
  createEffect(() => props.invalidParameters, invalid => {
    if (invalid && details) details.open = true;
  });

  const models = () => catalog()?.models ?? [];
  const defaultLabel = () => catalog()?.default ? `${props.t.defaultModel} · ${catalog()!.default}` : props.t.defaultModel;
  const manualActive = () => manual() || Boolean(props.draft.model && !models().some(model => model.id === props.draft.model));
  const modelValue = () => manualActive() ? manualModel : props.draft.model ? `model:${props.draft.model}` : "";
  const voices = () => models().find(model => model.id === (props.draft.model || catalog()?.default))?.voices ?? [];
  const voiceList = () => `voices-${props.capability}`;
  const edit = (field: "model" | "voice" | "parameters", value: string) => props.change({ [field]: value });
  const speed = () => props.draft.speed ?? speechSpeed.default;
  const speedLabel = () => `${new Intl.NumberFormat(props.language, { maximumFractionDigits: 2 }).format(speed())}×`;
  const streamingAvailable = () => current()?.speech_streaming === true;

  return <fieldset class={styles.stage}>
    <legend>{props.t[headings[props.capability]]}</legend>
    <label class={styles.field}>
      {props.t.modelProvider}
      <select value={props.draft.provider} onChange={event => {
        setManual(false);
        props.change({ provider: event.currentTarget.value, model: "", voice: "", speed: undefined, parameters: "" });
      }}>
        <Show when={!current()}>
          <option value={props.draft.provider} selected>{props.draft.provider} ({props.t.plugins.status.error})</option>
        </Show>
        <For each={providers()}>{plugin =>
          <option value={plugin.id} selected={plugin.id === props.draft.provider}>{plugin.title[props.language]}</option>
        }</For>
      </select>
    </label>
    <Show when={current() && !current()?.model_catalog}><p class={styles.note}>{props.t.providerUpdate}</p></Show>
    <Show when={current()?.model_catalog}>
      <label class={styles.field}>
        {props.t.modelName}
        <select value={modelValue()} onChange={event => {
          const value = event.currentTarget.value;
          setManual(value === manualModel);
          if (value !== manualModel) edit("model", value ? value.slice(6) : "");
        }}>
          <option value="" selected={!modelValue()}>{defaultLabel()}</option>
          <For each={models()}>{model => <option value={`model:${model.id}`} selected={modelValue() === `model:${model.id}`}>
            {model.name && model.name !== model.id ? `${model.name} (${model.id})` : model.id}
          </option>}</For>
          <option value={manualModel} selected={manualActive()}>{props.t.manualModel}</option>
        </select>
      </label>
      <Show when={loading()}><p class={styles.note} role="status">{props.t.modelCatalogLoading}</p></Show>
      <Show when={!loading() && !catalog()?.available}>
        <div class="gw-row">
          <p class={styles.note}>{props.t.modelCatalogMissing}</p>
          <button type="button" class={styles.retry} onClick={() => setReloadCount(value => value + 1)}>{props.t.plugins.retry}</button>
        </div>
      </Show>
      <Show when={manualActive()}>
        <label class={styles.field}>
          {props.t.modelId}
          <input value={props.draft.model} required maxlength={256} onInput={event => edit("model", event.currentTarget.value)} />
        </label>
      </Show>
      <Show when={props.capability === "speech"}>
        <label class={styles.field}>
          {props.t.voice}
          <input list={voiceList()} value={props.draft.voice} maxlength={256} onInput={event => edit("voice", event.currentTarget.value)} />
          <datalist id={voiceList()}><For each={voices()}>{voice => <option value={voice} />}</For></datalist>
        </label>
        <div class={styles.field}>
          <label for="speech-speed" class={styles.rangeLabel}>
            {props.t.speechSpeed}
            <output aria-hidden="true">{speedLabel()}</output>
          </label>
          <input id="speech-speed" class={styles.range} type="range" min={speechSpeed.min} max={speechSpeed.max}
            step={speechSpeed.step} value={speed()} aria-valuetext={speedLabel()} aria-describedby="speech-speed-hint"
            onInput={event => props.change({ speed: event.currentTarget.valueAsNumber })} />
          <p id="speech-speed-hint" class={styles.note}>{props.t.speechSpeedHint}</p>
        </div>
      </Show>
      <details ref={node => { details = node; }} class={styles.parameters}>
        <summary>{props.t.modelParameters}</summary>
        <textarea aria-label={`${props.t.modelParameters} · ${props.t[headings[props.capability]]}`}
          placeholder={"{ }"} value={props.draft.parameters} maxlength={2048} rows={3} spellcheck={false}
          aria-invalid={props.invalidParameters ? "true" : undefined} onInput={event => edit("parameters", event.currentTarget.value)} />
        <Show when={props.invalidParameters}><p role="alert" class={styles.error}>{props.t.badModelParameters}</p></Show>
      </details>
    </Show>
    <Show when={props.capability === "speech"}>
      <div class={styles.field}>
        <div class={styles.rangeLabel}>
          <span id="speech-streaming-label">{props.t.speechStreaming}</span>
          <button type="button" role="switch" class={switchStyles.listeningSwitch} aria-labelledby="speech-streaming-label"
            aria-describedby="speech-streaming-hint" aria-checked={streamingAvailable() && props.draft.streaming !== false ? "true" : "false"}
            disabled={!streamingAvailable()} onClick={() => props.change({ streaming: props.draft.streaming === false })}>
            <span />
          </button>
        </div>
        <p id="speech-streaming-hint" class={styles.note}>
          {streamingAvailable() ? props.t.speechStreamingHint : props.t.speechStreamingUnavailable}
        </p>
      </div>
    </Show>
  </fieldset>;
}
