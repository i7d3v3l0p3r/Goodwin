import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { ChoicePicker } from "../../../components/ui-kit/ChoicePicker";
import { Icon } from "../../../components/icons";
import { Disclosure } from "../../../components/ui-kit/Disclosure";
import { Field } from "../../../components/ui-kit/Field";
import { Switch } from "../../../components/ui-kit/Switch";
import { api, type Capability, type ModelCatalog, type ModelParameters, type Plugin } from "../../../helpers/api";
import type { Language, Words } from "../../../helpers/words";
import styles from "./AssistantForm.module.css";

export interface StageDraft {
  provider: string;
  model: string;
  voice: string;
  speed?: number;
  streaming?: boolean;
  parameters: ModelParameters;
}

const speechSpeed = { min: 0.25, max: 4, step: 0.05, default: 1 } as const;

const headings = { transcription: "transcriptionProvider", chat: "chatProvider", speech: "speechProvider" } as const;
const manualModel = "manual";

export function PipelineStageForm(props: {
  capability: Capability;
  draft: StageDraft;
  plugins: Plugin[];
  builtin?: boolean;
  t: Words;
  language: Language;
  focusOnOpen: boolean;
  change: (changes: Partial<StageDraft>) => void;
}) {
  const providers = createMemo(() => {
    const available = props.plugins.filter(plugin => plugin.status === "ready" && plugin.providers.includes(props.capability));
    const ids = new Set(props.plugins.flatMap(plugin => plugin.connections ?? []).map(connection => connection.providerId));
    const entries = available.flatMap(plugin => {
      if (plugin.connection_settings) return (plugin.connections ?? []).map(connection => ({
        id: connection.providerId, name: connection.name, model_catalog: plugin.model_catalog,
        speech_streaming: connection.speechStreaming && plugin.speech_streaming === true,
        revision: `${plugin.revision}:${connection.revision}`,
      }));
      if (ids.has(plugin.id)) return [];
      return [{ id: plugin.id, name: plugin.title[props.language], revision: plugin.revision ?? "",
        model_catalog: plugin.model_catalog, speech_streaming: plugin.speech_streaming === true }];
    });
    if (props.builtin) entries.unshift({
      id: "builtin", name: props.t.embedded.builtinProvider, revision: "",
      model_catalog: false, speech_streaming: true,
    });
    return entries;
  });
  const current = () => providers().find(plugin => plugin.id === props.draft.provider);
  const [catalog, setCatalog] = createSignal<ModelCatalog>();
  const [loading, setLoading] = createSignal(false);
  const [manual, setManual] = createSignal(false);
  const [reloadCount, setReloadCount] = createSignal(0);
  const [visited, setVisited] = createSignal(false);
  const revision = createMemo(() => visited() && current()?.model_catalog ? current()?.revision : undefined);
  let stage!: HTMLDetailsElement;

  createEffect(() => [props.draft.provider, revision(), reloadCount()] as const, ([provider, revision]) => {
    setCatalog(undefined);
    setLoading(Boolean(revision));
    if (!revision) return;
    let current = true;
    void api<ModelCatalog>(`plugin-models/${encodeURIComponent(provider)}/${props.capability}`)
      .then(response => { if (current) setCatalog(response); })
      .catch(() => {})
      .finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  });
  createEffect(() => props.focusOnOpen, focus => {
    if (!focus) return;
    stage.open = true;
    stage.querySelector("summary")?.focus();
  });

  const models = () => catalog()?.models ?? [];
  const defaultLabel = () => catalog()?.default ? `${props.t.defaultModel} · ${catalog()!.default}` : props.t.defaultModel;
  const manualActive = () => manual() || Boolean(props.draft.model && !models().some(model => model.id === props.draft.model));
  const modelValue = () => manualActive() ? manualModel : props.draft.model ? `model:${props.draft.model}` : "";
  const modelOptions = createMemo(() => [
    { value: "", label: props.t.defaultModel, detail: catalog()?.default || undefined },
    ...models().map(model => ({
      value: `model:${model.id}`, label: model.name || model.id,
      detail: model.name && model.name !== model.id ? model.id : undefined,
    })),
  ]);
  const voices = () => models().find(model => model.id === (props.draft.model || catalog()?.default))?.voices ?? [];
  const voiceList = () => `voices-${props.capability}`;
  const edit = (field: "model" | "voice", value: string) => props.change({ [field]: value });
  const parameter = <K extends keyof ModelParameters,>(key: K, value: ModelParameters[K]) =>
    props.change({ parameters: { ...props.draft.parameters, [key]: value } });
  const speed = () => props.draft.speed ?? speechSpeed.default;
  const speedLabel = () => `${new Intl.NumberFormat(props.language, { maximumFractionDigits: 2 }).format(speed())}×`;
  const streamingAvailable = () => current()?.speech_streaming === true;
  const providerLabel = () => current()?.name ?? `${props.draft.provider} (${props.t.plugins.status.error})`;
  const modelLabel = () => models().find(model => model.id === props.draft.model)?.name || props.draft.model || defaultLabel();

  return <Disclosure ref={node => { stage = node; }} class={styles.stage} name="assistant-stages"
    leading={<span class={styles.node} data-stage={props.capability} aria-hidden="true">
      <Icon name={({ transcription: "mic", chat: "chat", speech: "audioLines" } as const)[props.capability]} size={18} />
    </span>}
    title={props.t[headings[props.capability]]} description={`${providerLabel()} · ${modelLabel()}`}
    onToggle={event => { if (event.currentTarget.open) setVisited(true); }}>
    <div class={styles.stageFields}>
      <Field class={styles.primaryField} label={props.t.modelProvider}>
        {control => <ChoicePicker {...control} title={props.t.modelProvider} t={props.t} value={props.draft.provider}
          options={[...(!current() ? [{ value: props.draft.provider, label: providerLabel() }] : []),
            ...providers().map(plugin => ({ value: plugin.id, label: plugin.name }))]}
          onSelect={provider => {
            if (provider === props.draft.provider) return;
            setManual(false);
            props.change({ provider, model: "", voice: "", speed: undefined, parameters: {} });
          }} />}
      </Field>
      <Show when={current() && !current()?.model_catalog && current()?.id !== "builtin"}>
        <p class={styles.note}>{props.t.providerUpdate}</p>
      </Show>
      <Show when={current()?.model_catalog}>
        <Field class={styles.primaryField} label={props.t.modelName}>
          {control => <ChoicePicker {...control} title={props.t.modelName} t={props.t} searchHint={props.t.searchModels}
            options={modelOptions()} action={{ value: manualModel, label: props.t.manualModel, queryLabel: props.t.now.useId }} value={modelValue()}
            onSelect={(value, query) => {
              setManual(value === manualModel);
              if (query) edit("model", query);
              else if (value !== manualModel) edit("model", value ? value.slice(6) : "");
            }} />}
        </Field>
        <Show when={loading()}><p class={styles.note} role="status">{props.t.modelCatalogLoading}</p></Show>
        <Show when={visited() && !loading() && !catalog()?.available}>
          <div class="gw-row">
            <p class={styles.note}>{props.t.modelCatalogMissing}</p>
            <Button type="button" variant="quiet" onClick={() => setReloadCount(value => value + 1)}>{props.t.plugins.retry}</Button>
          </div>
        </Show>
        <Show when={manualActive()}>
          <Field label={props.t.modelId}>
            {control => <input {...control} value={props.draft.model} required maxlength={256}
              onInput={event => edit("model", event.currentTarget.value)} onInvalid={() => { stage.open = true; }} />}
          </Field>
        </Show>
        <Show when={props.capability === "speech"}>
          <Field label={props.t.voice}>
            {control => <>
              <input {...control} list={voiceList()} value={props.draft.voice} maxlength={256}
                onInput={event => edit("voice", event.currentTarget.value)} />
              <datalist id={voiceList()}><For each={voices()}>{voice => <option value={voice} />}</For></datalist>
            </>}
          </Field>
          <Field id="speech-speed" label={props.t.speechSpeed} hint={props.t.speechSpeedHint} valueLabel={speedLabel()}>
            {control => <input {...control} type="range" min={speechSpeed.min} max={speechSpeed.max}
              step={speechSpeed.step} value={speed()} aria-valuetext={speedLabel()}
              onInput={event => props.change({ speed: event.currentTarget.valueAsNumber })} />}
          </Field>
        </Show>
        <Disclosure title={props.t.modelParameters}>
          <Show when={props.capability !== "speech"}>
            <Field label={props.t.temperature}>
              {control => <input {...control} type="number" min="0" max="2" step="any"
                value={props.draft.parameters.temperature ?? ""}
                onInput={event => parameter("temperature", Number.isFinite(event.currentTarget.valueAsNumber)
                  ? event.currentTarget.valueAsNumber : undefined)} />}
            </Field>
          </Show>
          <Show when={props.capability === "chat"}>
            <Field label={props.t.topP}>
              {control => <input {...control} type="number" min="0" max="1" step="any"
                value={props.draft.parameters.top_p ?? ""}
                onInput={event => parameter("top_p", Number.isFinite(event.currentTarget.valueAsNumber)
                  ? event.currentTarget.valueAsNumber : undefined)} />}
            </Field>
          </Show>
          <Show when={props.capability === "transcription"}>
            <Field label={props.t.transcriptionPrompt}>
              {control => <textarea {...control} value={props.draft.parameters.prompt ?? ""} maxlength={1536} rows={2}
                onInput={event => parameter("prompt", event.currentTarget.value || undefined)} />}
            </Field>
          </Show>
          <Show when={props.capability === "speech"}>
            <Field label={props.t.speechInstructions}>
              {control => <textarea {...control} value={props.draft.parameters.instructions ?? ""} maxlength={1536} rows={2}
                onInput={event => parameter("instructions", event.currentTarget.value || undefined)} />}
            </Field>
          </Show>
        </Disclosure>
      </Show>
      <Show when={props.capability === "speech"}>
        <Switch label={props.t.speechStreaming}
          hint={streamingAvailable() ? props.t.speechStreamingHint : props.t.speechStreamingUnavailable}
          checked={streamingAvailable() && props.draft.streaming !== false}
          disabled={!streamingAvailable()} onChange={streaming => props.change({ streaming })} />
      </Show>
    </div>
  </Disclosure>;
}
