import { Show } from "solid-js";
import { PageHeader } from "../../components/PageHeader";
import { usePanel } from "../../helpers/panel";
import { errorLabel } from "../../helpers/words";
import { OpenRouterForm } from "./components/OpenRouterForm";
import { Preferences } from "./components/Preferences";
import { VoiceCheck } from "./components/VoiceCheck";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  const panel = usePanel();
  const pending = () => panel.pending() || !panel.reachable();

  return <>
    <PageHeader title={panel.t().settings} />
    <Show when={panel.snapshot()}>
      {(state) => <section class={styles.settings} aria-label={panel.t().settings}>
        <Preferences t={panel.t()} language={panel.language()} listening={state().settings.listening}
          pending={pending()} changeLanguage={panel.changeLanguage} toggleListening={() => {
            const settings = state().settings;
            void panel.saveSettings({ ...settings, listening: !settings.listening });
          }} />
        <VoiceCheck t={panel.t()} snapshot={state()} disabled={panel.busy() || pending()} test={panel.testStation} />
        <OpenRouterForm t={panel.t()} snapshot={state()} pending={pending()} error={panel.error()} save={panel.saveSettings} />
        <Show when={panel.error()}><p class={styles.error} role="alert">{errorLabel(panel.t(), panel.error())}</p></Show>
      </section>}
    </Show>
  </>;
}
