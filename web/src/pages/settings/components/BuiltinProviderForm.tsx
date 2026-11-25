import { createSignal, For, onSettled, Show } from "solid-js";
import { Field } from "../../../components/ui-kit/Field";
import { InlineError, StatusIndicator } from "../../../components/ui-kit/Feedback";
import { SaveActions } from "../../../components/ui-kit/SaveActions";
import { Switch } from "../../../components/ui-kit/Switch";
import { ApiError, api, type BuiltinProvider } from "../../../helpers/api";
import { protectDraft } from "../../../helpers/drafts";
import { usePanel } from "../../../helpers/panel";
import { assistantPath } from "../../../helpers/readiness";
import { errorLabel } from "../../../helpers/words";

const fields = ["base_url", "stt_model", "chat_model", "tts_model", "voice"] as const;

export function BuiltinProviderForm() {
  const panel = usePanel();
  const [saved, setSaved] = createSignal<BuiltinProvider>();
  const [draft, setDraft] = createSignal<Partial<BuiltinProvider>>({});
  const [key, setKey] = createSignal("");
  const [clearKey, setClearKey] = createSignal(false);
  const [error, setError] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  const [confirmed, setConfirmed] = createSignal(false);
  const t = () => panel.t();
  const labels = () => ({ ...t().embedded.providerFields, voice: t().voice });
  const value = (field: typeof fields[number]) => draft()[field] ?? saved()?.[field] ?? "";
  const streaming = () => draft().speech_streaming ?? saved()?.speech_streaming ?? false;
  const dirty = () => fields.some(field => value(field) !== saved()?.[field])
    || streaming() !== (saved()?.speech_streaming ?? false) || key() !== "" || clearKey();
  const disabled = () => !saved() || saving() || !panel.reachable() || Boolean(panel.snapshot()?.active)
    || !dirty() || fields.some(field => !value(field).trim());
  protectDraft(() => Boolean(saved()) && dirty(), path => path.replace(/\/$/, "") === assistantPath);

  function failed(reason: unknown) {
    setError(reason instanceof ApiError ? reason.code : "internal");
    if (reason instanceof ApiError && reason.code === "login_required") panel.refresh();
  }

  onSettled(() => {
    let live = true;
    void api<BuiltinProvider>("settings/provider").then(next => {
      if (live) setSaved(next);
    }).catch(reason => {
      if (live) failed(reason);
    });
    return () => { live = false; };
  });

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (disabled()) return;
    setSaving(true);
    setConfirmed(false);
    setError("");
    try {
      const next = await api<BuiltinProvider>("settings/provider", "PUT", {
        ...Object.fromEntries(fields.map(field => [field, value(field)])),
        api_key: clearKey() ? "" : key() || undefined,
        speech_streaming: streaming(),
      });
      setSaved(next);
      setDraft({});
      setKey("");
      setClearKey(false);
      setConfirmed(true);
      panel.refresh();
    } catch (reason) {
      failed(reason);
    } finally {
      setSaving(false);
    }
  }

  return <>
    <Show when={error()}><InlineError>{errorLabel(t(), error())}</InlineError></Show>
    <Show when={saved()} fallback={<Show when={!error()}><StatusIndicator tone="pending">{t().connecting}</StatusIndicator></Show>}>
      <form class="gw-stack" novalidate onSubmit={event => void save(event)}>
        <For each={fields}>{field =>
          <Field id={field} label={labels()[field]}>
            {control => <input {...control} type={field === "base_url" ? "url" : "text"}
              spellcheck={false} maxlength={256} value={value(field)} readonly={saving()}
              onInput={event => setDraft(previous => ({ ...previous, [field]: event.currentTarget.value }))} />}
          </Field>
        }</For>
        <Field id="provider-key" label={t().embedded.key} hint={saved()?.has_key ? t().embedded.keyStored : undefined}>
          {control => <input {...control} type="password" autocomplete="new-password" maxlength={256}
            value={key()} readonly={saving() || clearKey()} onInput={event => setKey(event.currentTarget.value)} />}
        </Field>
        <Show when={saved()?.has_key}>
          <label><input type="checkbox" checked={clearKey()} disabled={saving()}
            onChange={event => setClearKey(event.currentTarget.checked)} /> {t().embedded.clearKey}</label>
        </Show>
        <Switch label={t().speechStreaming} hint={t().speechStreamingHint} checked={streaming()} disabled={saving()}
          onChange={speech_streaming => setDraft(previous => ({ ...previous, speech_streaming }))} />
        <SaveActions labels={t()} disabled={disabled()} dirty={dirty()} saving={saving()} saved={confirmed()} />
      </form>
    </Show>
  </>;
}
