import { useLocation } from "@solidjs/router";
import { createEffect, createMemo, For, Show, type ParentProps } from "solid-js";
import { createPanel, PanelContext } from "../helpers/panel";
import { LoginPage } from "../pages/login/LoginPage";
import { chatPath, pluginsPath, settingsPath } from "../router";
import { pluginPagePath } from "../helpers/plugins";
import { Brand } from "./Brand";
import { Button } from "./Button";
import { Icon } from "./icons";
import styles from "./PanelLayout.module.css";

export function PanelLayout(props: ParentProps) {
  const panel = createPanel();
  const route = useLocation();
  const plugins = createMemo(() => panel.snapshot()?.plugins);
  const destinations = createMemo(() => plugins()?.flatMap(plugin => plugin.pages.filter(page => page.navigation).map(page => ({ plugin, page }))));
  const current = (path: string) => (route.pathname.replace(/\/$/, "") || "/") === path ? "" : undefined;
  createEffect(() => route.pathname, panel.clearError);

  return <PanelContext value={panel}>
    <Show when={panel.access()?.authorized} fallback={<LoginPage />}>
      <div class={styles.shell}>
        <aside class={styles.sidebar}>
          <Brand class={styles.brand} href={chatPath} />
          <nav class={styles.navigation} aria-label={panel.t().nav}>
            <a href={chatPath} data-current={current(chatPath)}><Icon name="chat" /><span>{panel.t().chat}</span></a>
            <For each={destinations()}>
              {({ plugin, page }) => <a href={pluginPagePath(plugin.id, page.path)} data-current={current(pluginPagePath(plugin.id, page.path))}><Icon name="plugin" /><span>{page.title[panel.language()]}</span></a>}
            </For>
            <a href={settingsPath} data-current={current(settingsPath)}><Icon name="settings" /><span>{panel.t().settings}</span></a>
            <a href={pluginsPath} data-current={current(pluginsPath)}><Icon name="plugin" /><span>{panel.t().plugins.title}</span></a>
          </nav>
          <Show when={panel.access()?.protected}>
            <Button class={styles.logout} type="button" disabled={panel.pending()} onClick={() => void panel.logout()}>
              {panel.t().logout}
            </Button>
          </Show>
        </aside>
        <main class={styles.main}>{props.children}</main>
      </div>
    </Show>
  </PanelContext>;
}
