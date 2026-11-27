import { createContext, createEffect, createSignal, onSettled, useContext } from "solid-js";
import { ApiError, api, apiOperation, type Access, type DeviceCapabilities, type Message, type Settings, type Snapshot } from "./api";
import { initialLanguage, words, type Language } from "./words";
import { capabilities, createProviderReadiness, wakeReady } from "./readiness";
import { readPinnedPlugins } from "./plugins";

export function createPanel() {
  const [pinnedPlugins, setPinnedPlugins] = createSignal(readPinnedPlugins());
  const [language, setLanguage] = createSignal<Language>(initialLanguage());
  const [access, setAccess] = createSignal<Access>();
  const [device, setDevice] = createSignal<DeviceCapabilities>();
  const [snapshot, setSnapshot] = createSignal<Snapshot>();
  const [chatDraft, setChatDraft] = createSignal<Message>();
  const [draftMessage, setDraftMessage] = createSignal("");
  const [setupDismissed, setSetupDismissed] = createSignal(localStorage.getItem("goodwin-setup-dismissed") === "true");
  const [reachable, setReachable] = createSignal(true);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal("");
  const [microphonePending, setMicrophonePending] = createSignal(false);
  const [microphoneError, setMicrophoneError] = createSignal("");
  const [voiceCancelPending, setVoiceCancelPending] = createSignal(false);
  const [voiceCancelError, setVoiceCancelError] = createSignal("");
  createEffect(() => snapshot()?.active?.request_id, () => { setVoiceCancelError(""); });
  const t = () => words[language()];
  const providerReadiness = createProviderReadiness(snapshot, reachable);
  const builtinProvider = () => device()?.provider_configuration === "builtin";
  const passwordTimeout = () => builtinProvider() ? 150_000 : 10_000;
  const stageIssue = (stage: Parameters<typeof providerReadiness.stageIssue>[0]) =>
    builtinProvider()
      ? snapshot()?.provider_ready ? undefined : { kind: "configure" as const, href: "/settings/assistant" }
      : providerReadiness.stageIssue(stage);
  let events: EventSource | undefined;
  let retry = 0;
  let profileRefresh = 0;
  let retryDelay = 1000;
  let generation = 0;
  let updates: DeviceCapabilities["panel_updates"] | undefined;

  function updateAccess(state: Access) {
    setAccess(state);
    if (!state.authorized) {
      setSnapshot(undefined);
      setChatDraft(undefined);
      setDraftMessage("");
      setMicrophoneError("");
    }
  }

  function disconnect() {
    generation++;
    window.clearTimeout(retry);
    window.clearTimeout(profileRefresh);
    events?.close();
    events = undefined;
  }

  function connect(refreshProfile = false) {
    disconnect();
    if (refreshProfile || !updates) void detectProfile(generation, true);
    else startUpdates(generation);
  }

  function reconnect() {
    disconnect();
    setReachable(false);
    retry = window.setTimeout(() => connect(true), retryDelay);
    retryDelay = Math.min(retryDelay * 2, 15_000);
  }

  function startUpdates(current: number) {
    scheduleProfileRefresh(current);
    if (updates === "poll") {
      void poll(current, true);
      return;
    }
    const source = new EventSource("/api/events");
    events = source;

    source.onopen = () => {
      if (events !== source) return;
      retryDelay = 1000;
      setReachable(true);
    };
    source.onerror = () => {
      if (events === source) reconnect();
    };
    source.addEventListener("access", (event: MessageEvent<string>) => {
      if (events === source) updateAccess(JSON.parse(event.data) as Access);
    });
    source.addEventListener("state", (event: MessageEvent<string>) => {
      if (events !== source) return;
      const next = JSON.parse(event.data) as Snapshot;
      setSnapshot(previous => previous && JSON.stringify(previous.plugins) === JSON.stringify(next.plugins)
        ? { ...next, plugins: previous.plugins }
        : next);
      setChatDraft(next.chat_draft);
    });
    source.addEventListener("chat_delta", (event: MessageEvent<string>) => {
      if (events !== source) return;
      const { id, text } = JSON.parse(event.data) as Pick<Message, "id" | "text">;
      setChatDraft(previous => ({
        id, role: "assistant", text: previous?.id === id ? previous.text + text : text,
      }));
    });
  }

  function scheduleProfileRefresh(current: number) {
    profileRefresh = window.setTimeout(() => void detectProfile(current, false), 30_000);
  }

  async function detectProfile(current: number, connecting: boolean) {
    let profile: DeviceCapabilities | undefined;
    try {
      profile = await api<DeviceCapabilities>("capabilities");
    } catch (reason) {
      if (generation !== current) return;
      if (!(reason instanceof ApiError && reason.code === "not_found")) {
        if (connecting) reconnect();
        else scheduleProfileRefresh(current);
        return;
      }
    }
    if (generation !== current) return;
    const mode = profile?.panel_updates ?? "events";
    setDevice(profile);
    const changed = mode !== updates;
    updates = mode;
    if (connecting) startUpdates(current);
    else if (changed) connect();
    else scheduleProfileRefresh(current);
  }

  async function poll(current: number, refreshAccess = false) {
    try {
      let authorized = access()?.authorized;
      if (refreshAccess || !authorized) {
        const next = await api<Access>("access");
        if (generation !== current) return;
        authorized = next.authorized;
        updateAccess(next);
      }
      if (authorized) {
        const next = await api<Snapshot>("state");
        if (generation !== current) return;
        setSnapshot(next);
      }
      retryDelay = 1000;
      setReachable(true);
    } catch (reason) {
      if (generation !== current) return;
      if (reason instanceof ApiError && reason.code === "login_required") {
        updateAccess({ protected: true, authorized: false, setup_required: false });
      } else {
        reconnect();
        return;
      }
    }
    if (generation === current) retry = window.setTimeout(() => void poll(current), 1000);
  }

  onSettled(() => {
    connect();
    const recheckAccess = () => { if (access()?.authorized === false || !reachable()) connect(true); };
    const restorePage = (event: PageTransitionEvent) => { if (event.persisted) connect(true); };
    window.addEventListener("focus", recheckAccess);
    window.addEventListener("pagehide", disconnect);
    window.addEventListener("pageshow", restorePage);
    return () => {
      window.removeEventListener("focus", recheckAccess);
      window.removeEventListener("pagehide", disconnect);
      window.removeEventListener("pageshow", restorePage);
      disconnect();
    };
  });

  createEffect(language, (lang) => { document.documentElement.lang = lang; });

  function changeLanguage(lang: Language) {
    setLanguage(lang);
    localStorage.setItem("goodwin-language", lang);
  }

  function clearError() {
    setError("");
  }

  async function action(request: () => Promise<unknown>, options: {
    recheckAccess?: boolean; microphone?: boolean; voiceCancel?: boolean; showError?: boolean; onError?: (code: string) => void;
  } = {}) {
    const [isPending, setActionPending, setActionError] = options.microphone
      ? [microphonePending, setMicrophonePending, setMicrophoneError] as const
      : options.voiceCancel
        ? [voiceCancelPending, setVoiceCancelPending, setVoiceCancelError] as const
        : [pending, setPending, setError] as const;
    if (isPending()) return false;
    setActionPending(true);
    setActionError("");
    try {
      await request();
      if (options.recheckAccess || builtinProvider()) connect();
      return true;
    } catch (reason) {
      const code = reason instanceof ApiError ? reason.code : "internal";
      if (code === "login_required") {
        updateAccess({ protected: true, authorized: false, setup_required: false });
        connect();
      } else if (code === "setup_complete" || code === "setup_required") {
        connect();
        if (code === "setup_complete") setActionError(code);
      } else if (options.onError) {
        options.onError(code);
      } else if (options.showError !== false) {
        setActionError(code);
      }
      return false;
    } finally {
      setActionPending(false);
    }
  }

  function status() {
    const state = snapshot();
    if (!reachable()) return t().disconnected;
    if (!state) return t().connecting;
    if (state.active) return t().stages[state.active.stage];
    if (state.settings.microphone_muted) return t().microphoneMuted;
    if (builtinProvider()) {
      if (!state.provider_ready) return t().embedded.provider;
      if (!state.model || !state.settings.listening) return t().embedded.pushToTalk;
    }
    if (!state.settings.listening) return t().paused;
    if (!state.microphone) return t().offline;
    if (!state.model) return t().noModel;
    return t().ready(state.wake_model?.wake_word ?? t().defaultWakeWord);
  }

  return {
    access, snapshot, chatDraft, draftMessage, setDraftMessage, reachable, pending, error, language, t, status, changeLanguage, clearError,
    pinnedPlugins, togglePluginPin(id: string) {
      const pins = pinnedPlugins();
      const next = pins.includes(id) ? pins.filter(pin => pin !== id) : [...pins, id];
      setPinnedPlugins(next);
      try { localStorage.setItem("goodwin-pinned-plugins", JSON.stringify(next)); } catch { /* Keep this session's choice. */ }
    },
    stageIssue, updateProviderSettings: providerReadiness.updateProviderSettings, builtinProvider,
    pluginPackages: () => device()?.plugin_packages ?? !builtinProvider(),
    hasWakeRuntime: () => Boolean(device()?.wake_runtimes.length),
    activationModes: () => device()?.activation ?? [],
    hasWakeCatalog: () => device()?.wake_catalog ?? !builtinProvider(),
    refresh: () => { if (builtinProvider()) connect(); },
    wakeReady: () => reachable() && wakeReady(snapshot()),
    assistantNeedsSetup: () => Boolean(snapshot() && capabilities.some(capability => {
      const issue = stageIssue(capability);
      return issue && issue.kind !== "checking";
    })),
    setupDismissed,
    dismissSetup: () => {
      localStorage.setItem("goodwin-setup-dismissed", "true");
      setSetupDismissed(true);
    },
    busy: () => pending() || !snapshot() || !!snapshot()?.active,
    microphonePending, microphoneError,
    voiceCancelPending,
    voiceCancelError,
    cancelVoice: (request_id: number) => action(async () => {
      try {
        await api("voice/cancel", "POST", { request_id });
      } catch (reason) {
        if (!(reason instanceof ApiError) || reason.code !== "voice_request_inactive") throw reason;
      }
    }, { voiceCancel: true }),
    setMicrophoneMuted: (muted: boolean) => action(() => api("settings/microphone", "PUT", { muted }), { microphone: true }),
    setupPassword: (password: string, confirmation: string, mode: "setup" | "enable") =>
      action(() => apiOperation<void>(mode === "setup" ? "setup" : "access/password", "POST", { password, confirmation }, passwordTimeout(), "auth/result"),
        { recheckAccess: true }),
    openAccess: (onError?: (code: string) => void) =>
      action(() => api("access/open", "POST", {}), { recheckAccess: true, onError }),
    login: (password: string) => action(() => apiOperation<void>("login", "POST", { password }, passwordTimeout(), "auth/result"),
      { recheckAccess: true }),
    logout: () => action(async () => {
      await api("logout", "POST", {});
      updateAccess({ protected: true, authorized: false, setup_required: false });
    }, { recheckAccess: true }),
    send: (message: string) => action(() => api("chat", "POST", { message })),
    testChat: (message: string) => action(() => api("chat", "POST", { message, test: true })),
    clearChat: () => action(() => api("chat/clear", "POST", {})),
    saveSettings: (settings: Settings, options?: { showError?: boolean }) =>
      action(() => api("settings", "PUT", { settings }), options),
    saveAssistant: (settings: Settings) =>
      action(() => api("settings/assistant", "PUT", settings.assistant)),
    saveSystemPrompt: (systemPrompt: string) =>
      action(() => api("settings/system-prompt", "PUT", { system_prompt: systemPrompt })),
    testStation: (mode: "manual" | "wake", onError: (code: string) => void) =>
      action(() => api("test", "POST", { mode }), { onError }),
    uploadPlugin: (file: File) => action(() => api("plugin-packages", "POST", file, builtinProvider() ? 90_000 : undefined)),
    removePlugin: (id: string) => action(() => api(`plugin-packages/${id}`, "DELETE")),
    setPluginEnabled: (id: string, enabled: boolean) => action(() => api(`plugin-enabled/${encodeURIComponent(id)}`, "PUT", enabled, 60_000)),
  };
}

export const PanelContext = createContext<ReturnType<typeof createPanel>>();

export function usePanel() {
  return useContext(PanelContext);
}
