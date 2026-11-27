import { createSignal, onCleanup, Show } from "solid-js";
import { InlineError } from "../../../web/src/components/ui-kit/Feedback";
import type { PluginContext } from "../../../web/src/helpers/plugins";

const labels = {
  ru: {
    link: "Ссылка на профиль или подписку", hint: "Вставьте vless:// или HTTP(S)-ссылку провайдера. Если туннель перестанет отвечать, Гудвин запросит по подписке новый профиль.",
    interval: "Интервал проверки туннеля, секунды", applyInterval: "Сохранить интервал",
    badInterval: "Укажите целое число от 5 до 86 400 секунд.", checking: "Проверяем подключение через VPN…",
    unreachable: "Туннель недоступен. Гудвин повторит проверку автоматически.",
    subscriptionFailed: "Провайдер не вернул доступную подписку. Проверьте ссылку и доступ к провайдеру.",
    probeFailed: "Через VPN не удалось получить ответ от адреса проверки.", probeSettings: "Адрес проверки",
    enabled: "Профиль включён", disabled: "Профиль выключен", save: "Включить профиль",
    replace: "Заменить профиль", stop: "Отключить и удалить профиль", plugins: "Выбрать плагины в каталоге",
    routing: "Отметьте плагины, которым нужен VPN. Для них соединение будет доступно только через включённый профиль.",
    invalid: "Проверьте ссылку. В ней есть ошибка или неподдерживаемый параметр.",
    failed: "Не удалось применить профиль. Проверьте параметры подключения.",
    pending: "Подключаем профиль. При первом использовании движок скачивается на устройство.",
    downloadFailed: "Не удалось получить или проверить движок VPN. Проверьте доступ к серверу загрузки и повторите попытку.",
    cancel: "Отменить подключение",
    loading: "Загружаем профиль…", retry: "Повторить",
  },
  en: {
    link: "Profile or subscription link", hint: "Paste a vless:// link or the provider’s HTTP(S) subscription URL. If the tunnel stops responding, Goodwin requests a new profile from the subscription.",
    interval: "Tunnel check interval in seconds", applyInterval: "Save interval",
    badInterval: "Enter a whole number from 5 to 86,400 seconds.", checking: "Checking the connection through the VPN…",
    unreachable: "The tunnel is unavailable. Goodwin will check again automatically.",
    subscriptionFailed: "Could not retrieve the subscription. Check the provider URL and connectivity.",
    probeFailed: "The check address did not respond successfully through the VPN.", probeSettings: "Check address",
    enabled: "Profile enabled", disabled: "Profile off", save: "Enable profile",
    replace: "Replace profile", stop: "Disconnect and remove profile", plugins: "Choose plugins in the catalog",
    routing: "Select the plugins that need the VPN. Their requests require an enabled profile.",
    invalid: "Check the link for invalid or unsupported parameters.", failed: "Could not apply the profile. Check the connection settings.",
    pending: "Connecting the profile. The first use downloads its engine to this device.",
    downloadFailed: "Could not download or verify the VPN engine. Check access to the download server and try again.",
    cancel: "Cancel connection",
    loading: "Loading profile…", retry: "Retry",
  },
} as const;

