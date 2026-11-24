import { createSignal, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { EmptyState, InlineError } from "../../../components/ui-kit/Feedback";
import { ApiError, api } from "../../../helpers/api";
import { errorLabel, type Words } from "../../../helpers/words";
import styles from "./DiagnosticLogs.module.css";

interface Logs {
  supported: boolean;
  kernel: string | null;
  goodwin: string | null;
}

export function DiagnosticLogs(props: { t: Words; passwordProtected: boolean }) {
  const [logs, setLogs] = createSignal<Logs>();
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal("");
  const t = () => props.t;

  async function refresh() {
    if (loading()) return;
    setLoading(true);
    setError("");
    try {
      setLogs(await api<Logs>("diagnostics/logs"));
    } catch (reason) {
      setError(reason instanceof ApiError ? reason.code : "internal");
    } finally {
      setLoading(false);
    }
  }

  return <section aria-labelledby="logs-title">
    <h2 id="logs-title">{t().diagnosticLogs}</h2>
    <Show when={props.passwordProtected} fallback={<EmptyState>{t().errors.diagnostics_password_required}</EmptyState>}>
      <p class={styles.note}>{t().logsHint}</p>
      <Button type="button" aria-busy={loading() ? "true" : "false"} disabled={loading()} onClick={() => void refresh()}>
        {loading() ? t().loadingLogs : logs() ? t().refreshLogs : t().loadLogs}
      </Button>
      <Show when={error()}><InlineError>{errorLabel(t(), error())}</InlineError></Show>
      <Show when={logs()}>{current =>
        <Show when={current().supported} fallback={<EmptyState>{t().logsUnavailable}</EmptyState>}>
          <div class={styles.logs}>
            <Log title={t().kernelLog} value={current().kernel} t={t()} />
            <Log title={t().goodwinLog} value={current().goodwin} t={t()} />
          </div>
        </Show>
      }</Show>
    </Show>
  </section>;
}

function Log(props: { title: string; value: string | null; t: Words }) {
  const [feedback, setFeedback] = createSignal<"copied" | "selected">();
  let log!: HTMLPreElement;

  async function copy() {
    if (!props.value) return;
    try {
      if (navigator.clipboard) {
        await navigator.clipboard.writeText(props.value);
        setFeedback("copied");
        return;
      }
    } catch {
      // The panel can also be served over HTTP on the local network.
    }
    const selection = window.getSelection();
    if (!selection) return;
    const range = document.createRange();
    range.selectNodeContents(log);
    selection.removeAllRanges();
    selection.addRange(range);
    setFeedback("selected");
    log.focus({ preventScroll: true });
  }

  return <div>
    <h3>{props.title}</h3>
    <Show when={props.value} fallback={<EmptyState>{props.value == null ? props.t.logUnavailable : props.t.logEmpty}</EmptyState>}>
      <div class="gw-row">
        <Button type="button" variant="quiet" onClick={() => void copy()}>{props.t.copyLog}</Button>
        <span role="status">{feedback() === "copied" ? props.t.logCopied : feedback() === "selected" ? props.t.logSelected : ""}</span>
      </div>
      <pre ref={log} tabindex={0} aria-label={props.title}>{props.value}</pre>
    </Show>
  </div>;
}
