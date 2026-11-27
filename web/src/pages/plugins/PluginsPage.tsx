import { useSearchParams } from "@solidjs/router";
import { createMemo, createSignal, For, Show } from "solid-js";
import { Icon, PluginNavigationIcon } from "../../components/icons";
import { PageHeader } from "../../components/PageHeader";
import { Button } from "../../components/ui-kit/Button";
import { ConfirmDialog } from "../../components/ui-kit/ConfirmDialog";
import { EmptyState, InlineError, StatusIndicator } from "../../components/ui-kit/Feedback";
import { api, ApiError, type Plugin } from "../../helpers/api";
import { usePanel } from "../../helpers/panel";
import { pluginLandingPage, pluginPagePath, pluginSettingsPath } from "../../helpers/plugins";
import { setupPath } from "../../helpers/readiness";
import { settingsSectionPath } from "../settings/sections";
import { errorLabel } from "../../helpers/words";
import styles from "./PluginsPage.module.css";

function landingPage(plugin: Plugin) {
  const page = pluginLandingPage(plugin);
  return page && pluginPagePath(plugin.id, page.path);
}

function catalogIcon(plugin: Plugin) {
  return plugin.pages.find(page => page.navigation && page.icon)?.icon ?? (plugin.providers?.length ? "sparkles" : "plug");
}

