import { createEffect, createSignal, For, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { Switch } from "../../../components/ui-kit/Switch";
import { api, apiOperation, ApiError, waitForOperation, type WakeModel, type WakeModelCatalog } from "../../../helpers/api";
import { protectDraft } from "../../../helpers/drafts";
import { errorLabel, type Words } from "../../../helpers/words";
import { isSettingsPath } from "../sections";
import styles from "./WakeWordSettings.module.css";

const operationKey = "goodwin-wake-model-operation";

export function WakeWordSettings(props: {
  t: Words;
  listening: boolean;
  sensitivity: number;
  visible: boolean;
  currentModel: WakeModel | null;
  modelLoaded: boolean;
  hasCatalog: boolean;
  onModelInstalled: () => void;
  pending: boolean;
  toggleListening: () => void;
  changeSensitivity: (value: number) => Promise<boolean>;
}) {
  const [preview, setPreview] = createSignal<number>();
  const [failedSensitivity, setFailedSensitivity] = createSignal<number>();
  const [catalog, setCatalog] = createSignal<WakeModelCatalog>();
  const [id, setId] = createSignal("");
  const [loading, setLoading] = createSignal(false);
  const [installing, setInstalling] = createSignal(false);
  const [operation, setOperation] = createSignal(sessionStorage.getItem(operationKey));
  const [installed, setInstalled] = createSignal(false);
  const [catalogFailed, setCatalogFailed] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const selected = () => catalog()?.models.find(model => model.id === id());
  const modelLabel = (model: WakeModel) => {
    const tier = model.runtime === "microwakeword-tflite" ? "microWakeWord"
      : model.runtime === "livekit-v2-micro-tflite" ? "Micro INT8" : model.tier === "micro" ? "Micro" : "Standard";
    return `${model.wake_word} · ${tier}`;
  };
  const sensitivity = () => preview() ?? props.sensitivity;
  const canInstall = () => Boolean(selected() && (!props.modelLoaded || selected()?.id !== props.currentModel?.id
    || selected()?.sha256 !== props.currentModel?.sha256));
  protectDraft(() => !installing() && canInstall(), isSettingsPath);

  async function saveSensitivity(value: number) {
    setFailedSensitivity(undefined);
    if (value === props.sensitivity) {
      setPreview(undefined);
      return;
    }
    setPreview(value);
    const saved = await props.changeSensitivity(value);
    setPreview(undefined);
    if (!saved) setFailedSensitivity(value);
  }

  async function loadCatalog() {
    if (loading()) return;
    setLoading(true);
    setCatalogFailed(false);
    try {
      setCatalog(await apiOperation<WakeModelCatalog>("wake-models", "GET", undefined, 95_000));
    } catch {
      setCatalogFailed(true);
    } finally {
      setLoading(false);
    }
  }

  createEffect(() => props.visible && props.hasCatalog, visible => {
    if (visible && !catalog()) void loadCatalog();
    if (visible && operation()) void install();
  });

  function rememberOperation(ticket: string | null) {
    setOperation(ticket);
    if (ticket) sessionStorage.setItem(operationKey, ticket);
    else sessionStorage.removeItem(operationKey);
  }

  async function install(event?: SubmitEvent) {
    event?.preventDefault();
    const release = catalog();
    const model = selected();
    let ticket = operation();
    if (installing() || (!ticket && (!release || !model))) return;
    setInstalling(true);
    setInstalled(false);
    setError(undefined);
    try {
      const deadline = performance.now() + 210_000;
      if (!ticket && release && model) {
        const started = await api<WakeModel | { operation: string }>("wake-models", "PUT", { id: model.id, tag: release.tag }, 210_000);
        if (started && "operation" in started) {
          if (typeof started.operation !== "string") throw new ApiError("internal");
          ticket = started.operation;
          rememberOperation(ticket);
        }
      }
      if (ticket) await waitForOperation<WakeModel>("wake-models/result", ticket, deadline, true);
      rememberOperation(null);
      props.onModelInstalled();
      setInstalled(true);
      setId("");
    } catch (failure) {
      const code = failure instanceof ApiError ? failure.code : "wake_model_failed";
      if (code !== "connection" && code !== "login_required") rememberOperation(null);
      setError(code);
      if (failure instanceof ApiError && failure.code === "wake_catalog_changed") void loadCatalog();
    } finally {
      setInstalling(false);
    }
  }

  return <section aria-labelledby="wake-word-title">
    <h2 id="wake-word-title">{props.t.wakeWord}</h2>
    <Switch label={props.t.listening(props.currentModel?.wake_word ?? props.t.defaultWakeWord)}
      hint={props.t.voiceRecovery.listeningHint}
      checked={props.listening} disabled={props.pending} onChange={props.toggleListening} />
    <Show when={props.currentModel?.runtime !== "microwakeword-tflite"}>
      <label class={styles.sensitivity} for="sensitivity">
        <span>{props.t.sensitivity} <output aria-hidden="true">{Math.round(sensitivity() * 100)}%</output></span>
        <input id="sensitivity" type="range" min={0} max={1} step={0.01} value={sensitivity()} disabled={props.pending}
          aria-describedby={failedSensitivity() !== undefined ? "sensitivity-error" : undefined}
          aria-valuetext={`${Math.round(sensitivity() * 100)}%`}
          onInput={event => { setPreview(event.currentTarget.valueAsNumber); setFailedSensitivity(undefined); }}
          onChange={event => void saveSensitivity(event.currentTarget.valueAsNumber)} />
      </label>
      <Show when={failedSensitivity() !== undefined}>
        <div class={styles.modelActions}>
          <p id="sensitivity-error" class={styles.modelError} role="alert">{props.t.sensitivitySaveFailed}</p>
          <Button type="button" disabled={props.pending} onClick={() => {
            const value = failedSensitivity();
            if (value !== undefined) void saveSensitivity(value);
          }}>{props.t.retryWakeCatalog}</Button>
        </div>
      </Show>
    </Show>
    <Show when={props.hasCatalog} fallback={<p class={styles.modelStatus}>{props.t.currentWakeModel} · {props.currentModel
      ? modelLabel(props.currentModel) : props.t.noWakeModel}</p>}>
    <form class={styles.modelForm} onSubmit={event => void install(event)}>
      <label for="wake-model">{props.t.wakeModel}</label>
      <p class={styles.modelStatus}>{props.t.currentWakeModel} · {props.currentModel
        ? modelLabel(props.currentModel)
        : props.modelLoaded ? props.t.localWakeModel : props.t.noWakeModel}</p>
      <select id="wake-model" value={id()} disabled={loading() || installing() || Boolean(operation()) || props.pending || !catalog()?.models.length}
        onChange={event => { setId(event.currentTarget.value); setInstalled(false); setError(undefined); }}>
        <option value="" selected={!id()}>{props.t.chooseWakeModel}</option>
        <For each={catalog()?.models}>{model =>
          <option value={model.id} selected={id() === model.id}>{modelLabel(model)}</option>
        }</For>
      </select>
      <div class={styles.modelActions}>
        <Button variant="primary" type="submit" disabled={installing() || props.pending || (!operation() && (loading() || !canInstall()))}>
          {installing() ? props.t.installingWakeModel : operation() ? props.t.checkWakeInstall : props.t.installWakeModel}
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
    </Show>
  </section>;
}
