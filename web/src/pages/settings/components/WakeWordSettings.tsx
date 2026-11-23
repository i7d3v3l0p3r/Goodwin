import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { Button } from "../../../components/Button";
import { api, ApiError, type WakeModel, type WakeModelCatalog } from "../../../helpers/api";
import { errorLabel, type Words } from "../../../helpers/words";
import styles from "./WakeWordSettings.module.css";

export function WakeWordSettings(props: {
  t: Words;
  listening: boolean;
  sensitivity: number;
  visible: boolean;
  currentModel: WakeModel | null;
  modelLoaded: boolean;
  pending: boolean;
  refresh: () => Promise<void>;
  toggleListening: () => void;
  changeSensitivity: (value: number) => void;
}) {
  const savedSensitivity = createMemo(() => props.sensitivity);
  const [sensitivity, setSensitivity] = createSignal(savedSensitivity);
  const [catalog, setCatalog] = createSignal<WakeModelCatalog>();
  const [id, setId] = createSignal("");
  const [loading, setLoading] = createSignal(false);
  const [installing, setInstalling] = createSignal(false);
  const [installed, setInstalled] = createSignal(false);
  const [catalogFailed, setCatalogFailed] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const selected = () => catalog()?.models.find(model => model.id === id());
  const modelLabel = (model: WakeModel) => `${model.wake_word} · ${model.tier === "micro" ? "Micro" : "Standard"}`;

  async function loadCatalog() {
    if (loading()) return;
    setLoading(true);
    setCatalogFailed(false);
    try {
      setCatalog(await api<WakeModelCatalog>("wake-models", "GET", undefined, 95_000));
    } catch {
      setCatalogFailed(true);
    } finally {
      setLoading(false);
    }
  }

  createEffect(() => props.visible, visible => {
    if (visible && !catalog()) void loadCatalog();
  });

  async function install(event: SubmitEvent) {
    event.preventDefault();
    const release = catalog();
    const model = selected();
    if (!release || !model || installing()) return;
    setInstalling(true);
    setInstalled(false);
    setError(undefined);
    try {
      await api<WakeModel>("wake-models", "PUT", { id: model.id, tag: release.tag }, 210_000);
      await props.refresh();
      setInstalled(true);
      setId("");
    } catch (failure) {
      setError(failure instanceof ApiError ? failure.code : "wake_model_failed");
      if (failure instanceof ApiError && failure.code === "wake_catalog_changed") void loadCatalog();
    } finally {
      setInstalling(false);
    }
  }

  return <section aria-labelledby="wake-word-title">
    <h3 id="wake-word-title">{props.t.wakeWord}</h3>
    <div class={styles.row}>
      <span id="listening-label">{props.t.listening(props.currentModel?.wake_word ?? props.t.defaultWakeWord)}</span>
      <button type="button" role="switch" class={styles.listeningSwitch} aria-labelledby="listening-label"
        aria-checked={props.listening ? "true" : "false"} disabled={props.pending}
        onClick={props.toggleListening}>
        <span />
      </button>
    </div>
    <label class={styles.sensitivity} for="sensitivity">
      <span>{props.t.sensitivity} <output aria-hidden="true">{Math.round(sensitivity() * 100)}%</output></span>
      <input id="sensitivity" type="range" min={0} max={1} step={0.01} value={sensitivity()} disabled={props.pending}
        aria-valuetext={`${Math.round(sensitivity() * 100)}%`}
        onInput={event => setSensitivity(event.currentTarget.valueAsNumber)}
        onChange={event => props.changeSensitivity(event.currentTarget.valueAsNumber)} />
    </label>
    <form class={styles.modelForm} onSubmit={event => void install(event)}>
      <label for="wake-model">{props.t.wakeModel}</label>
      <p class={styles.modelStatus}>{props.t.currentWakeModel} · {props.currentModel
        ? modelLabel(props.currentModel)
        : props.modelLoaded ? props.t.localWakeModel : props.t.noWakeModel}</p>
      <select id="wake-model" value={id()} disabled={loading() || installing() || props.pending || !catalog()?.models.length}
        onChange={event => { setId(event.currentTarget.value); setInstalled(false); setError(undefined); }}>
        <option value="" selected={!id()}>{props.t.chooseWakeModel}</option>
        <For each={catalog()?.models}>{model =>
          <option value={model.id} selected={id() === model.id}>{modelLabel(model)}</option>
        }</For>
      </select>
      <div class={styles.modelActions}>
        <Button variant="primary" type="submit" disabled={installing() || loading() || props.pending || !selected()
          || (props.modelLoaded && selected()?.sha256 === props.currentModel?.sha256)}>
          {installing() ? props.t.installingWakeModel : props.t.installWakeModel}
        </Button>
        <Show when={catalogFailed()}>
          <Button type="button" disabled={props.pending} onClick={() => void loadCatalog()}>{props.t.retryWakeCatalog}</Button>
        </Show>
        <Show when={loading()}><span role="status">{props.t.loadingWakeCatalog}</span></Show>
        <Show when={installed()}><span role="status">{props.t.wakeModelActivated}</span></Show>
      </div>
      <Show when={error() ?? (catalogFailed() ? "wake_catalog_unavailable" : undefined)}>
        {code => <p class={styles.modelError} role="alert">{errorLabel(props.t, code())}</p>}
      </Show>
      <Show when={catalog()?.models.length === 0}><p role="status">{props.t.emptyWakeCatalog}</p></Show>
    </form>
  </section>;
}
