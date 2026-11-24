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
    <h3 id="logs-title">{t().diagnosticLogs}</h3>
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
  return <div>
    <h4>{props.title}</h4>
    <Show when={props.value} fallback={<EmptyState>{props.value == null ? props.t.logUnavailable : props.t.logEmpty}</EmptyState>}>
      <pre tabindex={0} aria-label={props.title}>{props.value}</pre>
    </Show>
  </div>;
}
