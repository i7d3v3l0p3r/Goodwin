import { createComponent, render } from "@solidjs/web";
import { Errored, type Component } from "solid-js";
import { api, type Plugin } from "./api";
import type { Language } from "./words";

export interface PluginContext {
  pageId: string;
  language: Language;
  request: <T>(path: string, method?: string, body?: unknown) => Promise<T>;
  navigate: (pagePath: string) => void;
  protectDraft: (dirty: () => boolean) => () => void;
  setNowActive?: (active: boolean) => void;
}

interface Module {
  default?: Component<PluginContext>;
  NowCard?: Component<PluginContext>;
  mount?: (container: HTMLElement, context: PluginContext) => void | (() => void);
}

export function pluginPagePath(id: string, path: string) {
  return `/plugins/${id}${path ? `/${path}` : ""}`;
}

export function pluginLandingPage(plugin: Plugin) {
  if (!plugin.web || !plugin.revision) return undefined;
  return plugin.pages.find(page => page.path === "") ?? plugin.pages.find(page => page.navigation && page.path !== "settings");
}

export function readPinnedPlugins(): string[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem("goodwin-pinned-plugins") ?? "null");
    if (Array.isArray(saved)) return saved.filter((id): id is string => typeof id === "string");
  } catch { /* Use the default shortcuts when browser storage is unavailable. */ }
  return ["timers", "spotify"];
}

export function pluginSettingsPath(id: string) {
  return `/plugins/${id}/settings`;
}

export function loadPlugin(plugin: Plugin, container: HTMLElement, context: PluginContext, failed: () => void, view: "page" | "now" = "page") {
  const base = `/plugin-assets/${plugin.id}/${plugin.revision}/`;
  const web = plugin.web!;
  let mounted: void | (() => void);
  let stopped = false;
  const styles = web.styles.map(path => {
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = base + path;
    document.head.append(link);
    return link;
  });
  void import(/* @vite-ignore */ base + web.entry).then((module: Module) => {
    if (stopped) return;
    const Page = view === "now" ? module.NowCard : module.default;
    if (view === "page" && typeof module.mount === "function") mounted = module.mount(container, context);
    else if (typeof Page === "function") mounted = render(() => createComponent(Errored, {
      fallback: () => { failed(); return null; },
      get children() { return createComponent(Page, context); },
    }), container);
    else failed();
  }).catch(() => {
    if (!stopped) failed();
  });

  return () => {
    stopped = true;
    try { mounted?.(); }
    finally {
      container.replaceChildren();
      styles.forEach(link => link.remove());
    }
  };
}

export function pluginRequest<T>(id: string, path: string, method?: string, body?: unknown) {
  const prefix = `/api/plugins/${id}/`;
  const resource = new URL(prefix + path.replace(/^\/+/, ""), location.origin);
  if (!resource.pathname.startsWith(prefix)) return Promise.reject(new Error("Invalid plugin API path"));
  return api<T>((resource.pathname + resource.search).slice(5), method, body);
}
