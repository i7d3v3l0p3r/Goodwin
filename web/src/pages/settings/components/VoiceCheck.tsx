import { createMemo, createSignal, For, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { Disclosure } from "../../../components/ui-kit/Disclosure";
import { EmptyState, InlineError, StatusIndicator } from "../../../components/ui-kit/Feedback";
import type { Report, Snapshot, VoiceReport } from "../../../helpers/api";
import { usePanel } from "../../../helpers/panel";
import { assistantStagePath } from "../../../helpers/readiness";
import { errorLabel, type Language, type Stage, type Words } from "../../../helpers/words";
import { settingsSectionPath } from "../sections";
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
  setup?: boolean;
  blocked?: string;
  test: (mode: TestMode, onError: (code: string) => void) => Promise<boolean>;
}) {
  const panel = usePanel();
  const [starting, setStarting] = createSignal<TestMode>();
  const [error, setError] = createSignal("");
  const active = () => isVoiceTest(props.snapshot.active) ? props.snapshot.active : null;
  const last = () => props.setup ? props.snapshot.last_voice_test : props.snapshot.last_voice ?? props.snapshot.last_voice_test;
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
    if (props.blocked) return props.blocked;
    if (props.pending || props.snapshot.active) return props.t.errors.busy;
    return "";
  };
  const disabled = () => Boolean(inProgress() || reason());
  const wakeAvailable = () => props.snapshot.model && props.snapshot.settings.listening;
  const wakeReason = () => !disabled() && !props.snapshot.model ? props.t.errors.model : "";
  const instruction = () => active()?.stage === "recording" ? props.t.firstRun.speakNow : phase();

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

  return <section aria-label={props.setup ? props.t.testStation : undefined} aria-labelledby={props.setup ? undefined : "test-title"}>
    <Show when={!props.setup}>
      <h2 id="test-title">{props.t.testStation}</h2>
      <p class={styles.note}>{props.t.testHint}</p>
    </Show>
    <Show when={props.setup} fallback={<div class={styles.actions}>
      <Button type="button" disabled={disabled()} aria-describedby={reason() ? "test-unavailable" : undefined}
        onClick={() => void start("manual")}>{props.t.manual}</Button>
      <Button type="button" disabled={disabled() || !props.snapshot.model}
        aria-describedby={reason() || wakeReason() ? "test-unavailable" : undefined} onClick={() => void start("wake")}>
        {props.t.byWake(props.snapshot.wake_model?.wake_word ?? props.t.defaultWakeWord)}
      </Button>
    </div>}>
      <div class={styles.actions}>
        <Button type="button" variant={props.snapshot.voice_test_current && last()?.playback ? "secondary" : "primary"}
          disabled={disabled()} aria-describedby={reason() ? "test-unavailable" : undefined}
          onClick={() => void start(wakeAvailable() ? "wake" : "manual")}>{props.t.firstRun.voiceStart}</Button>
        <Show when={wakeAvailable()}>
          <Button type="button" variant="quiet" disabled={disabled()} aria-describedby={reason() ? "test-unavailable" : undefined}
            onClick={() => void start("manual")}>{props.t.firstRun.noWakeWord}</Button>
        </Show>
      </div>
    </Show>
    <Show when={reason() || (!props.setup && wakeReason())}>
      <p id="test-unavailable" class={styles.note}>{reason() || wakeReason()}</p>
      <Show when={props.reachable && props.snapshot.settings.microphone_muted}>
        <Button type="button" variant="quiet" disabled={panel.microphonePending()}
          onClick={() => void panel.setMicrophoneMuted(false)}>{props.t.unmuteMicrophone}</Button>
      </Show>
      <Show when={props.reachable && !props.snapshot.settings.microphone_muted && (!props.snapshot.microphone || wakeReason())}>
        <FailureAction t={props.t} code={wakeReason() ? "model" : "microphone"} />
      </Show>
    </Show>
    <Show when={error()}>
      <div class={styles.startError}>
        <InlineError>{errorLabel(props.t, error())}</InlineError>
        <FailureAction t={props.t} code={error()} />
      </div>
    </Show>
    <Show when={inProgress()}>
      <div class={styles.progress} role="status">
        <StatusIndicator tone="pending">{props.setup ? instruction() : phase()}</StatusIndicator>
        <Show when={!props.setup}>
          <Show when={active()}>{report => <Timings t={props.t} language={props.language} report={report()} />}</Show>
        </Show>
      </div>
    </Show>
    <Show when={props.setup} fallback={<section class={styles.details} aria-labelledby="last-voice-title">
      <h3 id="last-voice-title">{props.t.lastVoiceRequest}</h3>
      <Show when={last()} fallback={<EmptyState>{props.t.noRequest}</EmptyState>}>{report =>
        <RequestDetails t={props.t} language={props.language} report={report()}
          stale={isVoiceTest(report()) && props.snapshot.voice_test_current === false} />
      }</Show>
      <Show when={(last()?.source === "wake" || last()?.source === "follow_up") && props.snapshot.last_voice_test}>{report =>
        <Disclosure title={props.t.lastVoiceTest}>
          <RequestDetails t={props.t} language={props.language} report={report()} stale={props.snapshot.voice_test_current === false} />
        </Disclosure>
      }</Show>
    </section>}>
      <Show when={last() && !inProgress() ? last() : undefined}>{report =>
        <Disclosure title={props.t.lastVoiceTest} open={Boolean(report().error || props.snapshot.voice_test_current === false)}>
          <RequestDetails t={props.t} language={props.language} report={report()} showConfiguration
            stale={props.snapshot.voice_test_current === false} />
        </Disclosure>
      }</Show>
    </Show>
  </section>;
}

