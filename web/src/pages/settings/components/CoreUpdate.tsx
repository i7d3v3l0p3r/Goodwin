import { createSignal, onSettled, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { InlineError, StatusIndicator } from "../../../components/ui-kit/Feedback";
import { ApiError, api } from "../../../helpers/api";
import { errorLabel, type Words } from "../../../helpers/words";
import styles from "./CoreUpdate.module.css";

type Phase = "downloading" | "installing" | "completed" | "download_failed" | "install_failed" | "rolled_back" | "interrupted";
interface UpdateStatus {
  version: string;
  supported: boolean;
  job: { version: string; phase: Phase } | null;
}
interface Release {
  version: string;
  available: boolean;
}

let loadedVersion: string | undefined;

export function CoreUpdate(props: { t: Words; passwordProtected: boolean }) {
  const [status, setStatus] = createSignal<UpdateStatus>();
  const [release, setRelease] = createSignal<Release>();
  const [action, setAction] = createSignal<"check" | "install">();
  const [actionError, setActionError] = createSignal("");
  const [connectionError, setConnectionError] = createSignal("");
  const job = () => status()?.job;
  const updating = () => job()?.phase === "downloading" || job()?.phase === "installing";
  const completed = () => job()?.phase === "completed" && job()?.version === status()?.version;
  const errorCode = (reason: unknown) => reason instanceof ApiError ? reason.code : "internal";

  onSettled(() => {
    let stopped = false;
    let timer = 0;
    async function poll() {
      try {
        const next = await api<UpdateStatus>("core-update", "GET", undefined, 4_000);
        if (stopped) return;
        loadedVersion ??= next.version;
        setStatus(next);
        setConnectionError("");
        if (release()?.available && next.version === release()?.version) setRelease(undefined);
      } catch (reason) {
        if (!stopped) setConnectionError(errorCode(reason));
      }
      if (!stopped) timer = window.setTimeout(poll, 1500);
    }
    void poll();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  });

  async function check() {
    setAction("check");
    setActionError("");
    setRelease(undefined);
    try {
      setRelease(await api<Release>("core-update/latest", "GET", undefined, 20_000));
    } catch (reason) {
      setActionError(errorCode(reason));
    } finally {
      setAction(undefined);
    }
  }

  async function install(version: string) {
    setAction("install");
    setActionError("");
    try {
      await api("core-update", "POST", { version });
      // Keep the action disabled until the next status poll sees the worker.
      setStatus(current => current && { ...current, job: { phase: "downloading", version } });
    } catch (reason) {
      setActionError(errorCode(reason));
    } finally {
      setAction(undefined);
    }
  }

  const t = () => props.t;
  const ready = () => !action() && !updating() && !connectionError();
  const failedPhase = () => {
    const phase = job()?.phase;
    if (phase === "download_failed" || phase === "install_failed" || phase === "rolled_back" || phase === "interrupted") return phase;
  };

  return <div class={styles.content}>
    <Show when={status()} fallback={<Show when={!connectionError()}>
      <StatusIndicator tone="pending">{t().coreUpdateLoading}</StatusIndicator>
    </Show>}>{current => <>
      <p class={styles.version}>{t().coreUpdateInstalled} <strong>{current().version}</strong></p>
      <Show when={current().supported} fallback={<p class={styles.note}>{t().errors.core_update_unsupported}</p>}>
        <Show when={props.passwordProtected} fallback={<p class={styles.note}>{t().errors.core_update_password_required}</p>}>
          <div class={styles.actions}>
            <Button type="button" disabled={!ready()} onClick={() => void check()}>
              {action() === "check" ? t().coreUpdateChecking : t().coreUpdateCheck}
            </Button>
            <Show when={release()?.available ? release() : undefined}>{next =>
              <Button type="button" variant="primary" disabled={!ready()} onClick={() => void install(next().version)}>
                {t().coreUpdateInstall(next().version)}
              </Button>
            }</Show>
          </div>
          <Show when={release()?.available && !updating()}>
            <p class={styles.note}>{t().coreUpdateRestart}</p>
          </Show>
          <Show when={release()?.available === false}>
            <p role="status" class={styles.note}>{t().coreUpdateCurrent}</p>
          </Show>
          <Show when={updating()}>
            <StatusIndicator tone="pending">
              {job()?.phase === "installing" ? t().coreUpdateInstalling : t().coreUpdateDownloading}
            </StatusIndicator>
          </Show>
          <Show when={completed()}>
            <div class={styles.actions}>
              <StatusIndicator tone="success">{t().coreUpdateCompleted}</StatusIndicator>
              <Show when={loadedVersion !== current().version}>
                <Button type="button" variant="quiet" onClick={() => window.location.reload()}>
                  {t().coreUpdateReload}
                </Button>
              </Show>
            </div>
          </Show>
          <Show when={failedPhase()}>{phase => <InlineError>{t().coreUpdateFailures[phase()]}</InlineError>}</Show>
          <Show when={actionError()}>{code => <InlineError>{errorLabel(t(), code())}</InlineError>}</Show>
        </Show>
      </Show>
    </>}</Show>
    <Show when={connectionError()}>{code =>
      updating()
        ? <p role="status" class={styles.note}>{t().coreUpdateReconnecting}</p>
        : <InlineError>{errorLabel(t(), code())}</InlineError>
    }</Show>
  </div>;
}
