import { useLocation } from "@solidjs/router";
import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { PageHeader } from "../../components/PageHeader";
import { Button } from "../../components/ui-kit/Button";
import { InlineError } from "../../components/ui-kit/Feedback";
import { Field } from "../../components/ui-kit/Field";
import { SaveActions } from "../../components/ui-kit/SaveActions";
import { api, ApiError, type Plugin, type ProviderConnection } from "../../helpers/api";
import { protectDraft } from "../../helpers/drafts";
import { assistantPath, capabilities, setupPath } from "../../helpers/readiness";
import { usePanel } from "../../helpers/panel";
import { pluginSettingsPath } from "../../helpers/plugins";
import { errorLabel } from "../../helpers/words";
import styles from "./PluginSettingsPage.module.css";

type Draft = Pick<ProviderConnection, "name" | "protocol" | "baseUrl"> & { key: string; clearKey: boolean };
const emptyDraft: Draft = { name: "", protocol: "openai", baseUrl: "", key: "", clearKey: false };
const openrouterUrl = "https://openrouter.ai/api/v1";

export function ProviderConnectionsPage(props: { plugin: Plugin }) {
  const panel = usePanel();
  const location = useLocation();
  const words = () => panel.t().plugins;
  const [connections, setConnections] = createSignal<ProviderConnection[]>();
  const [selected, setSelected] = createSignal<string>();
  const [draft, setDraft] = createSignal<Draft>(emptyDraft);
  const [busy, setBusy] = createSignal(false);
  const [saved, setSaved] = createSignal(false);
  const [error, setError] = createSignal("");
  const current = () => connections()?.find(connection => connection.id === selected());
  const assigned = () => capabilities.filter(capability =>
    panel.snapshot()?.settings.assistant.pipeline[capability].provider === current()?.providerId);
  const stageLabels = { transcription: "transcriptionProvider", chat: "chatProvider", speech: "speechProvider" } as const;
  const baseUrl = () => draft().protocol === "openrouter" ? openrouterUrl : draft().baseUrl.trim();
  const changedEndpoint = () => Boolean(current() && (current()!.protocol !== draft().protocol || current()!.baseUrl !== baseUrl()));
  const dirty = () => Boolean(draft().key || draft().clearKey || draft().name !== (current()?.name ?? "") ||
    draft().protocol !== (current()?.protocol ?? "openai") || draft().baseUrl !== (current()?.baseUrl ?? ""));
  const route = () => `plugin-connections/${encodeURIComponent(props.plugin.id)}`;
  protectDraft(dirty, pathname => pathname.replace(/\/$/, "") === pluginSettingsPath(props.plugin.id));
  let generation = 0;

  function select(connection?: ProviderConnection) {
    setSelected(connection?.id);
    setDraft(connection ? { name: connection.name, protocol: connection.protocol, baseUrl: connection.baseUrl,
      key: "", clearKey: false } : emptyDraft);
    setSaved(false);
    setError("");
  }

  async function load() {
    const request = ++generation;
    setConnections(undefined);
    setBusy(false);
    select();
    try {
      const result = await api<ProviderConnection[]>(route());
      if (request === generation) {
        setConnections(result);
        const requested = new URLSearchParams(location.search).get("connection");
        select(result.find(connection => connection.id === requested) ?? result[0]);
      }
    } catch (reason) {
      if (request === generation) setError(reason instanceof ApiError ? reason.code : "internal");
    }
  }

  createEffect(() => [props.plugin.id, location.search], () => { void load(); });
  onCleanup(() => { generation++; });

  function edit(changes: Partial<Draft>) {
    setDraft(previous => ({ ...previous, ...changes }));
    setError("");
    setSaved(false);
  }

  function choose(connection?: ProviderConnection) {
    if (dirty() && !window.confirm(panel.t().unsaved)) return;
    select(connection);
  }

  async function save() {
    if (busy()) return;
    const request = ++generation;
    setBusy(true);
    setError("");
    try {
      const result = await api<{ id: string; connections: ProviderConnection[] }>(route(), "PUT", {
        id: selected(), name: draft().name.trim(), protocol: draft().protocol, baseUrl: baseUrl(),
        key: draft().clearKey ? "" : (draft().key.trim() || (changedEndpoint() ? "" : undefined)),
      });
      if (request === generation) {
        setConnections(result.connections);
        select(result.connections.find(connection => connection.id === result.id));
        setSaved(true);
      }
    } catch (reason) {
      if (request === generation) setError(reason instanceof ApiError ? reason.code : "internal");
    } finally {
      if (request === generation) setBusy(false);
    }
  }

  async function remove(connection: ProviderConnection) {
    if (busy() || assigned().length || !window.confirm(words().removeConnection(connection.name))) return;
    const request = ++generation;
    setBusy(true);
    setError("");
    try {
      const result = await api<ProviderConnection[]>(`${route()}/${encodeURIComponent(connection.id)}`, "DELETE");
      if (request === generation) {
        setConnections(result);
        select(result[0]);
      }
    } catch (reason) {
      if (request === generation) setError(reason instanceof ApiError ? reason.code : "internal");
    } finally {
      if (request === generation) setBusy(false);
    }
  }

  return <>
    <PageHeader title={props.plugin.title[panel.language()]} />
    <section class={["gw-page", styles.body]} aria-label={words().connections}>
      <div class="gw-row">
        <a class={styles.link} href="/plugins">{words().back}</a>
        <a class={styles.link} href={setupPath}>{panel.t().firstRun.checkWork}</a>
      </div>
      <Show when={connections()} fallback={<Show when={error()} fallback={<p role="status">{words().settingsLoading}</p>}>
        <Button type="button" onClick={() => void load()}>{words().retry}</Button>
      </Show>}>
        {items => <form onSubmit={event => { event.preventDefault(); void save(); }}>
          <Field label={words().connections}>
            {control => <select {...control} disabled={busy()} value={selected() ?? ""} onChange={event => {
              const id = event.currentTarget.value;
              choose(items().find(connection => connection.id === id));
              event.currentTarget.value = selected() ?? "";
            }}>
              <For each={items()} keyed={connection => connection.id}>{connection =>
                <option value={connection().id} selected={selected() === connection().id}>{connection().name}</option>
              }</For>
              <option value="" selected={!selected()}>{words().addConnection}</option>
            </select>}
          </Field>
          <fieldset class={styles.fields} disabled={busy()}>
            <Field label={words().connectionName}>
              {control => <input {...control} value={draft().name} required maxlength={160}
                onInput={event => edit({ name: event.currentTarget.value })} />}
            </Field>
            <Field label={words().connectionProtocol}>
              {control => <select {...control} value={draft().protocol} onChange={event => edit({
                protocol: event.currentTarget.value === "openrouter" ? "openrouter" : "openai",
              })}>
                <option value="openai" selected={draft().protocol === "openai"}>OpenAI-compatible</option>
                <option value="openrouter" selected={draft().protocol === "openrouter"}>OpenRouter</option>
              </select>}
            </Field>
            <Field label={words().connectionUrl}>
              {control => <input {...control} type="url" required maxlength={2048} readonly={draft().protocol === "openrouter"}
                value={draft().protocol === "openrouter" ? openrouterUrl : draft().baseUrl}
                onInput={event => edit({ baseUrl: event.currentTarget.value })} />}
            </Field>
            <Field label={words().connectionKey} hint={current()?.hasKey ?
              (changedEndpoint() ? words().connectionKeyChange : words().secretSaved) : undefined}>
              {control => <input {...control} type="password" autocomplete="new-password" maxlength={512}
                disabled={draft().clearKey} placeholder={current()?.hasKey ? words().replaceSecret : undefined}
                value={draft().key} onInput={event => edit({ key: event.currentTarget.value })} />}
            </Field>
            <Show when={draft().protocol === "openrouter" && !current()?.hasKey}>
              <a class={styles.link} href="https://openrouter.ai/settings/keys" target="_blank" rel="noopener noreferrer">
                {panel.t().firstRun.getOpenRouterKey}
              </a>
            </Show>
            <Show when={current()?.hasKey}>
              <label class={styles.remove}><input type="checkbox" checked={draft().clearKey}
                onChange={event => edit({ clearKey: event.currentTarget.checked, key: "" })} />{words().removeSecret}</label>
            </Show>
          </fieldset>
          <SaveActions labels={panel.t()} dirty={dirty()} saving={busy()} saved={saved()} disabled={!dirty()} />
          <Show when={current()}>
            {connection => <Show when={assigned().length} fallback={
              <div><Button type="button" variant="quiet" disabled={busy()}
                onClick={() => void remove(connection())}>{words().remove}</Button></div>
            }>
              <div class={styles.description}>
                <p>{panel.t().firstRun.inUse}</p>
                <ul><For each={assigned()}>{capability => <li>{panel.t()[stageLabels[capability]]}</li>}</For></ul>
                <a class={styles.link} href={assistantPath}>{panel.t().firstRun.select}</a>
              </div>
            </Show>}
          </Show>
        </form>}
      </Show>
      <Show when={error()}><InlineError>{errorLabel(panel.t(), error())}</InlineError></Show>
    </section>
  </>;
}
