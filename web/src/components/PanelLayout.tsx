import { useLocation } from "@solidjs/router";
import { createEffect, createMemo, createSignal, onSettled, Show, type ParentProps } from "solid-js";
import { createDraftProtection, DraftProtectionContext } from "../helpers/drafts";
import { createPanel, PanelContext, usePanel } from "../helpers/panel";
import { pluginLandingPage, pluginPagePath } from "../helpers/plugins";
import { LoginPage } from "../pages/login/LoginPage";
import { pluginsPath } from "../router";
import { settingsPath, settingsSectionPath } from "../pages/settings/sections";
import { assistantPath, setupPath, stationPath } from "../helpers/readiness";
import { Brand } from "./Brand";
import { Button } from "./ui-kit/Button";
import { ConfirmDialog } from "./ui-kit/ConfirmDialog";
import { Icon, pluginIcon } from "./icons";
import { MobileNavigation, NavigationLinks, type NavigationItem } from "./MobileNavigation";
import styles from "./PanelLayout.module.css";

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
  const compactLayout = window.matchMedia("(max-width: 760px), (pointer: coarse) and (max-width: 960px) and (max-height: 500px)");
  const [mobile, setMobile] = createSignal(compactLayout.matches);
  const pathname = () => route.pathname.replace(/\/$/, "") || "/";
  const items = createMemo(() => {
    const t = panel.t();
    const path = pathname();
    const links: NavigationItem[] = [];
    function link(href: string, title: string, icon: NavigationItem["icon"], primary = false, containsPage = false) {
      links.push({ href, title, icon, primary, current: path === href ? "page" : containsPage ? "location" : undefined });
    }

    link("/", t.now.title, "house", true);
    for (const id of panel.pinnedPlugins()) {
      const plugin = panel.snapshot()?.plugins.find(item => item.id === id);
      const page = plugin && pluginLandingPage(plugin);
      if (page) link(pluginPagePath(id, page.path), page.title[panel.language()], pluginIcon(page.icon));
    }
    const preferences = settingsSectionPath("panel");
    if (panel.builtinProvider()) {
      link(assistantPath, t.embedded.provider, "sparkles");
      link(settingsSectionPath("network"), t.wifi, "wifi", true);
      link(settingsSectionPath("activation"), t.activation, "mic");
      link(settingsSectionPath("diagnostics"), t.diagnostics, "activity");
      if (mobile()) link(settingsSectionPath("update"), t.coreUpdate, "download");
    } else {
      const stationSection = path === setupPath || path.startsWith(`${settingsPath}/`) && path !== assistantPath && path !== preferences;
      link(stationPath, t.station, "activity", true, stationSection);
      link(assistantPath, t.assistant, "sparkles");
    }
    if (panel.pluginPackages()) {
      const pluginSection = path.startsWith(`${pluginsPath}/`) && !links.some(item => item.href === path);
      link(pluginsPath, t.plugins.title, "plugin", false, pluginSection);
    }
    if (mobile()) link(preferences, t.panelPreferencesDescription, "settings");
    return links;
  });
  let main!: HTMLElement;
  const signOut = () => drafts.beforeSignOut(() => void panel.logout());

  onSettled(() => {
    const resize = () => setMobile(compactLayout.matches);
    compactLayout.addEventListener("change", resize);
    return () => compactLayout.removeEventListener("change", resize);
  });
  createEffect(() => route.pathname, () => panel.clearError());

  return <DraftProtectionContext value={drafts}>
    <div class={[styles.shell, mobile() && styles.compact]}>
      <a class={styles.skip} href="#panel-content" onClick={event => {
        event.preventDefault();
        main.focus({ preventScroll: true });
      }}>{panel.t().skipNavigation}</a>
      <Show when={!mobile()}>
        <aside class={styles.sidebar}>
          <div class={styles.brandBar}>
            <Brand class={styles.brand} href="/" />
            <PanelOptions onSignOut={signOut} />
          </div>
          <nav class={styles.navigation} aria-label={panel.t().nav}><NavigationLinks items={items()} /></nav>
        </aside>
      </Show>
      <main ref={main} id="panel-content" class={styles.main} tabindex={-1}>{props.children}</main>
      <Show when={mobile()}>
        <MobileNavigation items={items()} pathname={pathname()} onSignOut={signOut} />
      </Show>
      <ConfirmDialog open={Boolean(drafts.confirmation())} title={panel.t().unsaved} description={panel.t().discardDraftDescription}
        cancel={panel.t().cancel} confirm={panel.t().discardDraft} onCancel={drafts.cancel} onConfirm={drafts.proceed} />
    </div>
  </DraftProtectionContext>;
}

function PanelOptions(props: { onSignOut: () => void }) {
  const panel = usePanel();
  const route = useLocation();
  const [menuOpen, setMenuOpen] = createSignal(false);
  let menuContainer!: HTMLDivElement;
  let menuTrigger!: HTMLButtonElement;

  function closeMenu() {
    setMenuOpen(false);
  }

  function dismissMenu() {
    closeMenu();
    menuTrigger.focus();
  }

  createEffect(() => route.pathname, closeMenu);
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

  return <div class={styles.options} ref={menuContainer}>
    <Button ref={menuTrigger} class={styles.menuButton} type="button" aria-label={panel.t().panelMenu}
      aria-expanded={menuOpen() ? "true" : "false"} aria-controls="panel-menu" onClick={() => setMenuOpen(open => !open)}>
      <Icon name="settings" />
    </Button>
    <div id="panel-menu" class={styles.menu} hidden={!menuOpen()}>
      <a class={styles.menuItem} href={settingsSectionPath("panel")} onClick={closeMenu}>{panel.t().panelPreferencesDescription}</a>
      <Show when={panel.access()?.protected}>
        <Button class={styles.menuItem} variant="quiet" type="button" disabled={panel.pending()} onClick={() => {
          dismissMenu();
          props.onSignOut();
        }}>{panel.t().logout}</Button>
      </Show>
    </div>
  </div>;
}
