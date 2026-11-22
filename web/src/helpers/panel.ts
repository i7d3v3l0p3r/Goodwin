import { createContext, createEffect, createSignal, onSettled, useContext } from "solid-js";
import { ApiError, api, type Access, type Settings, type Snapshot } from "./api";
import { initialLanguage, words, type Language } from "./words";

export function createPanel() {
  const [language, setLanguage] = createSignal<Language>(initialLanguage());
  const [access, setAccess] = createSignal<Access>();
  const [snapshot, setSnapshot] = createSignal<Snapshot>();
  const [reachable, setReachable] = createSignal(true);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal("");
  const t = () => words[language()];
  let stopped = false;
  let refreshVersion = 0;
  let timer = 0;

  function requireLogin() {
    setAccess({ protected: true, authorized: false, setup_required: false });
    setSnapshot(undefined);
  }

  async function refresh(recheckAccess = false) {
    const version = ++refreshVersion;
    try {
      const state = (recheckAccess ? undefined : access()) ?? await api<Access>("access");
      const next = state.authorized ? await api<Snapshot>("state") : undefined;
      if (stopped || version !== refreshVersion) return;
      setAccess(state);
      setSnapshot(previous => previous && next && JSON.stringify(previous.plugins) === JSON.stringify(next.plugins)
        ? { ...next, plugins: previous.plugins }
        : next);
      setReachable(true);
    } catch (reason) {
      if (stopped || version !== refreshVersion) return;
      if (reason instanceof ApiError && reason.code === "login_required") {
        requireLogin();
      } else {
        setReachable(false);
      }
    }
  }

  async function poll() {
    if (!pending()) await refresh(!access()?.authorized);
    if (!stopped) timer = window.setTimeout(poll, 750);
  }

  onSettled(() => {
    void poll();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
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

  async function action(request: () => Promise<unknown>, recheckAccess = false) {
    if (pending()) return false;
    setPending(true);
    clearError();
    refreshVersion++;
    try {
      await request();
      await refresh(recheckAccess);
      return true;
    } catch (reason) {
      const code = reason instanceof ApiError ? reason.code : "internal";
      if (code === "login_required") {
        requireLogin();
      } else if (code === "setup_complete" || code === "setup_required") {
        await refresh(true);
        if (code === "setup_complete") setError(code);
      } else {
        setError(code);
      }
      return false;
    } finally {
      setPending(false);
    }
  }

  function status() {
    const state = snapshot();
    if (!reachable()) return t().disconnected;
    if (!state) return t().connecting;
    if (state.active) return t().stages[state.active.stage];
    if (!state.settings.listening) return t().paused;
    if (!state.microphone) return t().offline;
    if (!state.model) return t().noModel;
    return t().ready;
  }

  return {
    access, snapshot, reachable, pending, error, language, t, status, changeLanguage, clearError,
    busy: () => pending() || !!snapshot()?.active,
    setupPassword: (password: string, confirmation: string) =>
      action(() => api("setup", "POST", { password, confirmation }), true),
    login: (password: string) => action(async () => {
      await api("login", "POST", { password });
      setAccess(undefined);
    }, true),
    logout: () => action(async () => {
      await api("logout", "POST", {});
      requireLogin();
    }, true),
    send: (message: string) => action(() => api("chat", "POST", { message })),
    clearChat: () => action(() => api("chat/clear", "POST", {})),
    saveSettings: (settings: Settings, key?: string) => action(() => api("settings", "PUT", { settings, key })),
    saveSystemPrompt: (systemPrompt: string) =>
      action(() => api("settings/system-prompt", "PUT", { system_prompt: systemPrompt })),
    testStation: (mode: "manual" | "wake") => action(() => api("test", "POST", { mode })),
    uploadPlugin: (file: File) => action(() => api("plugin-packages", "POST", file)),
    removePlugin: (id: string) => action(() => api(`plugin-packages/${id}`, "DELETE")),
  };
}

export const PanelContext = createContext<ReturnType<typeof createPanel>>();

export function usePanel() {
  return useContext(PanelContext);
}
