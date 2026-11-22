import { createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import type { PluginContext } from "../../../web/src/helpers/plugins";

const copy = {
  en: {
    minutes: "Minutes", seconds: "Seconds", name: "Name (optional)", start: "Start timer",
    active: "Active", completed: "Completed", unnamed: "Timer", cancel: "Cancel",
    empty: "No active timers.", failed: "Could not reach the timer plugin.",
    errors: {
      invalid_timer_duration: "Enter a duration between one second and seven days.",
      invalid_timer_name: "Enter a name of up to 64 characters.",
      timer_limit: "There are already 32 active timers.",
      timer_not_active: "This timer is already finished or cancelled.",
    },
  },
  ru: {
    minutes: "Минуты", seconds: "Секунды", name: "Имя (необязательно)", start: "Поставить таймер",
    active: "Активные", completed: "Завершённые", unnamed: "Таймер", cancel: "Отменить",
    empty: "Активных таймеров нет.", failed: "Не удалось обратиться к плагину таймеров.",
    errors: {
      invalid_timer_duration: "Укажите длительность от одной секунды до семи дней.",
      invalid_timer_name: "Введите имя длиной до 64 символов.",
      timer_limit: "Уже запущено 32 таймера.",
      timer_not_active: "Этот таймер уже завершён или отменён.",
    },
  },
};

interface Timer {
  id: number;
  name: string | null;
  expiresAt: number;
  completed: boolean;
}

interface TimerSnapshot {
  now: number;
  timers: Timer[];
}

function countdown(ms: number) {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const seconds = String(total % 60).padStart(2, "0");
  return hours ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}

export default function TimersPage({ language, request }: PluginContext) {
  const text = copy[language];
  const formatter = new Intl.DateTimeFormat(language, { dateStyle: "short", timeStyle: "short" });
  const [timers, setTimers] = createSignal<Timer[]>([]);
  const [now, setNow] = createSignal(0);
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const active = createMemo(() => timers().filter(timer => !timer.completed));
  const completed = createMemo(() => timers().filter(timer => timer.completed).sort((a, b) => b.expiresAt - a.expiresAt));
  let form!: HTMLFormElement;
  let nameInput!: HTMLInputElement;
  let clockOffset = 0;
  let changing = false;
  let stopped = false;
  let generation = 0;

  async function refresh() {
    const version = ++generation;
    try {
      const snapshot = await request<TimerSnapshot>("/timers");
      if (stopped || generation !== version) return;
      clockOffset = snapshot.now - performance.now();
      setNow(snapshot.now);
      setTimers(snapshot.timers);
    } catch {
      if (!stopped && generation === version) setError(text.failed);
    }
  }

  async function change(operation: () => Promise<unknown>) {
    if (changing) return false;
    changing = true;
    generation++;
    setBusy(true);
    setError("");
    try {
      await operation();
      if (stopped) return false;
      await refresh();
      return !stopped;
    } catch (reason) {
      if (!stopped) {
        const code = reason instanceof Error && "code" in reason ? reason.code : undefined;
        const message = typeof code === "string" && Object.hasOwn(text.errors, code)
          ? text.errors[code as keyof typeof text.errors]
          : text.failed;
        setError(message);
      }
      return false;
    } finally {
      changing = false;
      if (!stopped) setBusy(false);
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const data = new FormData(form);
    const seconds = Number(data.get("minutes")) * 60 + Number(data.get("seconds"));
    if (await change(() => request("/timers", "POST", { seconds, name: nameInput.value.trim() }))) {
      nameInput.value = "";
    }
  }

  const clock = window.setInterval(() => setNow(performance.now() + clockOffset), 250);
  const poll = window.setInterval(() => { if (!changing) void refresh(); }, 1500);
  onCleanup(() => {
    stopped = true;
    window.clearInterval(clock);
    window.clearInterval(poll);
  });
  void refresh();

  return (
    <div class="timers-page gw-stack">
      <gw-card>
        <form ref={form} onSubmit={event => void submit(event)}>
          <label>
            <span>{text.minutes}</span>
            <input name="minutes" type="number" min={0} max={10080} step={1} value="5" />
          </label>
          <label>
            <span>{text.seconds}</span>
            <input name="seconds" type="number" min={0} max={59} step={1} value="0" />
          </label>
          <label>
            <span>{text.name}</span>
            <input ref={nameInput} type="text" maxlength={64} />
          </label>
          <gw-button variant="primary" disabled={busy()} onClick={() => form.requestSubmit()}>
            {text.start}
          </gw-button>
        </form>
      </gw-card>
      <Show when={error()}>
        <p class="timers-error" role="alert">{error()}</p>
      </Show>
      <section>
        <h2>{text.active}</h2>
        <ul>
          <For each={active()} keyed={timer => timer.id}>
            {timer => (
              <li>
                <span>{timer().name || text.unnamed}</span>
                <time>{countdown(timer().expiresAt - now())}</time>
                <gw-button onClick={() => void change(() => request(`/timers/${timer().id}`, "DELETE"))}>
                  {text.cancel}
                </gw-button>
              </li>
            )}
          </For>
        </ul>
        <Show when={active().length === 0}><p>{text.empty}</p></Show>
      </section>
      <Show when={completed().length > 0}>
        <section>
          <h2>{text.completed}</h2>
          <ul>
            <For each={completed()} keyed={timer => timer.id}>
              {timer => (
                <li>
                  <span>{timer().name || text.unnamed}</span>
                  <time datetime={new Date(timer().expiresAt).toISOString()}>{formatter.format(timer().expiresAt)}</time>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>
    </div>
  );
}
