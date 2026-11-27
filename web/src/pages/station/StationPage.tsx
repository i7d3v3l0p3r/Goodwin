import { For, Show } from "solid-js";
import { Icon } from "../../components/icons";
import { PageHeader } from "../../components/PageHeader";
import { ButtonLink } from "../../components/ui-kit/Button";
import { usePanel } from "../../helpers/panel";
import { setupPath } from "../../helpers/readiness";
import { settingsSectionPath, settingsSections } from "../settings/sections";
import styles from "./StationPage.module.css";

export function StationPage() {
  const panel = usePanel();
  const showSetup = () => panel.assistantNeedsSetup() && !panel.setupDismissed();
  const destinations = () => settingsSections.filter(section => section.id !== "assistant" && section.id !== "panel"
    && (section.id !== "network" || panel.builtinProvider()));
  return <>
    <PageHeader title={panel.t().station} />
    <div class={["gw-page", styles.page]}>
      <Show when={panel.snapshot()} fallback={<p role="status">{panel.status()}</p>}>{state =>
        <section class={styles.summary}>
          <div class={styles.instrument}>
            <h2>{state().wake_model?.wake_word ? panel.t().wakeWord : panel.t().currentWakeModel}</h2>
            <p>{state().wake_model?.wake_word ?? (state().model ? panel.t().localWakeModel : panel.t().noWakeModel)}</p>
            <ButtonLink variant="primary" href={showSetup() ? setupPath : settingsSectionPath("diagnostics")}>
              {showSetup() ? panel.t().firstRun.resume : panel.t().diagnostics}
            </ButtonLink>
          </div>
          <div class={styles.illustration} data-listening={panel.wakeReady() ? "true" : undefined} aria-hidden="true"><i class={styles.speaker} /></div>
        </section>
      }</Show>
      <nav class={styles.destinations} aria-label={panel.t().station}>
        <For each={destinations()}>{section =>
          <a href={settingsSectionPath(section.id)} data-section={section.id}>
            <span class={styles.symbol}><Icon name={section.icon} /></span>
            <span><strong>{panel.t()[section.title]}</strong><small>{panel.t()[section.description]}</small></span>
            <Icon name="chevronRight" size={16} />
          </a>
        }</For>
      </nav>
    </div>
  </>;
}
