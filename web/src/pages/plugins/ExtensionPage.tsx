import { useLocation, useNavigate } from "@solidjs/router";
import { createEffect, createMemo, createSignal, Show } from "solid-js";
import { PageHeader } from "../../components/PageHeader";
import { usePanel } from "../../helpers/panel";
import { loadPlugin, pluginPagePath, pluginRequest } from "../../helpers/plugins";
import styles from "./ExtensionPage.module.css";

export function ExtensionPage() {
  const panel = usePanel();
  const location = useLocation();
  const navigate = useNavigate();
  const [error, setError] = createSignal(false);
  const [container, setContainer] = createSignal<HTMLElement>();
  const path = () => location.pathname.replace(/\/$/, "");
  const plugins = createMemo(() => panel.snapshot()?.plugins);
  const selection = createMemo(() => {
    const id = path().split("/")[2];
    const plugin = plugins()?.find(entry => entry.id === id);
    const page = plugin?.pages.find(page => pluginPagePath(id, page.path) === path());
    return plugin?.web && plugin.revision && page ? { plugin, page } : undefined;
  }, { equals: (previous, next) => previous?.plugin.revision === next?.plugin.revision && previous?.page.id === next?.page.id });

  createEffect(() => {
    const selected = selection();
    const target = container();
    return selected && target ? { ...selected, target, language: panel.language() } : undefined;
  }, mounted => {
    setError(false);
    if (!mounted) return;
    const { plugin, page, language, target } = mounted;
    return loadPlugin(plugin, target, {
      pageId: page.id,
      language,
      request: (path, method, body) => pluginRequest(plugin.id, path, method, body),
      navigate: path => {
        if (plugin.pages.some(page => page.path === path)) void navigate(pluginPagePath(plugin.id, path));
      },
    }, () => setError(true));
  });

  return <>
    <PageHeader title={selection()?.page.title[panel.language()] ?? panel.t().plugins.title} />
    <section class={styles.body}>
      <div ref={setContainer} hidden={!selection()} />
      <Show when={!selection()}><p>{panel.snapshot() ? panel.t().plugins.missing : panel.t().plugins.loading}</p></Show>
      <Show when={error()}><p class={styles.error} role="alert">{panel.t().plugins.loadError}</p></Show>
    </section>
  </>;
}
