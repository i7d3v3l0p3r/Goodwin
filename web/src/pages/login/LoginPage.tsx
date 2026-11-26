import { useNavigate } from "@solidjs/router";
import { createEffect, Show } from "solid-js";
import { Brand } from "../../components/Brand";
import { PanelPasswordForm } from "../../components/PanelPasswordForm";
import { api, type WifiStatus } from "../../helpers/api";
import { usePanel } from "../../helpers/panel";
import { type Language } from "../../helpers/words";
import { assistantPath, setupPath, stationPath } from "../../helpers/readiness";
import { settingsSectionPath } from "../settings/sections";
import styles from "./LoginPage.module.css";

export function LoginPage() {
  const panel = usePanel();
  const navigate = useNavigate();
  const setup = () => panel.access()?.setup_required === true;
  const title = () => setup() ? panel.t().setupTitle : panel.t().login;
  createEffect(title, text => { document.title = `${text} · Goodwin`; });

  async function enter(settingUp: boolean) {
    if (panel.builtinProvider()) {
      const wifi = await api<WifiStatus>("network").catch(() => undefined);
      if (wifi?.status === "setup") void navigate(settingsSectionPath("network"), { replace: true });
      else if (settingUp) void navigate(assistantPath, { replace: true });
    } else if (settingUp) {
      void navigate(panel.setupDismissed() ? stationPath : setupPath, { replace: true });
    }
  }

  async function skipPassword() {
    if (await panel.openAccess()) await enter(true);
  }

  return <main class={styles.entry}>
    <Brand class={styles.brand} />
    <label class={styles.language}>
      {panel.t().language}
      <select value={panel.language()} onChange={event => panel.changeLanguage(event.currentTarget.value as Language)}>
        <option value="en">English</option><option value="ru">Русский</option>
      </select>
    </label>
    <Show when={panel.access()?.protected} fallback={<p role="status">{panel.status()}</p>}>
      <h1>{title()}</h1>
      <p class={styles.note}>{setup() ? panel.t().setupAccount : panel.t().account}</p>
      <Show when={setup()} fallback={<PanelPasswordForm mode="login" onSuccess={() => enter(false)} />}>
        <PanelPasswordForm mode="setup" onSuccess={() => enter(true)} onSkip={skipPassword} />
      </Show>
    </Show>
  </main>;
}
