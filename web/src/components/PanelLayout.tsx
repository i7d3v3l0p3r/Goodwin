import { useLocation } from "@solidjs/router";
import { createEffect, createSignal, Show, type ParentProps } from "solid-js";
import { createDraftProtection, DraftProtectionContext } from "../helpers/drafts";
import { createPanel, PanelContext, usePanel } from "../helpers/panel";
import { pluginPagePath } from "../helpers/plugins";
import { LoginPage } from "../pages/login/LoginPage";
import { chatPath, pluginsPath } from "../router";
import { settingsPath, settingsSectionPath } from "../pages/settings/sections";
import { assistantPath, setupPath, stationPath } from "../helpers/readiness";
import { Brand } from "./Brand";
import { Button } from "./ui-kit/Button";
import { ConfirmDialog } from "./ui-kit/ConfirmDialog";
import { Icon } from "./icons";
import styles from "./PanelLayout.module.css";

const timersPath = pluginPagePath("timers", "");

export function PanelLayout(props: ParentProps) {
  const panel = createPanel();
  return <PanelContext value={panel}>
    <Show when={panel.access()?.authorized} fallback={<LoginPage />}>
      <PanelShell>{props.children}</PanelShell>
    </Show>
  </PanelContext>;
}

function PanelShell(props: ParentProps) {
  const panel = usePanel();
  const route = useLocation();
  const drafts = createDraftProtection();
  const [menuOpen, setMenuOpen] = createSignal(false);
  const pathname = () => route.pathname.replace(/\/$/, "") || "/";
  const current = (path: string) => pathname() === path ? "page" : undefined;
  const timersPage = () => {
    const plugin = panel.snapshot()?.plugins.find(plugin => plugin.id === "timers");
    return plugin?.web && plugin.revision ? plugin.pages.find(page => page.path === "" && page.navigation) : undefined;
  };
  const stationCurrent = () => current(stationPath) ?? (
    pathname() === setupPath || pathname().startsWith(`${settingsPath}/`)
      && pathname() !== assistantPath && pathname() !== settingsSectionPath("panel") ? "location" : undefined
  );
  const extensionsCurrent = () => current(pluginsPath) ?? (
    pathname().startsWith(`${pluginsPath}/`) && !(timersPage() && current(timersPath)) ? "location" : undefined
  );
  let menuContainer!: HTMLDivElement;
  let menuTrigger!: HTMLButtonElement;
  let main!: HTMLElement;

  function closeMenu() {
    setMenuOpen(false);
  }

  function dismissMenu() {
    closeMenu();
    menuTrigger.focus();
  }

  function signOut() {
    dismissMenu();
    drafts.beforeSignOut(() => void panel.logout());
  }

  createEffect(() => route.pathname, () => {
    panel.clearError();
    closeMenu();
  });
  createEffect(menuOpen, open => {
    if (!open) return;
    function closeFromOutside(event: Event) {
      if (event.target instanceof Node && !menuContainer.contains(event.target)) closeMenu();
    }
    function escape(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      dismissMenu();
    }
    document.addEventListener("pointerdown", closeFromOutside);
    document.addEventListener("focusin", closeFromOutside);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", closeFromOutside);
      document.removeEventListener("focusin", closeFromOutside);
      document.removeEventListener("keydown", escape);
    };
  });

  return <DraftProtectionContext value={drafts}>
    <div class={styles.shell}>
      <a class={styles.skip} href="#panel-content" onClick={event => {
        event.preventDefault();
        main.focus({ preventScroll: true });
      }}>{panel.t().skipNavigation}</a>
      <aside class={styles.sidebar}>
        <div class={styles.brandBar}>
          <Brand class={styles.brand} href={chatPath} />
          <div class={styles.options} ref={menuContainer}>
            <Button ref={menuTrigger} class={styles.menuButton} type="button" aria-label={panel.t().panelMenu}
              aria-expanded={menuOpen() ? "true" : "false"} aria-controls="panel-menu" onClick={() => setMenuOpen(open => !open)}>
              <Icon name="settings" />
            </Button>
            <div id="panel-menu" class={styles.menu} hidden={!menuOpen()}>
              <a class={styles.menuItem} href={settingsSectionPath("panel")} onClick={closeMenu}>{panel.t().panelPreferencesDescription}</a>
              <Show when={panel.access()?.protected}>
                <Button class={styles.menuItem} variant="quiet" type="button" disabled={panel.pending()} onClick={signOut}>{panel.t().logout}</Button>
              </Show>
            </div>
          </div>
        </div>
        <nav class={styles.navigation} aria-label={panel.t().nav}>
          <a href={chatPath} aria-current={current(chatPath)}><Icon name="chat" /><span>{panel.t().chat}</span></a>
          <Show when={timersPage()}>{page =>
            <a href={timersPath} aria-current={current(timersPath)}><Icon name="timer" /><span>{page().title[panel.language()]}</span></a>
          }</Show>
          <Show when={!panel.builtinProvider()} fallback={
            <>
              <a href={assistantPath} aria-current={current(assistantPath)}><Icon name="sparkles" /><span>{panel.t().embedded.provider}</span></a>
              <a href={settingsSectionPath("network")} aria-current={current(settingsSectionPath("network"))}>
                <Icon name="monitor" /><span>{panel.t().wifi}</span>
              </a>
              <a href={settingsSectionPath("activation")} aria-current={current(settingsSectionPath("activation"))}>
                <Icon name="mic" /><span>{panel.t().activation}</span>
              </a>
              <a href={settingsSectionPath("diagnostics")} aria-current={current(settingsSectionPath("diagnostics"))}>
                <Icon name="activity" /><span>{panel.t().diagnostics}</span>
              </a>
            </>
          }>
            <a href={stationPath} aria-current={stationCurrent()}><Icon name="activity" /><span>{panel.t().station}</span></a>
            <a href={assistantPath} aria-current={current(assistantPath)}><Icon name="sparkles" /><span>{panel.t().assistant}</span></a>
          </Show>
          <Show when={panel.pluginPackages()}>
            <a href={pluginsPath} aria-current={extensionsCurrent()}><Icon name="plugin" /><span>{panel.t().extensions}</span></a>
          </Show>
        </nav>
      </aside>
      <main ref={main} id="panel-content" class={styles.main} tabindex={-1}>{props.children}</main>
      <ConfirmDialog open={Boolean(drafts.confirmation())} title={panel.t().unsaved} description={panel.t().discardDraftDescription}
        cancel={panel.t().cancel} confirm={panel.t().discardDraft} onCancel={drafts.cancel} onConfirm={drafts.proceed} />
    </div>
  </DraftProtectionContext>;
}
