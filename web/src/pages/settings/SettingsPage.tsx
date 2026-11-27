import { useLocation } from "@solidjs/router";
import { createEffect, createMemo, For, Show } from "solid-js";
import { Icon } from "../../components/icons";
import { PageHeader } from "../../components/PageHeader";
import { usePanel } from "../../helpers/panel";
import { errorLabel, type Language } from "../../helpers/words";
import { capabilities, setupPath, stationPath } from "../../helpers/readiness";
import { pluginSettingsPath } from "../../helpers/plugins";
import { AssistantForm } from "./components/AssistantForm";
import { AssistantLanguagesForm } from "./components/AssistantLanguagesForm";
import { BuiltinProviderForm } from "./components/BuiltinProviderForm";
import { WifiForm } from "./components/WifiForm";
import { CoreUpdate } from "./components/CoreUpdate";
import { FirmwareUpdate } from "./components/FirmwareUpdate";
import { DiagnosticLogs } from "./components/DiagnosticLogs";
import { SystemResources } from "./components/SystemResources";
import { WakeWordSettings } from "./components/WakeWordSettings";
import { WakeProbe } from "./components/WakeProbe";
import { MicrophoneGainForm } from "./components/MicrophoneGainForm";
import { DialogueForm } from "./components/DialogueForm";
import { PanelAccessControls } from "./components/PanelAccessControls";
import { SystemPromptForm } from "./components/SystemPromptForm";
import { EmbeddedVoiceDiagnostics, isVoiceTest, VoiceCheck } from "./components/VoiceCheck";
import { settingsSectionPath, settingsSections } from "./sections";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  const panel = usePanel();
  const route = useLocation();
  const pending = () => panel.pending() || !panel.reachable();
  const sections = () => panel.builtinProvider()
    ? settingsSections
    : settingsSections.filter(section => section.id !== "network");
  const active = createMemo(() => {
    const path = route.pathname.replace(/\/$/, "");
    return sections().find(section => settingsSectionPath(section.id) === path);
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
    <PageHeader title={panel.builtinProvider() && active()?.id === "assistant"
      ? panel.t().embedded.provider : panel.t()[active()?.title ?? "settings"]}
      showRequestStatus={active()?.id !== "diagnostics" || !isVoiceTest(panel.snapshot()?.active)} />
    <div ref={body} class="gw-page">
      <nav class={styles.directory} aria-label={panel.t().settings} hidden={!!active()}>
        <For each={sections()}>{section =>
          <a id={`settings-link-${section.id}`} class={styles.destination} href={settingsSectionPath(section.id)}>
            <Icon name={section.icon} />
            <span class={styles.destinationText}>
              <strong>{panel.builtinProvider() && section.id === "assistant" ? panel.t().embedded.provider : panel.t()[section.title]}</strong>
              <small>{panel.builtinProvider() && section.id === "assistant" ? panel.t().embedded.providerDescription : panel.t()[section.description]}</small>
            </span>
            <Icon name="chevronRight" size={17} />
          </a>
        }</For>
      </nav>
      <Show when={!panel.builtinProvider() && active() && active()?.id !== "assistant" && active()?.id !== "panel"}>
        <a class={styles.inlineLink} href={stationPath}><Icon name="arrowLeft" size={17} />{panel.t().station}</a>
      </Show>
      <For each={sections()}>{section =>
        <section class={styles.section} aria-labelledby="panel-page-title" hidden={active()?.id !== section.id}>
          {section.id === "activation" && <Show when={panel.snapshot()}>{state => <>
            <Show when={state().settings.dialogue && panel.activationModes().some(mode => mode === "wake_word" || mode === "push_to_talk")}>
              <DialogueForm t={panel.t()} settings={state().settings} dialogue={state().settings.dialogue!}
                activation={panel.activationModes()} save={panel.saveSettings}
                pending={pending() || (panel.builtinProvider() && !!state().active && state().active?.stage !== "awaiting_speech")} />
            </Show>
            <Show when={!panel.builtinProvider() || panel.hasWakeRuntime()}>
              <WakeWordSettings t={panel.t()} listening={state().settings.listening} sensitivity={state().settings.sensitivity}
                visible={active()?.id === "activation"} currentModel={state().wake_model} modelLoaded={state().model}
                hasCatalog={panel.hasWakeCatalog()}
                onModelInstalled={panel.refresh}
                changeSensitivity={sensitivity => panel.saveSettings({ ...state().settings, sensitivity }, { showError: false })}
                pending={pending() || (panel.builtinProvider() && !!state().active)} toggleListening={() => {
                  const settings = state().settings;
                  void panel.saveSettings({ ...settings, listening: !settings.listening });
                }} />
              <Show when={active()?.id === "activation"}><WakeProbe /></Show>
            </Show>
            <MicrophoneGainForm t={panel.t()} snapshot={state()} pending={pending() || (panel.builtinProvider() && !!state().active)}
              showWakeGain={!panel.builtinProvider() || panel.hasWakeRuntime()} save={panel.saveSettings} />
            <a class={styles.inlineLink} href={settingsSectionPath("diagnostics")}>
              {panel.t().diagnostics}<Icon name="arrowRight" size={17} />
            </a>
          </>}</Show>}
          {section.id === "assistant" && <Show when={panel.builtinProvider()} fallback={
            <Show when={assistantVisited() ? panel.snapshot() : undefined}>{state => <>
            <div class={["gw-row", styles.providerLinks]}>
                <For each={state().plugins.filter(plugin => plugin.status === "ready" && plugin.connection_settings)}>{plugin =>
                  <a class={styles.inlineLink} href={pluginSettingsPath(plugin.id)}><Icon name="plugin" size={16} />{panel.t().firstRun.manageProviders} · {plugin.title[panel.language()]}</a>
                }</For>
                <a class={styles.inlineLink} href={setupPath}>{panel.assistantNeedsSetup()
                  ? panel.setupDismissed() ? panel.t().firstRun.openWizard : panel.t().firstRun.resume
                  : panel.t().now.testVoice}</a>
            </div>
            <AssistantForm t={panel.t()} language={panel.language()} snapshot={state()}
              pending={pending()} error={panel.error()} save={panel.saveSettings} focusStage={focusStage()} />
            <SystemPromptForm t={panel.t()} systemPrompt={state().system_prompt}
              pending={pending()} save={panel.saveSystemPrompt} />
            </>}</Show>
          }>
            <Show when={active()?.id === "assistant"}>
              <BuiltinProviderForm />
              <Show when={panel.snapshot()}>{state =>
                <SystemPromptForm t={panel.t()} systemPrompt={state().system_prompt}
                  pending={pending()} save={panel.saveSystemPrompt} />
              }</Show>
              <Show when={panel.pluginPackages() && panel.snapshot()}>{state =>
                <Show when={state().plugins.some(plugin => plugin.providers?.length > 0)
                  || Object.values(state().settings.assistant.pipeline).some(stage =>
                    typeof stage === "object" && stage.provider !== "builtin")}>
                  <AssistantForm builtin t={panel.t()} language={panel.language()} snapshot={state()}
                    pending={pending()} error={panel.error()} save={panel.saveAssistant} focusStage={focusStage()} />
                </Show>
              }</Show>
            </Show>
          </Show>}
          {section.id === "assistant" && <Show when={assistantVisited() && panel.snapshot()?.settings.languages}>
            <AssistantLanguagesForm t={panel.t()} language={panel.language()} settings={panel.snapshot()!.settings}
              pending={pending() || (panel.builtinProvider() && !!panel.snapshot()?.active && panel.snapshot()?.active?.stage !== "awaiting_speech")}
              save={panel.saveSettings} />
          </Show>}
          {section.id === "network" && <Show when={active()?.id === "network"}><WifiForm /></Show>}
          {section.id === "diagnostics" && <Show when={active()?.id === "diagnostics"}>
            <Show when={panel.snapshot()}>{state => <Show when={panel.builtinProvider()} fallback={
                <VoiceCheck t={panel.t()} language={panel.language()} snapshot={state()} reachable={panel.reachable()}
                  pending={panel.pending()} test={panel.testStation} />
              }>
                <EmbeddedVoiceDiagnostics t={panel.t()} language={panel.language()} snapshot={state()}
                  pending={pending()} test={panel.testStation} />
              </Show>}</Show>
            <Show when={!panel.builtinProvider() && panel.assistantNeedsSetup() && !panel.setupDismissed()}>
              <a class={styles.inlineLink} href={setupPath}>{panel.t().firstRun.resume}</a>
            </Show>
            <SystemResources t={panel.t()} language={panel.language()} />
            <DiagnosticLogs t={panel.t()} passwordProtected={panel.access()?.protected === true}
              showKernel={!panel.builtinProvider()} />
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
              <Show when={active()?.id === "panel"}><PanelAccessControls /></Show>
            </section>
          </div>}
          {section.id === "update" && <Show when={active()?.id === "update"}>
            <Show when={panel.builtinProvider()} fallback={
              <CoreUpdate t={panel.t()} passwordProtected={panel.access()?.protected === true} />
            }>
              <Show when={panel.access()?.protected} fallback={<p>{panel.t().errors.core_update_password_required}</p>}>
                <FirmwareUpdate />
              </Show>
            </Show>
          </Show>}
        </section>
      }</For>
      <Show when={active() && active()?.id !== "panel" && panel.error()}><p class={styles.error} role="alert">{errorLabel(panel.t(), panel.error())}</p></Show>
    </div>
  </>;
}
