import { createSignal, onSettled, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { ConfirmDialog } from "../../../components/ui-kit/ConfirmDialog";
import { Field } from "../../../components/ui-kit/Field";
import { InlineError, StatusIndicator } from "../../../components/ui-kit/Feedback";
import { ApiError, api, type Access } from "../../../helpers/api";
import { usePanel } from "../../../helpers/panel";
import { errorLabel } from "../../../helpers/words";

interface Firmware {
  version: string;
  max_image_bytes: number;
}

export function FirmwareUpdate() {
  const panel = usePanel();
  const t = panel.t;
  const [info, setInfo] = createSignal<Firmware>();
  const [file, setFile] = createSignal<File>();
  const [error, setError] = createSignal("");
  const [confirm, setConfirm] = createSignal(false);
  const [installing, setInstalling] = createSignal(false);
  const [restarting, setRestarting] = createSignal(false);
  const disabled = () => !info() || !file() || installing() || restarting() || panel.busy();

  onSettled(() => {
    let live = true;
    let timer = 0;
    void api<Firmware>("firmware").then(value => { if (live) setInfo(value); })
      .catch(reason => { if (live) setError(reason instanceof ApiError ? reason.code : "internal"); });
    const waitForRestart = async () => {
      if (!restarting()) return;
      const access = await api<Access>("access", "GET", undefined, 3_000).catch(() => undefined);
      if (live && access && !access.authorized) window.location.reload();
    };
    timer = window.setInterval(() => void waitForRestart(), 4_000);
    return () => { live = false; window.clearInterval(timer); };
  });

  async function install() {
    setConfirm(false);
    const image = file();
    if (!image || disabled()) return;
    setInstalling(true);
    setError("");
    try {
      await api("firmware", "POST", new Blob([image], { type: "application/octet-stream" }), 180_000);
      setRestarting(true);
    } catch (reason) {
      setError(reason instanceof ApiError ? reason.code : "firmware_update_failed");
    } finally {
      setInstalling(false);
    }
  }

  return <div class="gw-stack">
    <Show when={info()}>{current => <p>Goodwin {current().version}</p>}</Show>
    <Field label={t().embedded.firmwareImage} hint={t().embedded.firmwareHint}>
      {control => <input {...control} type="file" accept=".bin,application/octet-stream"
        disabled={installing() || restarting()} onChange={event => {
          const selected = event.currentTarget.files?.[0];
          const fits = selected && selected.size >= 4096 && selected.size <= (info()?.max_image_bytes ?? 0);
          setFile(fits ? selected : undefined);
          setError(selected && !fits ? "invalid_firmware" : "");
        }} />}
    </Field>
    <Button type="button" variant="primary" disabled={disabled()} onClick={() => setConfirm(true)}>
      {t().coreUpdateConfirm}
    </Button>
    <Show when={installing() || restarting()}><StatusIndicator tone="pending">
      {restarting() ? t().coreUpdateRestart : t().embedded.firmwareInstalling}
    </StatusIndicator></Show>
    <Show when={error()}><InlineError>{errorLabel(t(), error())}</InlineError></Show>
    <ConfirmDialog open={confirm()} title={t().coreUpdateConfirm} description={t().coreUpdateRestart}
      cancel={t().cancel} confirm={t().coreUpdateConfirm} onCancel={() => setConfirm(false)}
      onConfirm={() => void install()} />
  </div>;
}
