import { createMemo, createSignal, onSettled, Show } from "solid-js";
import { EmptyState, InlineError, StatusIndicator } from "../../../components/ui-kit/Feedback";
import { ApiError, api } from "../../../helpers/api";
import { errorLabel, type Language, type Words } from "../../../helpers/words";
import styles from "./SystemResources.module.css";

interface Usage {
  used_bytes: number;
  total_bytes: number;
}

interface Resources {
  supported: boolean;
  cpu_percent: number | null;
  memory: Usage | null;
  storage: Usage | null;
  uptime_seconds: number | null;
  goodwin_cpu_percent: number | null;
  goodwin_rss_bytes: number | null;
}

export function SystemResources(props: { t: Words; language: Language }) {
  const [data, setData] = createSignal<Resources>();
  const [error, setError] = createSignal("");
  const t = () => props.t;
  const numbers = createMemo(() => new Intl.NumberFormat(props.language, { maximumFractionDigits: 1 }));
  const percent = (value?: number | null) => value == null ? t().metricUnavailable : `${numbers().format(value)}%`;
  const bytes = (value: number, large = value >= 1024 ** 3) => large
    ? `${numbers().format(value / 1024 ** 3)} ${t().gib}`
    : `${numbers().format(value / 1024 ** 2)} ${t().mib}`;
  const memoryText = (value: number | null) => value == null ? t().metricUnavailable : bytes(value);
  const usageText = (usage: Usage | null) => {
    if (!usage) return t().metricUnavailable;
    const large = usage.total_bytes >= 1024 ** 3;
    return `${bytes(usage.used_bytes, large)} / ${bytes(usage.total_bytes, large)}`;
  };
  const usagePercent = (usage: Usage | null) => usage && usage.total_bytes > 0
    ? 100 * usage.used_bytes / usage.total_bytes : null;
  const uptime = (seconds: number | null) => {
    if (seconds == null) return t().metricUnavailable;
    const minutes = Math.floor(seconds / 60);
    return t().uptimeValue(Math.floor(minutes / 1440), Math.floor(minutes / 60) % 24, minutes % 60);
  };

  onSettled(() => {
    let stopped = false;
    let timer = 0;
    async function poll() {
      try {
        const sample = await api<Resources>("diagnostics/resources");
        if (stopped) return;
        setData(sample);
        setError("");
        if (!sample.supported) return;
      } catch (reason) {
        if (stopped) return;
        setError(reason instanceof ApiError ? reason.code : "internal");
      }
      if (!stopped) timer = window.setTimeout(poll, 5_000);
    }
    void poll();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  });

  return <section aria-labelledby="resources-title">
    <h2 id="resources-title">{t().systemResources}</h2>
    <Show when={error()}><InlineError>{errorLabel(t(), error())}</InlineError></Show>
    <Show when={data()} fallback={<Show when={!error()}><StatusIndicator tone="pending">{t().loadingResources}</StatusIndicator></Show>}>{sample =>
      <Show when={sample().supported} fallback={<EmptyState>{t().resourcesUnavailable}</EmptyState>}>
        <dl class={styles.metrics}>
          <Metric label={t().systemCpu} value={percent(sample().cpu_percent)} level={sample().cpu_percent} />
          <Metric label={t().systemMemory} value={usageText(sample().memory)} level={usagePercent(sample().memory)} />
          <Metric label={t().dataStorage} value={usageText(sample().storage)} level={usagePercent(sample().storage)} />
          <Metric label={t().systemUptime} value={uptime(sample().uptime_seconds)} />
          <Metric label={t().goodwinCpu} value={percent(sample().goodwin_cpu_percent)} />
          <Metric label={t().goodwinMemory} value={memoryText(sample().goodwin_rss_bytes)} />
        </dl>
      </Show>
    }</Show>
  </section>;
}

function Metric(props: { label: string; value: string; level?: number | null }) {
  return <div class={styles.metric}>
    <dt>{props.label}</dt>
    <dd>{props.value}</dd>
    <Show when={props.level != null}>
      <dd aria-hidden="true"><meter min={0} max={100} value={props.level ?? 0} /></dd>
    </Show>
  </div>;
}
