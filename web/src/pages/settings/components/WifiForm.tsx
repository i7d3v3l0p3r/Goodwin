import { createSignal, onSettled, Show } from "solid-js";
import { Button } from "../../../components/ui-kit/Button";
import { Field } from "../../../components/ui-kit/Field";
import { InlineError, StatusIndicator } from "../../../components/ui-kit/Feedback";
import { api, ApiError, type WifiStatus } from "../../../helpers/api";
import { protectDraft } from "../../../helpers/drafts";
import { usePanel } from "../../../helpers/panel";
import { errorLabel } from "../../../helpers/words";
import { settingsSectionPath } from "../sections";

export function WifiForm() {
  const panel = usePanel();
  const t = panel.t;
  const [status, setStatus] = createSignal<WifiStatus>();
  const [ssid, setSsid] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [error, setError] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const [switching, setSwitching] = createSignal(false);
  const available = () => Boolean(status() && status()?.status !== "connecting");
  protectDraft(() => !switching() && Boolean(ssid() || password()), path => path.replace(/\/$/, "") === settingsSectionPath("network"));

  onSettled(() => {
    let live = true;
    let refreshing = false;
    const refresh = async () => {
      if (refreshing) return;
      refreshing = true;
      try {
        const next = await api<WifiStatus>("network");
        if (!live) return;
        setStatus(next);
        if (next.status !== "connecting") setSwitching(false);
      } catch (reason) {
        if (live && !switching()) setError(reason instanceof ApiError ? reason.code : "internal");
      } finally {
        refreshing = false;
      }
    };
    void refresh();
    const timer = window.setInterval(() => { if (switching()) void refresh(); }, 2000);
    return () => { live = false; window.clearInterval(timer); };
  });

  async function connect(event: SubmitEvent) {
    event.preventDefault();
    if (!ssid() || sending() || switching() || !available()) return;
    setSending(true);
    setError("");
    try {
      await api("network", "PUT", { ssid: ssid(), password: password() });
      setStatus({ status: "connecting" });
      setPassword("");
      setSsid("");
      setSwitching(true);
    } catch (reason) {
      setError(reason instanceof ApiError ? reason.code : "internal");
    } finally {
      setSending(false);
    }
  }

  const connected = () => {
    const current = status();
    return current && current.status !== "connecting" ? current : null;
  };
  return <div class="gw-stack">
    <Show when={connected()}>{current => <>
      <StatusIndicator tone={current().status === "connected" ? "success" : "pending"}>
        {current().status === "connected" ? t().embedded.wifiConnected : t().embedded.wifiSetup}
      </StatusIndicator>
      <p>{current().ssid} · {current().address}</p>
      <Show when={current().error}>{code => <InlineError>{errorLabel(t(), code())}</InlineError>}</Show>
    </>}</Show>
    <Show when={switching()}><p role="status">{t().embedded.wifiSwitching}</p></Show>
    <Show when={error()}><InlineError>{errorLabel(t(), error())}</InlineError></Show>
    <form class="gw-stack" onSubmit={event => void connect(event)}>
      <Field label={t().embedded.wifiName}>
        {control => <input {...control} required maxlength={32} spellcheck={false} value={ssid()}
          onInput={event => setSsid(event.currentTarget.value)} />}
      </Field>
      <Field label={t().embedded.wifiPassword} hint={t().embedded.wifiPasswordHint}>
        {control => <input {...control} type="password" autocomplete="new-password" maxlength={64} value={password()}
          onInput={event => setPassword(event.currentTarget.value)} />}
      </Field>
      <Button type="submit" variant="primary" disabled={!available() || !ssid() || sending() || switching() || panel.busy()}>
        {t().embedded.wifiConnect}
      </Button>
    </form>
  </div>;
}
