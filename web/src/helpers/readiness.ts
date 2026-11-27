import { createEffect, createMemo, createSignal } from "solid-js";
import { api, type Capability, type PluginSettings, type Snapshot } from "./api";
import { pluginSettingsPath } from "./plugins";

export const setupPath = "/setup";
export const stationPath = "/station";
export const assistantPath = "/settings/assistant";
export const capabilities = ["transcription", "chat", "speech"] as const;

export function assistantStagePath(capability: Capability) {
  return `${assistantPath}?stage=${capability}`;
}

export function wakeReady(snapshot?: Snapshot) {
  return Boolean(snapshot?.microphone && snapshot.model && snapshot.settings.listening && !snapshot.settings.microphone_muted);
}

export function stageProvider(snapshot: Snapshot, capability: Capability) {
  const id = snapshot.settings.assistant.pipeline[capability].provider;
  const owner = snapshot.plugins.find(plugin => plugin.connections?.some(connection => connection.providerId === id));
  const plugin = owner ?? snapshot.plugins.find(plugin => !plugin.connection_settings && plugin.id === id);
  if (plugin?.status !== "ready" || !plugin.providers.includes(capability)) return;
  return { plugin, connection: owner?.connections?.find(connection => connection.providerId === id) };
}

export type StageIssue = { kind: "install" | "select" | "configure" | "checking"; href: string };

export function createProviderReadiness(snapshot: () => Snapshot | undefined, reachable: () => boolean) {
  const settingsPlugins = createMemo(() => {
    const state = snapshot();
    if (!state || !reachable()) return "";
    const plugins = capabilities.map(capability => stageProvider(state, capability))
      .flatMap(provider => provider && !provider.connection && provider.plugin.has_settings
        ? [[provider.plugin.id, [provider.plugin.revision, provider.plugin.settings_revision]] as const] : []);
    return plugins.length ? JSON.stringify([...new Map(plugins)]) : "";
  });
  const [settings, setSettings] = createSignal<Record<string, PluginSettings | null>>({});

  createEffect(settingsPlugins, entries => {
    setSettings({});
    if (!entries) return;
    let current = true;
    for (const [id] of JSON.parse(entries) as [string, unknown][]) {
      void api<PluginSettings>(`plugin-settings/${encodeURIComponent(id)}`).catch(() => null)
        .then(result => { if (current) setSettings(previous => ({ ...previous, [id]: result })); });
    }
    return () => { current = false; };
  });

  function stageIssue(capability: Capability): StageIssue | undefined {
    const state = snapshot();
    if (!state) return { kind: "checking", href: setupPath };
    const provider = stageProvider(state, capability);
    if (!provider) {
      const candidates = state.plugins.filter(plugin => plugin.providers.includes(capability));
      if (candidates.some(plugin => plugin.status === "starting" || plugin.status === "updating")) {
        return { kind: "checking", href: setupPath };
      }
      const plugins = candidates.filter(plugin => plugin.status === "ready");
      if (plugins.some(plugin => !plugin.connection_settings || plugin.connections?.length)) {
        return { kind: "select", href: assistantStagePath(capability) };
      }
      if (plugins.length) return { kind: "configure", href: pluginSettingsPath(plugins[0].id) };
      // The catalog cannot report capabilities while a plugin process restarts.
      if (state.plugins.some(plugin => !plugin.providers.length && (plugin.status === "starting" || plugin.status === "updating"))) {
        return { kind: "checking", href: setupPath };
      }
      return { kind: "install", href: "/plugins" };
    }
    const { plugin, connection } = provider;
    const href = pluginSettingsPath(plugin.id);
    if (connection) {
      return connection.protocol === "openrouter" && !connection.hasKey
        ? { kind: "configure", href: `${href}?connection=${encodeURIComponent(connection.id)}` } : undefined;
    }
    if (!plugin.has_settings) return;
    const status = settings()[plugin.id];
    if (status === undefined) return { kind: "checking", href };
    if (status === null || status.fields.some(field => {
      if (!field.required) return false;
      return field.type === "secret"
        ? !status.configuredSecrets.includes(field.id)
        : !Object.hasOwn(status.values, field.id);
    })) return { kind: "configure", href };
  }
  return {
    stageIssue,
    updateProviderSettings: (id: string, view: PluginSettings) => setSettings(previous => ({ ...previous, [id]: view })),
  };
}
