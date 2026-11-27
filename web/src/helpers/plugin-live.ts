import { createSignal, onSettled } from "solid-js";
import type { PluginContext } from "./plugins";

// A slow plugin should only ever have one snapshot request in flight.
export function livePlugin<T>(request: PluginContext["request"], path: string) {
  const [data, setData] = createSignal<T>();
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [actionError, setActionError] = createSignal("");
  let mounted = true;
  let version = 0;
  let poll = 0;

  async function refresh() {
    const current = ++version;
    clearTimeout(poll);
    try {
      const result = await request<T>(path);
      if (mounted && current === version) { setData(() => result); setError(""); }
    } catch {
      if (mounted && current === version) setError("unavailable");
    } finally {
      if (mounted && current === version && !document.hidden) poll = window.setTimeout(refresh, 5000);
    }
  }

  async function act(route: string, body?: unknown) {
    if (busy()) return false;
    setBusy(true);
    setActionError("");
    clearTimeout(poll);
    version++;
    try {
      await request(route, "POST", body);
      return true;
    } catch (e) {
      if (mounted) setActionError(e instanceof Error && "code" in e ? String(e.code) : "internal");
      return false;
    } finally {
      if (mounted) { await refresh(); setBusy(false); }
    }
  }

  onSettled(() => {
    const visibility = () => {
      clearTimeout(poll);
      if (!document.hidden && !busy()) void refresh();
    };
    visibility();
    document.addEventListener("visibilitychange", visibility);
    return () => { mounted = false; clearTimeout(poll); document.removeEventListener("visibilitychange", visibility); };
  });
  return { data, error, busy, actionError, act };
}