interface ReportProps {
  t: Words;
  language: Language;
  report: Report;
}

export function EmbeddedVoiceDiagnostics(props: {
  t: Words;
  language: Language;
  snapshot: Snapshot;
  pending: boolean;
  test: (mode: TestMode, onError: (code: string) => void) => Promise<boolean>;
}) {
  const active = () => props.snapshot.active?.source === "chat" ? null : props.snapshot.active;
  const [error, setError] = createSignal("");
  const disabled = () => props.pending || !!props.snapshot.active
    || !(props.snapshot.voice_provider_ready ?? props.snapshot.provider_ready) || props.snapshot.settings.microphone_muted;
  const wakeWord = () => props.snapshot.wake_model?.wake_word ?? props.t.defaultWakeWord;
  const start = (mode: TestMode) => { setError(""); void props.test(mode, setError); };
  return <section class={styles.details} aria-labelledby="last-voice-title">
    <h2 id="last-voice-title">{props.t.lastVoiceRequest}</h2>
    <p class={styles.note}>{props.t.embedded.pushToTalkHint}</p>
    <div class="gw-row">
      <Button type="button" onClick={() => start("manual")} disabled={disabled()}>
        {props.t.firstRun.noWakeWord}
      </Button>
      <Show when={props.snapshot.model}>
        <Button type="button" onClick={() => start("wake")} disabled={disabled()}>
          {props.t.byWake(wakeWord())}
        </Button>
      </Show>
    </div>
    <Show when={error()}><p role="alert">{errorLabel(props.t, error())}</p></Show>
    <Show when={active()}>{report => <div class={styles.progress} role="status">
      <StatusIndicator tone="pending">{props.t.stages[report().stage]}</StatusIndicator>
      <Show when={report().stage === "awaiting_wake"}>
        <p class={styles.note}>{props.t.embedded.wakeTestHint(wakeWord())}</p>
      </Show>
      <Show when={isVoiceTest(report()) && report().stage === "recording"}>
        <p class={styles.note}>{props.t.embedded.speakNow}</p>
      </Show>
      <Timings t={props.t} language={props.language} report={report()} />
    </div>}</Show>
    <Show when={props.snapshot.last_voice} fallback={<EmptyState>{props.t.noRequest}</EmptyState>}>{report =>
      <RequestDetails t={props.t} language={props.language} report={report()} showConfiguration />
    }</Show>
  </section>;
}

export function RequestDetails(props: Omit<ReportProps, "report"> & { report: VoiceReport; stale?: boolean; showConfiguration?: boolean }) {
  const details = () => props.report;
  const dateFormat = createMemo(() => new Intl.DateTimeFormat(props.language, { dateStyle: "medium", timeStyle: "medium" }));
  const numberFormat = createMemo(() => new Intl.NumberFormat(props.language));
  const modes = () => ({
    wake: props.t.wakeRequest, manual_test: props.t.manualTest, wake_test: props.t.wakeTest, chat: props.t.chat,
    push_to_talk: props.t.embedded.pushToTalk, follow_up: props.t.followUp,
    wake_probe: props.t.now.probe, correction: props.t.now.correction,
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
    <Show when={props.stale} fallback={
      <Show when={props.report.error} fallback={<StatusIndicator tone="success">{props.t.completed}</StatusIndicator>}>{code => <>
        <InlineError announce={false}>{failure(code())}</InlineError>
        <FailureAction t={props.t} code={code()} stage={details().error_stage ?? props.report.stage} />
      </>}</Show>
    }>
      <StatusIndicator tone="pending">{props.t.firstRun.repeatTest}</StatusIndicator>
    </Show>
    <Show when={details().transcript}>{transcript =>
      <div class={styles.text}><h4>{props.t.transcript}</h4><p>{transcript()}</p></div>
    }</Show>
    <Show when={details().answer}>{answer =>
      <div class={styles.text}><h4>{props.t.voiceAnswer}</h4><p>{answer()}</p></div>
    }</Show>
    <Show when={props.report.timings.length}>
      <div><h4>{props.t.timings}</h4><Timings {...props} /></div>
    </Show>
    <Disclosure title={props.t.requestConfiguration} open={props.showConfiguration}>
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

function FailureAction(props: { t: Words; code: string; stage?: Stage }) {
  const panel = usePanel();
  const recovery = () => {
    const t = props.t.voiceRecovery;
    if (panel.builtinProvider()) {
      if (props.code === "microphone" || props.code === "no_speech") {
        return { hint: props.t.embedded.pushToTalkHint };
      }
      if (props.code === "provider_unavailable") {
        return { href: settingsSectionPath("assistant"), label: props.t.embedded.provider };
      }
    }
    switch (props.code) {
      case "model":
      case "no_wake":
        return { href: settingsSectionPath("activation"), label: t.activation };
      case "microphone":
      case "microphone_muted":
      case "no_speech":
        return { href: settingsSectionPath("activation"), label: t.microphone };
      case "playback":
        return { hint: t.playback };
    }
    switch (props.stage) {
      case "transcribing": return { href: assistantStagePath("transcription"), label: t.transcription };
      case "thinking": return { href: assistantStagePath("chat"), label: t.chat };
      case "synthesizing":
      case "playing": return { href: assistantStagePath("speech"), label: t.speech };
    }
  };
  return <Show when={recovery()}>{action =>
    <Show when={action().href} fallback={<p class={styles.note}>{action().hint}</p>}>{href =>
      <a class={styles.recoveryLink} href={href()}>{action().label}</a>
    }</Show>
  }</Show>;
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
