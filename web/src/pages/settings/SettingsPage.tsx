import { useLocation } from "@solidjs/router";
import { createEffect, createMemo, For, Show } from "solid-js";
import { Icon } from "../../components/icons";
import { PageHeader } from "../../components/PageHeader";
import { usePanel } from "../../helpers/panel";
import { errorLabel, type Language } from "../../helpers/words";
import { capabilities, setupPath, stationPath } from "../../helpers/readiness";
import { pluginSettingsPath } from "../../helpers/plugins";
import { AssistantForm } from "./components/AssistantForm";
import { CoreUpdate } from "./components/CoreUpdate";
import { DiagnosticLogs } from "./components/DiagnosticLogs";
import { SystemResources } from "./components/SystemResources";
import { WakeWordSettings } from "./components/WakeWordSettings";
import { MicrophoneGainForm } from "./components/MicrophoneGainForm";
import { SystemPromptForm } from "./components/SystemPromptForm";
import { isVoiceTest, VoiceCheck } from "./components/VoiceCheck";
import { settingsSectionPath, settingsSections } from "./sections";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  const panel = usePanel();
  const route = useLocation();
  const pending = () => panel.pending() || !panel.reachable();
  const active = createMemo(() => {
    const path = route.pathname.replace(/\/$/, "");
    return settingsSections.find(section => settingsSectionPath(section.id) === path);
  });
  const assistantVisited = createMemo<boolean>(visited => visited === true || active()?.id === "assistant");
  const focusStage = createMemo(() => {
    if (active()?.id !== "assistant") return undefined;
    const requested = new URLSearchParams(route.search).get("stage");
    return capabilities.find(capability => requested === capability);
  });
  let body!: HTMLDivElement;

  createEffect(() => active()?.id, (section, previous) => {
    if (!section && !previous) return;
    body.scrollTop = 0;
    const target = section ? "panel-page-title" : `settings-link-${previous}`;
    if (!focusStage()) document.getElementById(target)?.focus({ preventScroll: Boolean(section) });
  });

  return <>
    <PageHeader title={panel.t()[active()?.title ?? "settings"]}
      showRequestStatus={active()?.id !== "diagnostics" || !isVoiceTest(panel.snapshot()?.active)} />
    <div ref={body} class="gw-page">
      <nav aria-label={panel.t().settings} hidden={!!active()}>
        <For each={settingsSections}>{section =>
          <a id={`settings-link-${section.id}`} class={styles.destination} href={settingsSectionPath(section.id)}>
            <Icon name={section.icon} />
            <span class={styles.destinationText}>
              <strong>{panel.t()[section.title]}</strong>
              <small>{panel.t()[section.description]}</small>
            </span>
            <Icon name="chevronRight" size={17} />
          </a>
        }</For>
      </nav>
      <Show when={active() && active()?.id !== "assistant" && active()?.id !== "panel"}>
        <a class={styles.inlineLink} href={stationPath}><Icon name="arrowLeft" size={17} />{panel.t().station}</a>
      </Show>
      <For each={settingsSections}>{section =>
        <section class={styles.section} aria-labelledby="panel-page-title" hidden={active()?.id !== section.id}>
          {section.id === "activation" && <Show when={panel.snapshot()}>{state => <>
            <WakeWordSettings t={panel.t()} listening={state().settings.listening} sensitivity={state().settings.sensitivity}
              visible={active()?.id === "activation"} currentModel={state().wake_model} modelLoaded={state().model}
              changeSensitivity={sensitivity => panel.saveSettings({ ...state().settings, sensitivity }, { showError: false })}
              pending={pending()} toggleListening={() => {
                const settings = state().settings;
                void panel.saveSettings({ ...settings, listening: !settings.listening });
              }} />
            <MicrophoneGainForm t={panel.t()} snapshot={state()} pending={pending()} save={panel.saveSettings} />
            <a class={styles.inlineLink} href={settingsSectionPath("diagnostics")}>
              {panel.t().diagnostics}<Icon name="arrowRight" size={17} />
            </a>
          </>}</Show>}
          {section.id === "assistant" && <Show when={assistantVisited() ? panel.snapshot() : undefined}>{state => <>
            <div class={styles.group}>
              <p>{panel.t().assistantDescription}</p>
              <div class="gw-row">
                <For each={state().plugins.filter(plugin => plugin.status === "ready" && plugin.connection_settings)}>{plugin =>
                  <a class={styles.inlineLink} href={pluginSettingsPath(plugin.id)}>{panel.t().firstRun.manageProviders} · {plugin.title[panel.language()]}</a>
                }</For>
                <a class={styles.inlineLink} href={setupPath}>{panel.assistantNeedsSetup()
                  ? panel.setupDismissed() ? panel.t().firstRun.openWizard : panel.t().firstRun.resume
                  : panel.t().firstRun.checkWork}</a>
              </div>
            </div>
            <AssistantForm t={panel.t()} language={panel.language()} snapshot={state()}
              pending={pending()} error={panel.error()} save={panel.saveSettings} focusStage={focusStage()} />
            <SystemPromptForm t={panel.t()} systemPrompt={state().system_prompt}
              pending={pending()} save={panel.saveSystemPrompt} />
          </>}</Show>}
          {section.id === "diagnostics" && <Show when={active()?.id === "diagnostics"}>
            <Show when={panel.snapshot()}>{state =>
              <VoiceCheck t={panel.t()} language={panel.language()} snapshot={state()} reachable={panel.reachable()}
                pending={panel.pending()} test={panel.testStation} />
              }</Show>
            <Show when={panel.assistantNeedsSetup() && !panel.setupDismissed()}>
              <a class={styles.inlineLink} href={setupPath}>{panel.t().firstRun.resume}</a>
            </Show>
            <SystemResources t={panel.t()} language={panel.language()} />
            <DiagnosticLogs t={panel.t()} passwordProtected={panel.access()?.protected === true} />
          </Show>}
          {section.id === "panel" && <div class={styles.group}>
            <label class={styles.language}>
              {panel.t().language}
              <select value={panel.language()} onChange={event => panel.changeLanguage(event.currentTarget.value as Language)}>
                <option value="en">English</option><option value="ru">Русский</option>
              </select>
            </label>
            <section aria-labelledby="panel-access-title" class={styles.access}>
              <h2 id="panel-access-title">{panel.t().panelAccess}</h2>
              <p>{panel.access()?.protected ? panel.t().passwordAccess : panel.t().openAccess}</p>
              <Show when={panel.access()?.protected === false}>
                <p>{panel.t().passwordInstructions}</p>
                <code>goodwin set-password &lt;data-dir&gt;</code>
              </Show>
            </section>
          </div>}
          {section.id === "update" && <Show when={active()?.id === "update"}>
            <CoreUpdate t={panel.t()} passwordProtected={panel.access()?.protected === true} />
          </Show>}
        </section>
      }</For>
      <Show when={active() && panel.error()}><p class={styles.error} role="alert">{errorLabel(panel.t(), panel.error())}</p></Show>
    </div>
  </>;
}
