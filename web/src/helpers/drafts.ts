import { useBeforeLeave } from "@solidjs/router";
import { createContext, createEffect, createSignal, onCleanup, useContext } from "solid-js";

type Draft = { dirty: () => boolean; keptAt: (pathname: string) => boolean };

export function createDraftProtection() {
  const [drafts, setDrafts] = createSignal<Draft[]>([]);
  const [confirmation, setConfirmation] = createSignal<() => void>();

  function register(draft: Draft) {
    setDrafts(previous => [...previous, draft]);
    return () => setDrafts(previous => previous.filter(entry => entry !== draft));
  }

  const hasDrafts = (destination?: string) =>
    drafts().some(draft => draft.dirty() && (destination === undefined || !draft.keptAt(destination)));

  function confirm(action: () => void) {
    setConfirmation(() => action);
  }

  useBeforeLeave(event => {
    // For Back and Forward the browser has already moved to the target URL.
    const pathname = typeof event.to === "number" ? window.location.pathname : new URL(event.to, location.href).pathname;
    if (event.defaultPrevented || !hasDrafts(pathname)) return;
    event.preventDefault();
    confirm(() => event.retry(true));
  });

  createEffect(() => hasDrafts(), dirty => {
    if (!dirty) return;
    function warn(event: BeforeUnloadEvent) {
      event.preventDefault();
      event.returnValue = true;
    }
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  });

  return {
    register, confirmation,
    cancel: () => setConfirmation(undefined),
    proceed: () => {
      const action = confirmation();
      setConfirmation(undefined);
      action?.();
    },
    beforeSignOut: (action: () => void) => {
      if (hasDrafts()) confirm(action);
      else action();
    },
  };
}

export const DraftProtectionContext = createContext<ReturnType<typeof createDraftProtection>>();

export function protectDraft(dirty: Draft["dirty"], keptAt: Draft["keptAt"]) {
  const protection = useContext(DraftProtectionContext);
  onCleanup(protection.register({ dirty, keptAt }));
}
