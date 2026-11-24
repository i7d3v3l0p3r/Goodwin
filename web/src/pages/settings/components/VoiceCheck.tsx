import { createMemo, createSignal, For, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { Disclosure } from "../../../components/ui-kit/Disclosure";
import { EmptyState, InlineError, StatusIndicator } from "../../../components/ui-kit/Feedback";
import type { Report, Snapshot, VoiceReport } from "../../../helpers/api";
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
  const last = () => props.snapshot.last_voice ?? props.snapshot.last_voice_test;
  const inProgress = () => Boolean(active() || starting());
  const phase = () => {
    const stage = active()?.stage;
    return stage ? props.t.stages[stage] : props.t.startingVoiceTest;
  };
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
    <h3 id="test-title">{props.t.testStation}</h3>
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
    <Show when={inProgress()}>
      <div class={styles.progress} role="status">
        <StatusIndicator tone="pending">{phase()}</StatusIndicator>
        <Show when={active()}>{report =>
          <Timings t={props.t} language={props.language} report={report()} />
        }</Show>
      </div>
    </Show>
    <section class={styles.details} aria-labelledby="last-voice-title">
      <h4 id="last-voice-title">{props.t.lastVoiceRequest}</h4>
      <Show when={last()} fallback={<EmptyState>{props.t.noRequest}</EmptyState>}>{report =>
        <RequestDetails t={props.t} language={props.language} report={report()} />
      }</Show>
      <Show when={last()?.source === "wake" && props.snapshot.last_voice_test}>{report =>
        <Disclosure title={props.t.lastVoiceTest}>
          <RequestDetails t={props.t} language={props.language} report={report()} />
        </Disclosure>
      }</Show>
    </section>
  </section>;
}

interface ReportProps {
  t: Words;
  language: Language;
  report: Report;
}

function RequestDetails(props: Omit<ReportProps, "report"> & { report: VoiceReport }) {
  const details = () => props.report;
  const dateFormat = createMemo(() => new Intl.DateTimeFormat(props.language, { dateStyle: "medium", timeStyle: "medium" }));
  const numberFormat = createMemo(() => new Intl.NumberFormat(props.language));
  const modes = () => ({
    wake: props.t.wakeRequest, manual_test: props.t.manualTest, wake_test: props.t.wakeTest, chat: props.t.chat,
  });
  const failure = (code: string) => {
    const message = errorLabel(props.t, code);
    const stage = details().error_stage;
    return stage ? `${props.t.timingStages[stage]} · ${message}` : message;
  };
  const duration = () => props.report.timings.reduce((total, stage) => total + stage.ms, 0);
  return <div class={styles.report}>
    <div class={styles.summary}>
      <span>{modes()[props.report.source]}</span>
      <Show when={details().started_at_ms}>{start =>
        <time datetime={new Date(start()).toISOString()}>{dateFormat().format(start())}</time>
      }</Show>
      <span>{numberFormat().format(duration())}&nbsp;{props.t.ms}</span>
    </div>
    <Show when={props.report.error} fallback={<StatusIndicator tone="success">{props.t.completed}</StatusIndicator>}>{code =>
      <InlineError announce={false}>{failure(code())}</InlineError>
    }</Show>
    <Show when={details().transcript}>{transcript =>
      <div class={styles.text}><h5>{props.t.transcript}</h5><p>{transcript()}</p></div>
    }</Show>
    <Show when={details().answer}>{answer =>
      <div class={styles.text}><h5>{props.t.voiceAnswer}</h5><p>{answer()}</p></div>
    }</Show>
    <Show when={props.report.timings.length}>
      <div><h5>{props.t.timings}</h5><Timings {...props} /></div>
    </Show>
    <Disclosure title={props.t.requestConfiguration}>
      <dl class={styles.configuration}>
        <dt>{props.t.transcriptionProvider}</dt><dd>{modelLabel(details().transcription, props.t)}</dd>
        <dt>{props.t.chatProvider}</dt><dd>{modelLabel(details().chat, props.t)}</dd>
        <dt>{props.t.speechProvider}</dt><dd>{modelLabel(details().speech, props.t)}</dd>
        <Show when={details().playback}>{mode => <>
          <dt>{props.t.timingStages.playing}</dt><dd>{props.t.playbackModes[mode()]}</dd>
        </>}</Show>
      </dl>
    </Disclosure>
  </div>;
}

function modelLabel(model: VoiceReport["transcription"], t: Words) {
  return `${model.provider} · ${model.model ?? t.defaultModel}`;
}

function Timings(props: ReportProps) {
  const numberFormat = createMemo(() => new Intl.NumberFormat(props.language));
  return <dl class={styles.timings}>
    <For each={props.report.timings}>{timing => <>
      <dt>{props.t.timingStages[timing.stage]}</dt>
      <dd>{numberFormat().format(timing.ms)}&nbsp;{props.t.ms}</dd>
    </>}</For>
  </dl>;
}
