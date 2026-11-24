import { createSignal, For, Show } from "solid-js";
import { Icon, PluginNavigationIcon } from "../../components/icons";
import { PageHeader } from "../../components/PageHeader";
import { Button, ButtonLink } from "../../components/ui-kit/Button";
import { ConfirmDialog } from "../../components/ui-kit/ConfirmDialog";
import { EmptyState, InlineError, StatusIndicator } from "../../components/ui-kit/Feedback";
import type { Plugin } from "../../helpers/api";
import { usePanel } from "../../helpers/panel";
import { pluginPagePath } from "../../helpers/plugins";
import { setupPath } from "../../helpers/readiness";
import { settingsSectionPath } from "../settings/sections";
import { errorLabel } from "../../helpers/words";
import { PluginSettingsLink } from "./PluginSettingsLink";
import styles from "./PluginsPage.module.css";

function landingPage(plugin: Plugin) {
  if (!plugin.web || !plugin.revision) return undefined;
  const page = plugin.pages.find(page => page.path === "") ?? plugin.pages.find(page => page.navigation && page.path !== "settings");
  return page && pluginPagePath(plugin.id, page.path);
}

export function PluginsPage() {
  const panel = usePanel();
  const [removing, setRemoving] = createSignal<{ id: string; title: string }>();
  const [deleting, setDeleting] = createSignal(false);
  const words = () => panel.t().plugins;
  const installed = () => panel.snapshot()?.plugins;
  const allowed = () => panel.access()?.protected;
  const pending = () => panel.pending() || !panel.reachable();
  let archive!: HTMLInputElement;
  let uploadButton: HTMLButtonElement | undefined;
  let removeTrigger: HTMLElement | undefined;

  async function remove() {
    const target = removing();
    if (!target || !allowed() || pending() || deleting()) return;
    setDeleting(true);
    try {
      if (await panel.removePlugin(target.id)) setRemoving(undefined);
    } finally {
      setDeleting(false);
    }
  }

  return <>
    <PageHeader title={panel.t().extensions} />
    <section class="gw-page" aria-label={panel.t().extensions}>
      <Show when={allowed()} fallback={<p class={styles.note}>{words().password} <a href={settingsSectionPath("panel")}>{panel.t().enablePassword}</a></p>}>
        <div class={styles.upload}>
          <div>
            <Button ref={node => { uploadButton = node; }} type="button" variant="primary" disabled={pending()}
              onClick={() => archive.click()}>{words().upload}</Button>
            <input ref={archive} type="file" hidden accept=".tar.gz,.tgz,application/gzip" onChange={event => {
              const file = event.currentTarget.files?.[0];
              event.currentTarget.value = "";
              if (file) void panel.uploadPlugin(file);
            }} />
          </div>
          <p class={styles.note}>{words().archive}</p>
        </div>
      </Show>
      <Show when={panel.stageIssue("chat")?.kind === "install"}>
        <p class={styles.note}><a href={setupPath}>{panel.t().firstRun.install}</a></p>
      </Show>
      <Show when={!removing() && panel.error()}><InlineError>{errorLabel(panel.t(), panel.error())}</InlineError></Show>
      <div role="status">
        <Show when={!installed()}><StatusIndicator tone="pending">{words().listLoading}</StatusIndicator></Show>
        <Show when={installed()?.length === 0}><EmptyState>{words().empty}</EmptyState></Show>
      </div>
      <Show when={installed()?.length}>
        <ul class={styles.list}>
          <For each={installed()} keyed={plugin => plugin.id}>
            {plugin => <li class={styles.item}>
              <div class={styles.details}>
                <h2>{plugin().title[panel.language()]}</h2>
                <div class={styles.metadata}>
                  <span class={styles.note}>{plugin().version}</span>
                  <StatusIndicator tone={plugin().status === "ready" ? "success" : plugin().status === "error" ? "error" : "pending"}>
                    {words().status[plugin().status]}
                  </StatusIndicator>
                </div>
                <Show when={plugin().error}>
                  {code => <InlineError announce={false}>{errorLabel(panel.t(), code())}</InlineError>}
                </Show>
                <Show when={plugin().web && plugin().revision}>
                  <div class="gw-row">
                    <For each={plugin().pages.filter(page => page.navigation && pluginPagePath(plugin().id, page.path) !== landingPage(plugin()))}>{page =>
                      <a href={pluginPagePath(plugin().id, page.path)}>
                        <PluginNavigationIcon name={page.icon} /> {page.title[panel.language()]}
                      </a>
                    }</For>
                  </div>
                </Show>
              </div>
              <div class="gw-row">
                <Show when={landingPage(plugin())}>{href =>
                  <ButtonLink href={href()} aria-label={`${words().open} · ${plugin().title[panel.language()]}`}>
                    {words().open}
                  </ButtonLink>
                }</Show>
                <Show when={plugin().has_settings || (plugin().web && plugin().pages.some(page => page.path === "settings"))}>
                  <PluginSettingsLink plugin={plugin()} quiet={Boolean(landingPage(plugin()))} />
                </Show>
                <Show when={allowed()}>
                  <details class={styles.rowMenu} name="plugin-actions" onFocusOut={event => {
                    if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) {
                      event.currentTarget.open = false;
                    }
                  }} onKeyDown={event => {
                    if (event.key !== "Escape" || !event.currentTarget.open) return;
                    event.preventDefault();
                    event.currentTarget.open = false;
                    event.currentTarget.querySelector("summary")?.focus();
                  }}>
                    <summary class={["gw-button", styles.more]} data-variant="quiet"
                      aria-label={words().actionsFor(plugin().title[panel.language()])}><Icon name="more" /></summary>
                    <div class={styles.menuItems}>
                      <Button type="button" variant="quiet" class={styles.remove} disabled={pending()} onClick={event => {
                        const target = plugin();
                        const menu = event.currentTarget.closest("details");
                        removeTrigger = menu?.querySelector("summary") ?? undefined;
                        if (menu) menu.open = false;
                        panel.clearError();
                        setRemoving({ id: target.id, title: target.title[panel.language()] });
                      }}>{words().remove}</Button>
                    </div>
                  </details>
                </Show>
              </div>
            </li>}
          </For>
        </ul>
      </Show>
    </section>
    <ConfirmDialog open={Boolean(removing())} title={words().removeTitle}
      description={words().confirm(removing()?.title ?? "")}
      cancel={words().cancel} confirm={deleting() ? words().removing : words().remove} disabled={pending()} busy={deleting()}
      error={removing() && panel.error() ? errorLabel(panel.t(), panel.error()) : undefined}
      onConfirm={() => void remove()} onCancel={() => {
        setRemoving(undefined);
        panel.clearError();
      }}
      onClose={() => (removeTrigger?.isConnected ? removeTrigger : uploadButton)?.focus()} />
  </>;
}
