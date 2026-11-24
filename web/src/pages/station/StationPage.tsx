import { For, Show } from "solid-js";
import { Icon } from "../../components/icons";
import { PageHeader } from "../../components/PageHeader";
import { ButtonLink } from "../../components/ui-kit/Button";
import { usePanel } from "../../helpers/panel";
import { setupPath } from "../../helpers/readiness";
import { settingsSectionPath, settingsSections } from "../settings/sections";
import styles from "./StationPage.module.css";

const stationSections = settingsSections.filter(section => section.id !== "assistant" && section.id !== "panel");

export function StationPage() {
  const panel = usePanel();
  const showSetup = () => panel.assistantNeedsSetup() && !panel.setupDismissed();
  return <>
    <PageHeader title={panel.t().station} />
    <div class={["gw-page", styles.page]}>
      <Show when={panel.snapshot()} fallback={<p role="status">{panel.status()}</p>}>{state =>
        <section class={styles.summary}>
          <h2>{panel.t().activation}</h2>
          <p>{panel.t().currentWakeModel} · {state().wake_model?.wake_word ?? (state().model ? panel.t().localWakeModel : panel.t().noWakeModel)}</p>
          <div class="gw-row">
            <ButtonLink variant="primary" href={showSetup() ? setupPath : settingsSectionPath("diagnostics")}>
              {showSetup() ? panel.t().firstRun.resume : panel.t().firstRun.checkWork}
            </ButtonLink>
          </div>
        </section>
      }</Show>
      <nav class={styles.destinations} aria-label={panel.t().station}>
        <For each={stationSections}>{section =>
          <a href={settingsSectionPath(section.id)}>
            <Icon name={section.icon} />
            <span><strong>{panel.t()[section.title]}</strong><small>{panel.t()[section.description]}</small></span>
            <Icon name="chevronRight" size={16} />
          </a>
        }</For>
      </nav>
    </div>
  </>;
}
