import { useLocation } from "@solidjs/router";
import { createEffect, createMemo, createSignal, For, onCleanup, Show, type ParentProps } from "solid-js";
import { createDraftProtection, DraftProtectionContext } from "../helpers/drafts";
import { createPanel, PanelContext, usePanel } from "../helpers/panel";
import { LoginPage } from "../pages/login/LoginPage";
import { chatPath, pluginsPath } from "../router";
import { settingsPath } from "../pages/settings/sections";
import { pluginPagePath } from "../helpers/plugins";
import { Brand } from "./Brand";
import { Button } from "./ui-kit/Button";
import { ConfirmDialog } from "./ui-kit/ConfirmDialog";
import { Icon, PluginNavigationIcon } from "./icons";
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
  const [menuOpen, setMenuOpen] = createSignal(false);
  const destinations = createMemo(() => panel.snapshot()?.plugins.flatMap(plugin => plugin.pages
    .filter(page => page.navigation)
    .map(page => ({ path: pluginPagePath(plugin.id, page.path), title: page.title, icon: page.icon }))));
  const current = (path: string) => (route.pathname.replace(/\/$/, "") || "/") === path ? "page" : undefined;
  const settingsCurrent = () => current(settingsPath) ?? (route.pathname.startsWith(`${settingsPath}/`) ? "location" : undefined);
  let menu!: HTMLDialogElement;
  let menuTrigger!: HTMLButtonElement;
  let main!: HTMLElement;

  function closeMenu() {
    if (menu?.open) menu.close();
    setMenuOpen(false);
  }

  function dismissMenu() {
    closeMenu();
    menuTrigger.focus();
  }

  function signOut() {
    closeMenu();
    drafts.beforeSignOut(() => void panel.logout());
  }

  createEffect(() => route.pathname, () => {
    panel.clearError();
    closeMenu();
  });

  const mobile = window.matchMedia("(max-width: 650px)");
  const closeOnDesktop = (event: MediaQueryListEvent) => { if (!event.matches) closeMenu(); };
  mobile.addEventListener("change", closeOnDesktop);
  onCleanup(() => mobile.removeEventListener("change", closeOnDesktop));

  function Navigation(props: { onNavigate?: (event: MouseEvent) => void }) {
    return <nav class={styles.navigation} aria-label={panel.t().nav}>
      <a href={chatPath} aria-current={current(chatPath)} onClick={props.onNavigate}><Icon name="chat" /><span>{panel.t().chat}</span></a>
      <a href={settingsPath} aria-current={settingsCurrent()} onClick={props.onNavigate}><Icon name="settings" /><span>{panel.t().settings}</span></a>
      <a href={pluginsPath} aria-current={current(pluginsPath)} onClick={props.onNavigate}><Icon name="plugin" /><span>{panel.t().plugins.title}</span></a>
      <Show when={destinations()?.length}>
        <div class={styles.extensions}>
          <For each={destinations()}>
            {page => <a href={page.path} aria-current={current(page.path)} onClick={props.onNavigate}>
              <PluginNavigationIcon name={page.icon} /><span>{page.title[panel.language()]}</span>
            </a>}
          </For>
        </div>
      </Show>
    </nav>;
  }

  return <DraftProtectionContext value={drafts}>
    <div class={styles.shell}>
      <a class={styles.skip} href="#panel-content" onClick={event => {
        event.preventDefault();
        main.focus({ preventScroll: true });
      }}>{panel.t().skipNavigation}</a>
      <aside class={styles.sidebar}>
        <Brand class={styles.brand} href={chatPath} />
        <div class={styles.desktopNavigation}>
          <Navigation />
          <Show when={panel.access()?.protected}>
            <Button class={styles.logout} type="button" disabled={panel.pending()} onClick={signOut}>{panel.t().logout}</Button>
          </Show>
        </div>
        <Button ref={menuTrigger} class={styles.menuButton} type="button" aria-label={panel.t().menu} aria-haspopup="dialog"
          aria-expanded={menuOpen() ? "true" : "false"} aria-controls="panel-menu" onClick={() => {
            menu.showModal();
            setMenuOpen(true);
          }}>
          <Icon name="menu" />
        </Button>
      </aside>
      <main ref={main} id="panel-content" class={styles.main} tabindex={-1}>{props.children}</main>
      <dialog ref={menu} id="panel-menu" class={styles.menu} aria-label={panel.t().menu}
        onCancel={event => { event.preventDefault(); dismissMenu(); }} onClick={event => {
          if (event.target === event.currentTarget) dismissMenu();
        }}>
        <div class={styles.menuHeader}>
          <h2>{panel.t().menu}</h2>
          <Button class={styles.menuClose} type="button" aria-label={panel.t().close} autofocus onClick={dismissMenu}>
            <Icon name="close" />
          </Button>
        </div>
        <Navigation onNavigate={event => {
          if (!event.ctrlKey && !event.metaKey && !event.shiftKey && !event.altKey) closeMenu();
        }} />
        <Show when={panel.access()?.protected}>
          <Button class={styles.menuLogout} type="button" disabled={panel.pending()} onClick={signOut}>{panel.t().logout}</Button>
        </Show>
      </dialog>
      <ConfirmDialog open={Boolean(drafts.confirmation())} title={panel.t().unsaved} description={panel.t().discardDraftDescription}
        cancel={panel.t().cancel} confirm={panel.t().discardDraft} onCancel={drafts.cancel} onConfirm={drafts.proceed} />
    </div>
  </DraftProtectionContext>;
}
