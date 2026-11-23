import { createMemo, createSignal, For, Match, Show, Switch } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { Disclosure } from "../../../components/ui-kit/Disclosure";
import { EmptyState, InlineError, StatusIndicator } from "../../../components/ui-kit/Feedback";
import type { Report, Snapshot } from "../../../helpers/api";
import { errorLabel, type Language, type Words } from "../../../helpers/words";
import styles from "./VoiceCheck.module.css";

export function isVoiceTest(report?: Report | null) {
  return report?.source === "manual_test" || report?.source === "wake_test";
}

type TestMode = "manual" | "wake";

export function VoiceCheck(props: {
  t: Words;
  language: Language;
  snapshot: Snapshot;
  reachable: boolean;
  pending: boolean;
  test: (mode: TestMode, onError: (code: string) => void) => Promise<boolean>;
}) {
  const [starting, setStarting] = createSignal<TestMode>();
  const [error, setError] = createSignal("");
  const active = () => isVoiceTest(props.snapshot.active) ? props.snapshot.active : null;
  const last = () => props.snapshot.last_voice_test ?? (isVoiceTest(props.snapshot.last) ? props.snapshot.last : null);
  const report = () => active() ?? (starting() ? null : last());
  const inProgress = () => Boolean(active() || starting());
  const phase = () => {
    const stage = active()?.stage;
    return stage ? props.t.stages[stage] : props.t.startingVoiceTest;
  };
  const modeLabel = () => {
    const result = report();
    return result?.source === "wake_test" || (!result && starting() === "wake") ? props.t.wakeTest : props.t.manualTest;
  };
  const formatTime = createMemo(() => new Intl.NumberFormat(props.language));
  const reason = () => {
    if (!props.reachable) return props.t.disconnected;
    if (inProgress()) return "";
    if (props.snapshot.settings.microphone_muted) return props.t.errors.microphone_muted;
    if (!props.snapshot.microphone) return props.t.errors.microphone;
    if (props.pending || props.snapshot.active) return props.t.errors.busy;
    return "";
  };
  const disabled = () => Boolean(inProgress() || reason());
  const wakeReason = () => !disabled() && !props.snapshot.model ? props.t.errors.model : "";

  async function start(mode: TestMode) {
    if (disabled() || (mode === "wake" && !props.snapshot.model)) return;
    setError("");
    setStarting(mode);
    try {
      await props.test(mode, setError);
    } finally {
      setStarting(undefined);
    }
  }

  return <section aria-labelledby="test-title">
    <h3 id="test-title">{props.t.voiceRequest}</h3>
    <p class={styles.note}>{props.t.testHint}</p>
    <div class={styles.actions}>
      <Button type="button" disabled={disabled()} aria-describedby={reason() ? "test-unavailable" : undefined}
        onClick={() => void start("manual")}>{props.t.manual}</Button>
      <Button type="button" disabled={disabled() || !props.snapshot.model}
        aria-describedby={reason() || wakeReason() ? "test-unavailable" : undefined} onClick={() => void start("wake")}>
        {props.t.byWake(props.snapshot.wake_model?.wake_word ?? props.t.defaultWakeWord)}
      </Button>
    </div>
    <Show when={reason() || wakeReason()}><p id="test-unavailable" class={styles.note}>{reason() || wakeReason()}</p></Show>
    <Show when={error()}>
      <div class={styles.startError}><InlineError>{errorLabel(props.t, error())}</InlineError></div>
    </Show>
    <div class={styles.details}>
      <h3>{inProgress() ? props.t.currentVoiceTest : props.t.lastVoiceTest}</h3>
      <div class={styles.result} role="status">
        <Show when={inProgress() || report()}><span class={styles.note}>{modeLabel()}</span></Show>
        <Switch fallback={<EmptyState>{props.t.noRequest}</EmptyState>}>
          <Match when={inProgress()}>
            <StatusIndicator tone="pending">{phase()}</StatusIndicator>
          </Match>
          <Match when={report()?.error}>
            {code => <InlineError announce={false}>{errorLabel(props.t, code())}</InlineError>}
          </Match>
          <Match when={report()}><StatusIndicator tone="success">{props.t.completed}</StatusIndicator></Match>
        </Switch>
      </div>
      <Show when={report()?.timings.length}>
        <Disclosure title={props.t.timings}>
          <dl>
            <For each={report()?.timings}>{timing => <>
              <dt>{props.t.timingStages[timing.stage]}</dt>
              <dd>{formatTime().format(timing.ms)}&nbsp;{props.t.ms}</dd>
            </>}</For>
          </dl>
        </Disclosure>
      </Show>
    </div>
  </section>;
}