export default function VpnPage(ctx: PluginContext) {
  const words = labels[ctx.language];
  const [active, setActive] = createSignal<boolean>();
  const [configured, setConfigured] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [pending, setPending] = createSignal(false);
  const [checking, setChecking] = createSignal(false);
  const [unreachable, setUnreachable] = createSignal(false);
  const [savedInterval, setSavedInterval] = createSignal<number>();
  const [seconds, setSeconds] = createSignal("300");
  const [intervalEdited, setIntervalEdited] = createSignal(false);
  const isSubscription = () => /^https?:\/\//i.test(link().trim());
  const [error, setError] = createSignal("");
  const [link, setLink] = createSignal("");
  let poll: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;
  let submittedLink = "";
  onCleanup(() => { disposed = true; clearTimeout(poll); });
  onCleanup(ctx.protectDraft(() => Boolean(link()) || intervalEdited()));

  function message(code?: string) {
    switch (code) {
      case "vpn_engine_download_failed": return words.downloadFailed;
      case "invalid_vless_profile": return words.invalid;
      case "invalid_vpn_interval": return words.badInterval;
      case "vpn_subscription_failed": return words.subscriptionFailed;
      case "vpn_probe_failed": return words.probeFailed;
      default: return words.failed;
    }
  }

  async function update(method: "GET" | "PUT" | "DELETE" | "interval") {
    if (busy()) return;
    clearTimeout(poll);
    let checkAgain = pending();
    let watching = savedInterval() !== undefined;
    setBusy(true);
    setError("");
    try {
      if (method === "PUT") submittedLink = link();
      const state = await ctx.request<{ active: boolean; configured: boolean; pending?: boolean; checking?: boolean; unreachable?: boolean; intervalSeconds?: number; error?: string }>(
        method === "interval" ? "/monitoring" : "/profile", method === "interval" ? "PUT" : method,
        method === "PUT" ? { link: link().trim(), intervalSeconds: Number(seconds()) }
          : method === "interval" ? { intervalSeconds: Number(seconds()) } : undefined);
      if (disposed) return;
      setActive(state.active);
      setConfigured(state.configured);
      checkAgain = Boolean(state.pending);
      watching = state.intervalSeconds !== undefined;
      setChecking(Boolean(state.checking));
      setUnreachable(Boolean(state.unreachable));
      setSavedInterval(state.intervalSeconds);
      if (method === "interval" || method === "DELETE" || (!state.pending && !state.error && submittedLink && link() === submittedLink)) {
        setIntervalEdited(false);
        setSeconds(String(state.intervalSeconds ?? 300));
      } else if (!intervalEdited()) {
        setSeconds(String(state.intervalSeconds ?? 300));
      }
      setPending(checkAgain);
      if (state.error) {
        submittedLink = "";
        setError(message(state.error));
      } else if (!state.pending && (method === "DELETE" || link() === submittedLink)) {
        setLink("");
        submittedLink = "";
      }
    } catch (reason) {
      if (method === "PUT") submittedLink = "";
      const code = (reason as { code?: string })?.code;
      setError(message(code));
    } finally {
      setBusy(false);
      if (!disposed && (checkAgain || watching)) poll = setTimeout(() => void update("GET"), checkAgain ? 750 : 15000);
    }
  }
  void update("GET");
  return <section class="gw-stack">
    <p role="status">{active() === undefined ? (busy() ? words.loading : "") : unreachable() ? words.unreachable : active() ? words.enabled : words.disabled}</p>
    <Show when={pending()}><p role="status">{checking() ? words.checking : words.pending}</p></Show>
    <Show when={error()}><InlineError>{error()}</InlineError></Show>
    <Show when={active() === undefined && !busy()}>
      <button class="gw-button" type="button" onClick={() => void update("GET")}>{words.retry}</button>
    </Show>
    <Show when={active() !== undefined}>
      <form class="gw-stack" onSubmit={event => {
        event.preventDefault();
        if (link().trim()) void update("PUT");
        else if (savedInterval() !== undefined && intervalEdited()) void update("interval");
      }}>
        <label class="gw-field"><span class="gw-field-label">{words.link}</span>
          <input class="gw-control" aria-describedby="vpn-hint" type="password" autocomplete="new-password" placeholder="vless:// · https://" maxlength={4096}
            value={link()} disabled={busy() || (pending() && !checking())} onInput={event => setLink(event.currentTarget.value)} />
        </label>
        <p class="gw-field-hint" id="vpn-hint">{words.hint}</p>
        <Show when={isSubscription() || (savedInterval() !== undefined && !link())}>
          <label class="gw-field"><span class="gw-field-label">{words.interval}</span>
            <input class="gw-control" type="number" min="5" max="86400" step="1" required value={seconds()}
              onInput={event => { setSeconds(event.currentTarget.value); setIntervalEdited(true); }} />
          </label>
          <div class="gw-row">
            <Show when={savedInterval() !== undefined && !link()}><button class="gw-button" type="button"
              disabled={busy() || !intervalEdited() || !Number.isInteger(Number(seconds())) || Number(seconds()) < 5 || Number(seconds()) > 86400}
              onClick={() => void update("interval")}>{words.applyInterval}</button></Show>
            <a href="/plugins/vpn/settings">{words.probeSettings}</a>
          </div>
        </Show>
        <div class="gw-row">
          <button class="gw-button" type="submit" data-variant="primary" disabled={busy() || (pending() && !checking()) || !link().trim()}>{active() ? words.replace : words.save}</button>
          <Show when={configured() || pending()}><button class="gw-button" type="button" disabled={busy()} onClick={() => void update("DELETE")}>{configured() ? words.stop : words.cancel}</button></Show>
        </div>
      </form>
      <p>{words.routing} <a href="/plugins">{words.plugins}</a></p>
    </Show>
  </section>;
}
