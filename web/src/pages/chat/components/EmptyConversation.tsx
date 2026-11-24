import { createEffect, createMemo, createSignal, Show } from "solid-js";
import { SymbolMark } from "../../../components/Brand";
import { ButtonLink } from "../../../components/ui-kit/Button";
import { api, type PluginSettings, type Snapshot } from "../../../helpers/api";
import { pluginSettingsPath } from "../../../helpers/plugins";
import type { Words } from "../../../helpers/words";
import { settingsSectionPath } from "../../settings/sections";
import styles from "./Conversation.module.css";

function needsSettings(id: string, settings: PluginSettings) {
  return settings.fields.some(field => {
    // OpenRouter lets owners remove the saved key, although requests still require one.
    if (!field.required && !(id === "openrouter" && field.id === "key" && field.type === "secret")) return false;
    return field.type === "secret"
      ? !settings.configuredSecrets.includes(field.id)
      : !Object.hasOwn(settings.values, field.id);
  });
}

export function EmptyConversation(props: { t: Words; snapshot?: Snapshot; reachable: boolean }) {
  const [missingSettings, setMissingSettings] = createSignal<string>();
  const providers = () => props.snapshot?.plugins.filter(plugin => plugin.status === "ready" && plugin.providers.includes("chat"));
  const selectedId = () => props.snapshot?.settings.assistant.pipeline.chat.provider;
  const connectionProvider = () => providers()?.find(plugin => plugin.connections?.some(connection => connection.providerId === selectedId()));
  const connection = () => connectionProvider()?.connections?.find(connection => connection.providerId === selectedId());
  const provider = () => connectionProvider() ?? providers()?.find(plugin => !plugin.connection_settings && plugin.id === selectedId());
  const settingsId = createMemo(() => props.reachable && !connection() && provider()?.has_settings ? provider()?.id : undefined);

  createEffect(settingsId, id => {
    setMissingSettings(undefined);
    if (!id) return;
    let current = true;
    void api<PluginSettings>(`plugin-settings/${encodeURIComponent(id)}`).then(settings => {
      if (current && needsSettings(id, settings)) setMissingSettings(id);
    }).catch(() => {});
    return () => { current = false; };
  });

  const setup = () => {
    if (!props.reachable || !props.snapshot) return undefined;
    const t = props.t;
    const selected = props.snapshot.plugins.find(plugin => plugin.id === selectedId());
    if (selected?.status === "starting" || selected?.status === "updating") return undefined;
    if (!provider()) return providers()?.some(plugin => !plugin.connection_settings || plugin.connections?.length)
      ? { description: t.selectChatProviderHint, label: t.selectChatProvider, href: settingsSectionPath("assistant") }
      : { description: t.installChatProviderHint, label: t.plugins.title, href: "/plugins" };
    const id = settingsId();
    if (connection()?.protocol === "openrouter" && !connection()?.hasKey) {
      return { description: t.configureChatProviderHint, label: t.plugins.configure, href: pluginSettingsPath(provider()!.id) };
    }
    if (id && missingSettings() === id) {
      return { description: t.configureChatProviderHint, label: t.plugins.configure, href: pluginSettingsPath(id) };
    }
    return undefined;
  };

  return <div class={styles.emptyChat}>
    <SymbolMark class={styles.emptyMark} />
    <Show when={setup()} fallback={<>
      <h2>{props.t.empty}</h2>
      <p>{props.snapshot?.model && props.snapshot.microphone && props.snapshot.settings.listening
        ? props.t.sayOrWrite(props.snapshot.wake_model?.wake_word ?? props.t.defaultWakeWord) : props.t.write}</p>
    </>}>
      {action => <>
        <h2>{props.t.chatSetupTitle}</h2>
        <p>{action().description}</p>
        <ButtonLink variant="primary" class={styles.setupAction} href={action().href}>{action().label}</ButtonLink>
      </>}
    </Show>
  </div>;
}
