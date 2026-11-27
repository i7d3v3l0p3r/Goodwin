import { useNavigate } from "@solidjs/router";
import { createSignal, For, Match, Show, Switch } from "solid-js";
import { PageHeader } from "../../components/PageHeader";
import { Button, ButtonLink } from "../../components/ui-kit/Button";
import { InlineError, StatusIndicator } from "../../components/ui-kit/Feedback";
import { usePanel } from "../../helpers/panel";
import { assistantStagePath, capabilities, stationPath } from "../../helpers/readiness";
import { errorLabel } from "../../helpers/words";
import { VoiceCheck, isVoiceTest } from "../settings/components/VoiceCheck";
import styles from "./SetupPage.module.css";

const providerPackage = "https://github.com/i7d3v3l0p3r/Goodwin/releases/latest/download/openai-compatible-wasm32.tar.gz";
const steps = ["service", "text", "voice"] as const;

export function SetupPage() {
  const panel = usePanel();
  const navigate = useNavigate();
  const [selected, setSelected] = createSignal<typeof steps[number]>();
  const [uploading, setUploading] = createSignal(false);
  const copy = () => panel.t().firstRun;
  const issues = () => capabilities.map(capability => ({ capability, issue: panel.stageIssue(capability) }));
  const missing = () => issues().find(item => item.issue && item.issue.kind !== "checking");
  const providerReady = () => issues().every(item => !item.issue);
  const textReport = () => panel.snapshot()?.last_chat_test;
  const textRunning = () => Boolean(panel.snapshot()?.active?.chat_test);
  const textCurrent = () => Boolean(panel.snapshot()?.chat_test_current);
  const textReply = () => textCurrent() && !textReport()?.error && textReport()?.chat_test?.answered;
  const textError = () => textCurrent() ? textReport()?.error : undefined;
  const step = () => !providerReady() ? "service" : selected() ?? (textReply() ? "voice" : "text");
  const voiceReply = () => {
    const state = panel.snapshot();
    const test = state?.last_voice_test;
    return state?.voice_test_current && !isVoiceTest(state.active) && test && !test.error
      && (test.playback === "buffered" || test.playback === "streaming");
  };
  let archive: HTMLInputElement | undefined;
  let heading: HTMLHeadingElement | undefined;

  function goTo(next: typeof steps[number]) {
    setSelected(next);
    heading?.focus();
  }

  function skip() {
    panel.dismissSetup();
    navigate(stationPath, { replace: true });
  }

  return <>
    <PageHeader title={providerReady() ? copy().checkTitle : copy().title} showRequestStatus={!isVoiceTest(panel.snapshot()?.active)} />
    <div class={["gw-page", styles.page]}>
      <div class={styles.progress}>
        <ol class={styles.steps}>
          <For each={steps}>{name =>
            <li data-complete={steps.indexOf(name) < steps.indexOf(step()) ? "true" : undefined}
              aria-current={step() === name ? "step" : undefined}>{copy().steps[name]}</li>
          }</For>
        </ol>
        <Button type="button" variant="quiet" onClick={skip}>{copy().skip}</Button>
      </div>
      <section class={styles.card} aria-labelledby="setup-step-title">
        <h2 id="setup-step-title" ref={heading} tabindex="-1">
          <Switch>
            <Match when={step() === "text"}>{copy().text}</Match>
            <Match when={step() === "voice"}>{copy().voice}</Match>
            <Match when={providerReady()}>{copy().connected}</Match>
            <Match when={missing()}>{item => item().issue?.kind === "install"
              ? copy().packageTitle : copy().connectStage(copy().stageNames[item().capability])}</Match>
            <Match when={true}>{copy().provider}</Match>
          </Switch>
        </h2>
        <Switch>
          <Match when={step() === "service" && providerReady()}>
            <p>{copy().connectedHint}</p>
            <div class="gw-row">
              <Button type="button" variant="primary" onClick={() => goTo("text")}>{copy().sendTestNext}</Button>
            </div>
          </Match>
          <Match when={step() === "service"}>
            <Show when={missing()?.issue} fallback={<p role="status">{panel.reachable() ? copy().checking : panel.status()}</p>}>{issue =>
              <Switch>
                <Match when={issue().kind === "install"}>
                  <p>{copy().packageHint}</p>
                  <div class="gw-row">
                    <ButtonLink href={providerPackage}>{copy().download}</ButtonLink>
                    <Button type="button" variant="primary" disabled={panel.pending() || !panel.reachable()} onClick={() => archive?.click()}>
                      {uploading() ? copy().uploading : copy().upload}
                    </Button>
                    <input ref={archive} type="file" hidden accept=".tar.gz,.tgz,application/gzip" onChange={event => {
                      const file = event.currentTarget.files?.[0];
                      event.currentTarget.value = "";
                      if (file) {
                        setUploading(true);
                        void panel.uploadPlugin(file).finally(() => setUploading(false));
                      }
                    }} />
                  </div>
                </Match>
                <Match when={issue().kind === "configure" || issue().kind === "select"}>
                  <p>{issue().kind === "configure" ? copy().configureHint : copy().selectHint}</p>
                  <div class="gw-row">
                    <ButtonLink variant="primary" href={issue().href}>
                      {issue().kind === "configure" ? copy().configure : copy().select}
                    </ButtonLink>
                  </div>
                </Match>
              </Switch>
            }</Show>
          </Match>
          <Match when={step() === "text"}>
            <Show when={!textReply()}><p>{copy().textHint}</p></Show>
            <Show when={textRunning()} fallback={
              <Show when={textReport() && !textCurrent()} fallback={
                <Show when={textReply()}><p role="status"><StatusIndicator tone="success">{copy().textSuccess}</StatusIndicator></p></Show>
              }><p role="status"><StatusIndicator tone="pending">{copy().repeatTest}</StatusIndicator></p></Show>
            }><p role="status"><StatusIndicator tone="pending">{panel.status()}</StatusIndicator></p></Show>
            <Show when={!textRunning() && textError()}>{code => <InlineError>{errorLabel(panel.t(), code())}</InlineError>}</Show>
            <Show when={!textRunning() && textReport() && textCurrent() && !textError() && !textReply()}>
              <InlineError>{copy().emptyTestReply}</InlineError>
            </Show>
            <Show when={textReport()?.chat_test?.model}>{model =>
              <p>{panel.t().chatProvider} · {model().provider} · {model().model ?? panel.t().defaultModel}</p>
            }</Show>
            <div class="gw-row">
              <Button type="button" variant="quiet" onClick={() => goTo("service")}>{copy().back}</Button>
              <Show when={textReply()} fallback={
                <Button type="button" variant="primary" disabled={!panel.reachable() || panel.busy()}
                  onClick={() => { setSelected("text"); void panel.testChat(copy().textMessage); }}>{copy().sendTest}</Button>
              }>
                <Button type="button" variant="primary" onClick={() => goTo("voice")}>{copy().voiceNext}</Button>
                <a href="/#conversation">{copy().inspectChat}</a>
              </Show>
              <Show when={textError()}><a href={assistantStagePath("chat")}>{copy().checkService}</a></Show>
            </div>
          </Match>
          <Match when={step() === "voice"}>
            <Show when={panel.snapshot()}>{state => <>
              <Show when={voiceReply()} fallback={
                <p>{state().model && state().settings.listening
                  ? copy().wakeInstructions(state().wake_model?.wake_word ?? panel.t().defaultWakeWord)
                  : copy().manualInstructions}</p>
              }><p role="status"><StatusIndicator tone="success">{copy().voiceSuccess}</StatusIndicator></p></Show>
              <VoiceCheck t={panel.t()} language={panel.language()} snapshot={state()} reachable={panel.reachable()}
                pending={panel.pending()} setup test={panel.testStation} />
            </>}</Show>
            <div class="gw-row">
              <Button type="button" variant="quiet" onClick={() => goTo("text")}>{copy().back}</Button>
              <Show when={voiceReply()}>
                <ButtonLink href={stationPath} variant="primary">{copy().finish}</ButtonLink>
              </Show>
            </div>
          </Match>
        </Switch>
        <Show when={panel.error()}>{code => <InlineError>{errorLabel(panel.t(), code())}</InlineError>}</Show>
      </section>
    </div>
  </>;
}
