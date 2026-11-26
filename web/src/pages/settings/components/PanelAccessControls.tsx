import { createSignal, Show } from "solid-js";
import { PanelPasswordForm } from "../../../components/PanelPasswordForm";
import { Button } from "../../../components/ui-kit/Button";
import { ConfirmDialog } from "../../../components/ui-kit/ConfirmDialog";
import { usePanel } from "../../../helpers/panel";
import { errorLabel } from "../../../helpers/words";

export function PanelAccessControls() {
  const panel = usePanel();
  const [creating, setCreating] = createSignal(false);
  const [confirming, setConfirming] = createSignal(false);
  const [disableError, setDisableError] = createSignal("");
  const pending = () => panel.pending() || !panel.reachable();

  function beginCreation() {
    panel.clearError();
    setCreating(true);
  }

  async function disablePassword() {
    setDisableError("");
    if (await panel.openAccess(setDisableError)) setConfirming(false);
  }

  return <Show when={panel.access()?.protected} fallback={<div class="gw-stack">
    <p>{panel.t().openAccess}</p>
    <Show when={creating()} fallback={
      <Button type="button" disabled={pending()} onClick={beginCreation}>{panel.t().enablePassword}</Button>
    }>
      <PanelPasswordForm mode="enable" onSuccess={() => { setCreating(false); }} />
      <Button type="button" disabled={pending()} onClick={() => setCreating(false)}>{panel.t().cancel}</Button>
    </Show>
  </div>}>
    <p>{panel.t().passwordAccess}</p>
    <Button type="button" disabled={pending()} onClick={() => { setDisableError(""); setConfirming(true); }}>
      {panel.t().disablePassword}
    </Button>
    <ConfirmDialog open={confirming()} title={panel.t().disablePassword} description={panel.t().openAccessDescription}
      cancel={panel.t().cancel} confirm={panel.t().disablePassword} busy={panel.pending()}
      error={disableError() ? errorLabel(panel.t(), disableError()) : undefined}
      onCancel={() => setConfirming(false)} onConfirm={() => void disablePassword()} />
  </Show>;
}
