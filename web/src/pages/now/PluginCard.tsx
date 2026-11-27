import { useNavigate } from "@solidjs/router";
import { createEffect, createMemo, createSignal, Show, useContext } from "solid-js";
import { PluginNavigationIcon } from "../../components/icons";
import type { Plugin } from "../../helpers/api";
import { DraftProtectionContext } from "../../helpers/drafts";
import { usePanel } from "../../helpers/panel";
import { loadPlugin, pluginPagePath, pluginRequest } from "../../helpers/plugins";
import styles from "./NowPage.module.css";

export function PluginCard(props: { plugin: Plugin; activity: (active: boolean) => void }) {
  const panel = usePanel();
  const navigate = useNavigate();
  const drafts = useContext(DraftProtectionContext);
  const [failed, setFailed] = createSignal(false);
  const [container, setContainer] = createSignal<HTMLDivElement>();
  const page = () => props.plugin.pages.find(p => p.path === "") ?? props.plugin.pages[0];
  const title = () => props.plugin.title[panel.language()];
  const moduleKey = createMemo(() => [props.plugin.id, props.plugin.revision, panel.language(), page()?.id].join("/"));

  createEffect(() => [moduleKey(), container()] as const, ([, element]) => {
    if (!element) return;
    setFailed(false);
    const plugin = props.plugin;
    const unmount = loadPlugin(plugin, element, {
      pageId: page()?.id ?? "", language: panel.language(),
      request: (path, method, body) => pluginRequest(plugin.id, path, method, body),
      navigate: path => { if (plugin.pages.some(p => p.path === path)) void navigate(pluginPagePath(plugin.id, path)); },
      protectDraft: dirty => drafts.register({ dirty, keptAt: path => path === "/" }),
      setNowActive: props.activity,
    }, () => setFailed(true), "now");
    return () => { unmount(); props.activity(false); };
  });
  return <section class={styles.plugin} data-size={props.plugin.web?.now} aria-label={title()}>
    <h2><Show when={page()} fallback={<span>{title()}</span>}>{p =>
      <a href={pluginPagePath(props.plugin.id, p().path)}><PluginNavigationIcon name={p().icon} />{title()}<small aria-hidden="true">↗</small></a>
    }</Show></h2>
    <div ref={setContainer} />
    <Show when={failed()}><p role="status">{panel.t().plugins.loadError}</p></Show>
  </section>;
}
