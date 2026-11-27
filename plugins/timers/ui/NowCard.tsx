import { createEffect, createMemo, createSignal, For, onSettled, Show } from "solid-js";
import type { PluginContext } from "../../../web/src/helpers/plugins";
import { livePlugin } from "../../../web/src/helpers/plugin-live";

interface Timer { id: number; name?: string; seconds: number; expiresAt: number; completed: boolean; waitingForClock?: boolean }
interface Phrase { phrase: string; seconds: number }
interface TimerState { now: number; timers: Timer[]; phrases?: Phrase[]; canExtend?: boolean }

const copy = {
  ru: { timer: "Таймер", empty: "Нет запущенных таймеров", last: "Последний таймер", min: "+1 мин", again: "Запустить ещё раз",
    shortcut: "Запомнить фразу", label: "Голосовая фраза", example: "Например, варим пасту", save: "Сохранить", cancel: "Отмена",
    saved: "Фраза сохранена", remove: "Удалить", phrases: "Мои фразы", more: "Другие таймеры", sync: "Ждём часы",
    loading: "Загрузка…", error: "Таймеры недоступны", failed: "Не удалось выполнить действие", limit: "Сохранено 20 фраз. Удалите одну, чтобы добавить новую.", invalid: "Введите до 64 символов со словами." },
  en: { timer: "Timer", empty: "No timers running", last: "Last timer", min: "+1 min", again: "Start again",
    shortcut: "Save a phrase", label: "Voice phrase", example: "e.g. Time for pasta", save: "Save", cancel: "Cancel",
    saved: "Phrase saved", remove: "Remove", phrases: "My phrases", more: "Other timers", sync: "Waiting for clock",
    loading: "Loading…", error: "Timers unavailable", failed: "Action failed", limit: "Remove an old phrase before adding another (limit 20).", invalid: "Use up to 64 characters with at least one word." },
};

const clock = (ms: number) => {
  const seconds = Math.max(0, Math.ceil(ms / 1000));
  return Math.floor(seconds / 60) + ":" + String(seconds % 60).padStart(2, "0");
};

export function NowCard({ request, language, setNowActive }: PluginContext) {
  const t = copy[language];
  const feed = livePlugin<TimerState>(request, "/timers");
  const [now, setNow] = createSignal(performance.now());
  const [editing, setEditing] = createSignal<Timer>();
  const [phrase, setPhrase] = createSignal("");
  const [saved, setSaved] = createSignal(false);
  const sampled = createMemo(() => ({ state: feed.data(), received: performance.now() }));
  const active = () => feed.data()?.timers.filter(t => !t.completed) ?? [];
  createEffect(() => active().length > 0 && !feed.error(), value => { setNowActive?.(value); });
  const latest = () => feed.data()?.timers.filter(t => t.completed).sort((a,b) => b.id-a.id)[0];
  const first = () => active()[0] ?? latest();
  const disabled = () => feed.busy() || !!feed.error();
  onSettled(() => {
    const tick = window.setInterval(() => { if (!document.hidden && active().length) setNow(performance.now()); }, 500);
    return () => clearInterval(tick);
  });
  function edit(timer: Timer) { setPhrase(""); setEditing(timer); setSaved(false); }
  async function save() {
    if (await feed.act("/phrases", { timer: editing()?.id, phrase: phrase().trim() })) { setEditing(undefined); setSaved(true); }
  }
  function row(timer: () => Timer) {
    return <div class="timers-now-row">
      <span>{timer().name || (timer().completed ? t.last : t.timer)}</span>
      <strong>{timer().waitingForClock ? t.sync : clock(timer().completed ? timer().seconds * 1000 :
        timer().expiresAt - (sampled().state?.now ?? 0) - Math.max(0, now() - sampled().received))}</strong>
      <Show when={timer().completed} fallback={<Show when={feed.data()?.canExtend}>
        <button type="button" disabled={disabled() || timer().waitingForClock} onClick={() => void feed.act("/extend/" + timer().id)}>{t.min}</button>
      </Show>}>
        <button type="button" disabled={disabled()} onClick={() => void feed.act("/timers", { name: timer().name, seconds: timer().seconds })}>{t.again}</button>
      </Show>
      <Show when={feed.data()?.phrases}><button class="timers-now-save" type="button" disabled={disabled()} onClick={() => edit(timer())}>{t.shortcut}</button></Show>
    </div>;
  }
  return <div class="timers-now">
    <Show when={first()} fallback={<p>{feed.error() ? t.error : feed.data() ? t.empty : t.loading}</p>}>{timer => row(timer)}</Show>
    <Show when={active().length > 1}><details><summary>{t.more} · {active().length - 1}</summary>
      <For each={active().slice(1)} keyed={t => t.id}>{timer => row(timer)}</For></details></Show>
    <Show when={editing()}>{timer => <form onSubmit={e => { e.preventDefault(); if (!disabled() && phrase().trim()) void save(); }}>
      <label>{t.label} · {timer().name || t.timer}<input autofocus required maxlength={64} placeholder={t.example} value={phrase()} onInput={e => setPhrase(e.currentTarget.value)} /></label>
      <div class="gw-row"><button disabled={disabled() || !phrase().trim()}>{t.save}</button>
        <button type="button" onClick={() => setEditing(undefined)}>{t.cancel}</button></div>
    </form>}</Show>
    <Show when={saved()}><p role="status">{t.saved}</p></Show>
    <Show when={feed.data()?.phrases?.length}><details><summary>{t.phrases}</summary>
      <For each={feed.data()?.phrases} keyed={p => p.phrase}>{p => <div class="gw-row">
        <span>«{p().phrase}» · {clock(p().seconds * 1000)}</span>
        <button type="button" disabled={disabled()} aria-label={t.remove + " «" + p().phrase + "»"} onClick={() => void feed.act("/phrases/remove", { phrase: p().phrase })}>{t.remove}</button>
      </div>}</For>
    </details></Show>
    <Show when={feed.error() && feed.data()}><p role="status">{t.error}</p></Show>
    <Show when={feed.actionError()}>{code => <p role="alert">{code() === "phrase_limit" ? t.limit : code() === "invalid_phrase" ? t.invalid : t.failed}</p>}</Show>
  </div>;
}
