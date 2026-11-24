import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { Disclosure } from "../../../components/ui-kit/Disclosure";
import { Field } from "../../../components/ui-kit/Field";
import { Switch } from "../../../components/ui-kit/Switch";
import { api, type Capability, type ModelCatalog, type Plugin } from "../../../helpers/api";
import type { Language, Words } from "../../../helpers/words";
import styles from "./AssistantForm.module.css";

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
  focusOnOpen: boolean;
  change: (changes: Partial<StageDraft>) => void;
}) {
  const providers = createMemo(() => {
    const available = props.plugins.filter(plugin => plugin.status === "ready" && plugin.providers.includes(props.capability));
    const ids = new Set(props.plugins.flatMap(plugin => plugin.connections ?? []).map(connection => connection.providerId));
    return available.flatMap(plugin => {
      if (plugin.connection_settings) return (plugin.connections ?? []).map(connection => ({
        id: connection.providerId, name: connection.name, model_catalog: plugin.model_catalog,
        speech_streaming: connection.speechStreaming && plugin.speech_streaming === true,
        revision: `${plugin.revision}:${connection.revision}`,
      }));
      if (ids.has(plugin.id)) return [];
      return [{ id: plugin.id, name: plugin.title[props.language], revision: plugin.revision ?? "",
        model_catalog: plugin.model_catalog, speech_streaming: plugin.speech_streaming === true }];
    });
  });
  const current = () => providers().find(plugin => plugin.id === props.draft.provider);
  const [catalog, setCatalog] = createSignal<ModelCatalog>();
  const [loading, setLoading] = createSignal(false);
  const [manual, setManual] = createSignal(false);
  const [reloadCount, setReloadCount] = createSignal(0);
  const [visited, setVisited] = createSignal(false);
  const revision = createMemo(() => visited() && current()?.model_catalog ? current()?.revision : undefined);
  let stage!: HTMLDetailsElement;
  let parameters: HTMLDetailsElement | undefined;

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
  createEffect(() => props.invalidParameters, invalid => {
    if (!invalid) return;
    stage.open = true;
    if (parameters?.isConnected) {
      parameters.open = true;
      parameters.querySelector("textarea")?.focus();
    }
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
  const voices = () => models().find(model => model.id === (props.draft.model || catalog()?.default))?.voices ?? [];
  const voiceList = () => `voices-${props.capability}`;
  const edit = (field: "model" | "voice" | "parameters", value: string) => props.change({ [field]: value });
  const speed = () => props.draft.speed ?? speechSpeed.default;
  const speedLabel = () => `${new Intl.NumberFormat(props.language, { maximumFractionDigits: 2 }).format(speed())}×`;
  const streamingAvailable = () => current()?.speech_streaming === true;
  const providerLabel = () => current()?.name ?? `${props.draft.provider} (${props.t.plugins.status.error})`;
  const modelLabel = () => models().find(model => model.id === props.draft.model)?.name || props.draft.model || defaultLabel();

  return <Disclosure ref={node => { stage = node; }} class={styles.stage} name="assistant-stages"
    title={props.t[headings[props.capability]]} description={`${providerLabel()} · ${modelLabel()}`}
    onToggle={event => { if (event.currentTarget.open) setVisited(true); }}>
    <div class={styles.stageFields}>
      <Field class={styles.primaryField} label={props.t.modelProvider}>
        {control => <select {...control} value={props.draft.provider} onChange={event => {
          setManual(false);
          props.change({ provider: event.currentTarget.value, model: "", voice: "", speed: undefined, parameters: "" });
        }}>
          <Show when={!current()}>
            <option value={props.draft.provider} selected>{providerLabel()}</option>
          </Show>
          <For each={providers()}>{plugin =>
            <option value={plugin.id} selected={plugin.id === props.draft.provider}>{plugin.name}</option>
          }</For>
        </select>}
      </Field>
      <Show when={current() && !current()?.model_catalog}><p class={styles.note}>{props.t.providerUpdate}</p></Show>
      <Show when={current()?.model_catalog}>
        <Field class={styles.primaryField} label={props.t.modelName}>
          {control => <select {...control} value={modelValue()} onChange={event => {
            const value = event.currentTarget.value;
            setManual(value === manualModel);
            if (value !== manualModel) edit("model", value ? value.slice(6) : "");
          }}>
            <option value="" selected={!modelValue()}>{defaultLabel()}</option>
            <For each={models()}>{model => <option value={`model:${model.id}`} selected={modelValue() === `model:${model.id}`}>
              {model.name && model.name !== model.id ? `${model.name} (${model.id})` : model.id}
            </option>}</For>
            <option value={manualModel} selected={manualActive()}>{props.t.manualModel}</option>
          </select>}
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
        <Disclosure ref={node => { parameters = node; }} title={props.t.modelParameters}>
          <Field label={`${props.t.modelParameters} · ${props.t[headings[props.capability]]}`} hideLabel
            error={props.invalidParameters ? props.t.badModelParameters : undefined}>
            {control => <textarea {...control} class={[control.class, styles.parametersInput]}
              placeholder={"{ }"} value={props.draft.parameters} maxlength={2048} rows={3} spellcheck={false}
              onInput={event => edit("parameters", event.currentTarget.value)} />}
          </Field>
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
