import { createContext, createEffect, createSignal, onSettled, useContext } from "solid-js";
import { ApiError, api, type Access, type Message, type Settings, type Snapshot } from "./api";
import { initialLanguage, words, type Language } from "./words";
import { capabilities, createProviderReadiness, wakeReady } from "./readiness";

export function createPanel() {
  const [language, setLanguage] = createSignal<Language>(initialLanguage());
  const [access, setAccess] = createSignal<Access>();
  const [snapshot, setSnapshot] = createSignal<Snapshot>();
  const [chatDraft, setChatDraft] = createSignal<Message>();
  const [draftMessage, setDraftMessage] = createSignal("");
  const [setupDismissed, setSetupDismissed] = createSignal(localStorage.getItem("goodwin-setup-dismissed") === "true");
  const [reachable, setReachable] = createSignal(true);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal("");
  const [microphonePending, setMicrophonePending] = createSignal(false);
  const [microphoneError, setMicrophoneError] = createSignal("");
  const t = () => words[language()];
  const { stageIssue, updateProviderSettings } = createProviderReadiness(snapshot, reachable);
  let events: EventSource | undefined;
  let retry = 0;
  let retryDelay = 1000;

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
    window.clearTimeout(retry);
    events?.close();
    events = undefined;
  }

  function connect() {
    disconnect();
    const source = new EventSource("/api/events");
    events = source;

    source.onopen = () => {
      if (events !== source) return;
      retryDelay = 1000;
      setReachable(true);
    };
    source.onerror = () => {
      if (events !== source) return;
      setReachable(false);
      if (source.readyState === EventSource.CLOSED) {
        retry = window.setTimeout(connect, retryDelay);
        retryDelay = Math.min(retryDelay * 2, 15_000);
      }
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

  onSettled(() => {
    connect();
    const recheckAccess = () => { if (access()?.authorized === false) connect(); };
    const restorePage = (event: PageTransitionEvent) => { if (event.persisted) connect(); };
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
    recheckAccess?: boolean; microphone?: boolean; showError?: boolean; onError?: (code: string) => void;
  } = {}) {
    const [isPending, setActionPending, setActionError] = options.microphone
      ? [microphonePending, setMicrophonePending, setMicrophoneError] as const
      : [pending, setPending, setError] as const;
    if (isPending()) return false;
    setActionPending(true);
    setActionError("");
    try {
      await request();
      if (options.recheckAccess) connect();
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
    if (!state.settings.listening) return t().paused;
    if (!state.microphone) return t().offline;
    if (!state.model) return t().noModel;
    return t().ready(state.wake_model?.wake_word ?? t().defaultWakeWord);
  }

  return {
    access, snapshot, chatDraft, draftMessage, setDraftMessage, reachable, pending, error, language, t, status, changeLanguage, clearError,
    stageIssue, updateProviderSettings, wakeReady: () => reachable() && wakeReady(snapshot()),
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
    setMicrophoneMuted: (muted: boolean) => action(() => api("settings/microphone", "PUT", { muted }), { microphone: true }),
    setupPassword: (password: string, confirmation: string) =>
      action(() => api("setup", "POST", { password, confirmation }), { recheckAccess: true }),
    login: (password: string) => action(() => api("login", "POST", { password }), { recheckAccess: true }),
    logout: () => action(async () => {
      await api("logout", "POST", {});
      updateAccess({ protected: true, authorized: false, setup_required: false });
    }, { recheckAccess: true }),
    send: (message: string) => action(() => api("chat", "POST", { message })),
    testChat: (message: string) => action(() => api("chat", "POST", { message, test: true })),
    clearChat: () => action(() => api("chat/clear", "POST", {})),
    saveSettings: (settings: Settings, options?: { showError?: boolean }) =>
      action(() => api("settings", "PUT", { settings }), options),
    saveSystemPrompt: (systemPrompt: string) =>
      action(() => api("settings/system-prompt", "PUT", { system_prompt: systemPrompt })),
    testStation: (mode: "manual" | "wake", onError: (code: string) => void) =>
      action(() => api("test", "POST", { mode }), { onError }),
    uploadPlugin: (file: File) => action(() => api("plugin-packages", "POST", file)),
    removePlugin: (id: string) => action(() => api(`plugin-packages/${id}`, "DELETE")),
  };
}

export const PanelContext = createContext<ReturnType<typeof createPanel>>();

export function usePanel() {
  return useContext(PanelContext);
}
