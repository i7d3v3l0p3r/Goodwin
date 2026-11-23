import { useParams } from "@solidjs/router";
import { createEffect, createSignal, For, Match, onCleanup, Show, Switch } from "solid-js";
import { Button } from "../../components/ui-kit/Button";
import { Disclosure } from "../../components/ui-kit/Disclosure";
import { InlineError } from "../../components/ui-kit/Feedback";
import { Field } from "../../components/ui-kit/Field";
import { SaveActions } from "../../components/ui-kit/SaveActions";
import { PageHeader } from "../../components/PageHeader";
import { api, ApiError, type PluginSettings } from "../../helpers/api";
import { protectDraft } from "../../helpers/drafts";
import { usePanel } from "../../helpers/panel";
import { pluginSettingsPath } from "../../helpers/plugins";
import { errorLabel } from "../../helpers/words";
import { ExtensionPage } from "./ExtensionPage";
import styles from "./PluginSettingsPage.module.css";

type Draft = Record<string, string>;
type Changes = Record<string, string | boolean | number | null>;

export function PluginSettingsRoute() {
  const panel = usePanel();
  const params = useParams<{ id: string }>();
  const legacyPage = () => panel.snapshot()?.plugins.some(plugin =>
    plugin.id === params.id && !plugin.has_settings && plugin.pages.some(page => page.path === "settings"),
  );
  return <Show when={panel.snapshot()} fallback={<PageHeader title={panel.t().settings} />}>
    <Show when={!legacyPage()} fallback={<ExtensionPage />}><PluginSettingsPage /></Show>
  </Show>;
}

