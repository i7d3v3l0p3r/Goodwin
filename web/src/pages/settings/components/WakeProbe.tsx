import { createEffect, createSignal, For, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { api, ApiError } from "../../../helpers/api";
import { usePanel } from "../../../helpers/panel";
import { errorLabel } from "../../../helpers/words";
import styles from "./WakeProbe.module.css";

export function WakeProbe() {
  const panel = usePanel();
  const t = () => panel.t().now;
  const state = panel.snapshot;
  const [results, setResults] = createSignal<boolean[]>([]);
  const [request, setRequest] = createSignal<number>();
  const [starting, setStarting] = createSignal(false);
  const [error, setError] = createSignal("");
  let profile = "";
  const word = () => state()?.wake_model?.wake_word ?? panel.t().defaultWakeWord;
  const blocked = () => !panel.reachable() ? panel.t().disconnected : !state()?.model ? panel.t().errors.model
    : state()?.settings.microphone_muted ? panel.t().errors.microphone_muted : !state()?.microphone ? panel.t().errors.microphone : "";
  createEffect(() => [request(), state()?.last_probe] as const, ([id, report]) => {
    if (!id || !report || report.request_id !== id) return;
    setRequest(undefined);
    if (report.error && report.error !== "no_wake") setError(errorLabel(panel.t(), report.error));
    else setResults(previous => [...previous, !report.error]);
  });
  createEffect(() => [state()?.settings.sensitivity, state()?.settings.wake_gain, state()?.wake_model?.id].join("|"),
    next => {
      if (next === profile) return;
      profile = next; setResults([]); setRequest(undefined); setError("");
    });
  async function attempt() {
    if (starting() || request() || blocked() || state()?.active) return;
    setStarting(true); setError("");
    try {
      const { request_id } = await api<{request_id: number}>("test", "POST", { mode: "probe" });
      setRequest(request_id);
    } catch (e) { setError(errorLabel(panel.t(), e instanceof ApiError ? e.code : "internal")); }
    finally { setStarting(false); }
  }
  return <section class={styles.probe} aria-labelledby="wake-probe-title">
    <h2 id="wake-probe-title">{t().probe}</h2><p>{t().probeHint(word())}</p>
    <ol><For each={[0, 1, 2]}>{i => <li data-outcome={results()[i] === undefined ? undefined : results()[i] ? "hit" : "miss"}>
      <span>{i+1}</span>{results()[i] === undefined ? "—" : results()[i] ? t().hit : t().miss}</li>}</For></ol>
    <p role="status">{error() || blocked() || (request() ? t().listen : results().length ? t().score(results().filter(Boolean).length, results().length) : "")}</p>
    <div class="gw-row"><Show when={results().length < 3} fallback={
      <Button type="button" onClick={() => setResults([])}>{t().reset}</Button>}>
      <Button type="button" disabled={!!blocked() || starting() || !!request() || !!state()?.active} onClick={() => void attempt()}>{t().attempt(results().length+1)}</Button>
    </Show>
      <Show when={request()}><Button type="button" variant="quiet" onClick={() => {
        const id = request(); setRequest(undefined);
        if (id && state()?.active?.request_id === id) void panel.cancelVoice(id);
      }}>{panel.t().cancel}</Button></Show>
    </div>
  </section>;
}