export function PluginsPage() {
  const panel = usePanel();
  const [removing, setRemoving] = createSignal<{ id: string; title: string }>();
  const [deleting, setDeleting] = createSignal(false);
  const [routing, setRouting] = createSignal("");
  const [routeError, setRouteError] = createSignal<{ id: string; code: string }>();
  const words = () => panel.t().plugins;
  const installed = () => panel.snapshot()?.plugins;
  const [search, setSearch] = useSearchParams();
  const query = () => typeof search.q === "string" ? search.q : "";
  const matches = createMemo(() => {
    const plugins = installed() ?? [];
    const term = query().trim().toLowerCase();
    if (!term) return plugins;
    const language = panel.language();
    return plugins.filter(plugin => plugin.title[language].toLowerCase().includes(term) || plugin.id.includes(term)
      || plugin.pages.some(page => page.navigation && page.title[language].toLowerCase().includes(term)));
  });
  const allowed = () => panel.access()?.protected;
  const pending = () => panel.pending() || !panel.reachable();
  let archive!: HTMLInputElement;
  let uploadButton: HTMLButtonElement | undefined;
  let removeTrigger: HTMLElement | undefined;

  async function route(plugin: Plugin, selected: boolean) {
    if (routing() || pending() || plugin.vpn === selected) return;
    setRouting(plugin.id);
    setRouteError(undefined);
    try {
      await api(`plugin-vpn/${encodeURIComponent(plugin.id)}`, "PUT", selected);
    } catch (error) {
      setRouteError({ id: plugin.id, code: error instanceof ApiError ? error.code : "internal" });
    } finally {
      setRouting("");
    }
  }

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
    <PageHeader title={words().title} />
    <section class={["gw-page", styles.catalog]} aria-label={words().title}>
      <Show when={allowed()} fallback={<p class={styles.note}>{words().password} <a href={settingsSectionPath("panel")}>{panel.t().enablePassword}</a></p>}>
        <div class={styles.upload}>
          <div>
            <Button ref={node => { uploadButton = node; }} type="button" variant="primary" disabled={pending()}
              onClick={() => archive.click()}><Icon name="plus" size={17} />{words().upload}</Button>
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
      <Show when={(installed()?.length ?? 0) >= 6 || query()}>
        <label class={styles.search}>
          <Icon name="search" size={18} />
          <input type="search" aria-label={words().search} placeholder={words().search}
            value={query()} onInput={event => setSearch({ q: event.currentTarget.value }, { replace: true })} />
        </label>
      </Show>
      <div role="status">
        <Show when={!installed()}><StatusIndicator tone="pending">{words().listLoading}</StatusIndicator></Show>
        <Show when={installed()?.length === 0}><EmptyState>{words().empty}</EmptyState></Show>
        <Show when={installed()?.length && !matches().length}><EmptyState>{words().noMatches}</EmptyState></Show>
      </div>
      <Show when={matches().length}>
        <ul class={styles.list}>
          <For each={matches()} keyed={plugin => plugin.id}>
            {plugin => {
              const title = () => plugin().title[panel.language()];
              const landing = () => landingPage(plugin());
              const settings = () => plugin().has_settings || (plugin().web && plugin().pages.some(page => page.path === "settings"))
                ? pluginSettingsPath(plugin().id) : undefined;
              const pinned = () => panel.pinnedPlugins().includes(plugin().id);
              const extraPages = () => plugin().web && plugin().revision
                ? plugin().pages.filter(page => page.navigation && ![landing(), settings()].includes(pluginPagePath(plugin().id, page.path))) : [];
              return <li class={styles.item} data-status={plugin().status}>
                <div class={styles.identity}>
                  <span class={styles.tile} data-icon={catalogIcon(plugin())} aria-hidden="true">
                    <PluginNavigationIcon name={catalogIcon(plugin())} />
                  </span>
                  <div class={styles.details}>
                    <h2><Show when={landing() || settings()} fallback={title()}>
                      {href => <a href={href()} aria-label={`${landing() ? words().open : words().configure} · ${title()}`}>
                        {title()}
                      </a>}
                    </Show></h2>
                    <div class={styles.metadata}>
                      <span>{plugin().version}</span>
                      <StatusIndicator tone={plugin().status === "ready" ? "success" : plugin().status === "error" ? "error" : "pending"}>
                        {words().status[plugin().status]}
                      </StatusIndicator>
                    </div>
                  </div>
                </div>
                <div class={styles.actions}>
                  <Show when={plugin().vpn !== undefined && (plugin().vpn || installed()?.some(entry => entry.id === "vpn"))}>
                    <button type="button" class={styles.vpn} role="switch" aria-checked={plugin().vpn ? "true" : "false"}
                      aria-label={`${words().vpnRouting} · ${title()}`} title={plugin().vpn ? words().vpnOff : words().vpnOn}
                      aria-busy={routing() === plugin().id ? "true" : "false"} disabled={Boolean(routing()) || pending()}
                      onClick={() => void route(plugin(), !plugin().vpn)}>
                      VPN<span aria-hidden="true" class={styles.track} />
                    </button>
                  </Show>
                  <Show when={landing() && settings()}>{href =>
                    <a href={href()} title={words().configure} aria-label={`${words().configure} · ${title()}`}>
                      <Icon name="settings" size={16} />
                    </a>
                  }</Show>
                  <Show when={landing()}>
                    <button type="button" class={styles.bookmark} aria-pressed={pinned() ? "true" : "false"}
                      aria-label={`${words().inMenu} · ${title()}`} title={pinned() ? panel.t().now.unpin : panel.t().now.pin}
                      onClick={() => panel.togglePluginPin(plugin().id)}>
                      <Icon name="bookmark" size={16} />
                    </button>
                  </Show>
                  <Show when={allowed()}>
                    <button type="button" class={styles.remove} disabled={pending()}
                      title={words().remove} aria-label={`${words().remove} · ${title()}`} onClick={event => {
                        removeTrigger = event.currentTarget;
                        panel.clearError();
                        setRemoving({ id: plugin().id, title: title() });
                      }}><Icon name="trash" size={16} /></button>
                  </Show>
                </div>
                <Show when={extraPages().length}>
                  <div class={styles.pages}><For each={extraPages()}>{page =>
                    <a href={pluginPagePath(plugin().id, page.path)}>{page.title[panel.language()]}</a>
                  }</For></div>
                </Show>
                <Show when={plugin().error}>
                  {code => <InlineError announce={false}>{errorLabel(panel.t(), code())}</InlineError>}
                </Show>
                <Show when={routeError()?.id === plugin().id && routeError()?.code}>
                  {code => <InlineError>{errorLabel(panel.t(), code())}</InlineError>}
                </Show>
              </li>;
            }}
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