function PluginSettingsPage() {
  const panel = usePanel();
  const params = useParams<{ id: string }>();
  const words = () => panel.t().plugins;
  const plugin = () => panel.snapshot()?.plugins.find(entry => entry.id === params.id);
  const [snapshot, setSnapshot] = createSignal<PluginSettings>();
  const [draft, setDraft] = createSignal<Draft>({});
  const [removed, setRemoved] = createSignal<ReadonlySet<string>>(new Set());
  const [busy, setBusy] = createSignal(false);
  const [saved, setSaved] = createSignal(false);
  const [error, setError] = createSignal("");
  const dirty = () => {
    const settings = snapshot();
    return removed().size > 0 || Boolean(settings?.fields.some(({ id }) =>
      (draft()[id] ?? "") !== String(settings.values[id] ?? "")));
  };
  protectDraft(dirty, pathname => pathname.replace(/\/$/, "") === pluginSettingsPath(params.id));
  let generation = 0;
  let fieldsDisclosure: HTMLDetailsElement;

  function revealFields() {
    fieldsDisclosure.open = true;
  }

  function populate(settings: PluginSettings) {
    setSnapshot(settings);
    setDraft(Object.fromEntries(settings.fields.map(field => [
      field.id, String(Object.hasOwn(settings.values, field.id) ? settings.values[field.id] : ""),
    ])));
    setRemoved(new Set<string>());
  }

  async function load(id: string) {
    const request = ++generation;
    setSnapshot(undefined);
    setBusy(false);
    setSaved(false);
    setError("");
    try {
      const settings = await api<PluginSettings>(`plugin-settings/${encodeURIComponent(id)}`);
      if (request === generation) populate(settings);
    } catch (reason) {
      if (request === generation) setError(reason instanceof ApiError ? reason.code : "internal");
    }
  }

  createEffect(() => params.id, id => { void load(id); });
  onCleanup(() => { generation++; });

  function edit(id: string, value: string) {
    setDraft(previous => ({ ...previous, [id]: value }));
    setSaved(false);
    setError("");
  }

  function removeSecret(id: string, remove: boolean) {
    const updated = new Set(removed());
    if (remove) updated.add(id);
    else updated.delete(id);
    setRemoved(updated);
    edit(id, "");
  }

  async function save(settings: PluginSettings, form: HTMLFormElement) {
    if (busy()) return;
    const controls = new FormData(form);
    const updates: [string, Changes[string]][] = [];
    for (const field of settings.fields) {
      const { id } = field;
      if (field.type === "secret") {
        const value = String(controls.get(id) ?? "");
        if (removed().has(id)) updates.push([id, null]);
        else if (value) updates.push([id, value]);
        continue;
      }
      const text = String(controls.get(id) ?? "").trim();
      const value = text === "" ? null
        : field.type === "integer" ? Number(text)
        : field.type === "boolean" ? text === "true"
        : text;
      if (value !== (Object.hasOwn(settings.values, id) ? settings.values[id] : null)) {
        updates.push([id, value]);
      }
    }

    const request = ++generation;
    setBusy(true);
    setSaved(false);
    setError("");
    try {
      const updated = await api<PluginSettings>(`plugin-settings/${encodeURIComponent(params.id)}`,
        "PATCH", Object.fromEntries(updates));
      if (request === generation) {
        populate(updated);
        for (const { id, type } of updated.fields) {
          if (type !== "secret") continue;
          const input = form.elements.namedItem(id);
          if (input instanceof HTMLInputElement) input.value = "";
        }
        setSaved(true);
      }
    } catch (reason) {
      if (request === generation) setError(reason instanceof ApiError ? reason.code : "internal");
    } finally {
      if (request === generation) setBusy(false);
    }
  }

  const message = () => error() === "plugin_not_found" ? words().settingsMissing : errorLabel(panel.t(), error());

  return <>
    <PageHeader title={plugin()?.title[panel.language()] ?? panel.t().settings} />
    <section class={["gw-page", styles.body]} aria-label={panel.t().settings}>
      <a class={styles.link} href="/plugins">{words().back}</a>
      <Show when={plugin()?.status === "error"}>
        <p class={styles.description}>{errorLabel(panel.t(), "plugin_unavailable")}</p>
      </Show>
      <Show when={!snapshot()}>
        <Show when={error()} fallback={<p role="status">{words().settingsLoading}</p>}>
          <InlineError>{message()}</InlineError>
          <Show when={error() !== "plugin_not_found"}>
            <Button type="button" onClick={() => void load(params.id)}>{words().retry}</Button>
          </Show>
        </Show>
      </Show>
      <Show when={snapshot()}>
        {settings => <form onSubmit={event => { event.preventDefault(); void save(settings(), event.currentTarget); }}>
          <Disclosure title={words().settingsFields} class={styles.disclosure} open ref={node => { fieldsDisclosure = node; }}>
            <fieldset class={styles.fields} disabled={busy()}>
              <For each={settings().fields} keyed={field => field.id}>{field => {
                const id = field().id;
                const secretSaved = () => settings().configuredSecrets.includes(id);
                const clearing = () => removed().has(id);
                const hint = () => [field().description?.[panel.language()],
                  field().type === "secret" && secretSaved() && words().secretSaved].filter(Boolean).join(" ");
                return <Field id={`plugin-field-${id}`} label={field().label[panel.language()]} hint={hint()}>
                  {control => <Switch>
                    <Match when={field().type === "boolean"}>
                      <select {...control} name={id} value={draft()[id] ?? ""}
                        required={field().required}
                        onChange={event => edit(id, event.currentTarget.value)} onInvalid={revealFields}>
                        <option value="" selected={!draft()[id]}>{words().unset}</option>
                        <option value="true" selected={draft()[id] === "true"}>{words().yes}</option>
                        <option value="false" selected={draft()[id] === "false"}>{words().no}</option>
                      </select>
                    </Match>
                    <Match when={field().type === "secret"}>
                      <input {...control} name={id} type="password" autocomplete="new-password"
                        maxlength={field().maxLength ?? 512}
                        required={field().required && !secretSaved()} disabled={clearing()}
                        value={draft()[id] ?? ""}
                        placeholder={secretSaved() ? words().replaceSecret : undefined}
                        onInput={event => edit(id, event.currentTarget.value)} onInvalid={revealFields} />
                      <Show when={secretSaved() && !field().required}>
                        <label class={styles.remove}>
                          <input type="checkbox" checked={clearing()}
                            onChange={event => removeSecret(id, event.currentTarget.checked)} />
                          {words().removeSecret}
                        </label>
                      </Show>
                    </Match>
                    <Match when={true}>
                      <input {...control} name={id}
                        type={field().type === "integer" ? "number" : field().type}
                        step={field().type === "integer" ? 1 : undefined}
                        min={field().type === "integer" ? -Number.MAX_SAFE_INTEGER : undefined}
                        max={field().type === "integer" ? Number.MAX_SAFE_INTEGER : undefined}
                        required={field().required}
                        maxlength={field().maxLength ?? (field().type === "url" ? 2048 : 1024)}
                        value={draft()[id] ?? ""} onInput={event => edit(id, event.currentTarget.value)} onInvalid={revealFields} />
                    </Match>
                  </Switch>}
                </Field>;
              }}</For>
            </fieldset>
          </Disclosure>
          <SaveActions labels={panel.t()} dirty={dirty()} saving={busy()} saved={saved()} />
          <Show when={error()}><InlineError>{message()}</InlineError></Show>
        </form>}
      </Show>
    </section>
  </>;
}
