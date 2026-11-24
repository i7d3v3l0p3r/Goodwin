import { createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { EmptyState, InlineError, StatusIndicator } from "../../../web/src/components/ui-kit/Feedback";
import type { PluginContext } from "../../../web/src/helpers/plugins";

const copy = {
  en: {
    minutes: "Minutes", seconds: "Seconds", name: "Name (optional)", start: "Start timer",
    active: "Active", completed: "Completed", unnamed: "Timer", cancel: "Cancel",
    loading: "Loading timers…", empty: "No active timers.", failed: "Could not reach the timer plugin.",
    stale: "Showing the last available timer data.", cancelNamed: (name: string) => `Cancel timer “${name}”?`,
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
    loading: "Загружаем таймеры…", empty: "Активных таймеров нет.", failed: "Не удалось обратиться к плагину таймеров.",
    stale: "Показаны последние доступные данные.", cancelNamed: (name: string) => `Отменить таймер «${name}»?`,
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

function countdown(total: number) {
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const seconds = String(total % 60).padStart(2, "0");
  return hours ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}

export default function TimersPage({ language, request }: PluginContext) {
  const text = copy[language];
  const formatter = new Intl.DateTimeFormat(language, { dateStyle: "short", timeStyle: "short" });
  const [timers, setTimers] = createSignal<Timer[]>();
  const [now, setNow] = createSignal(0);
  const [connectionFailed, setConnectionFailed] = createSignal(false);
  const [operationError, setOperationError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const active = createMemo(() => timers()?.filter(timer => !timer.completed) ?? []);
  const completed = createMemo(() => timers()?.filter(timer => timer.completed).sort((a, b) => b.expiresAt - a.expiresAt) ?? []);
  let nameInput!: HTMLInputElement;
  let clockOffset = 0;
  let changing = false;
  let stopped = false;
  let generation = 0;
  let poll = 0;

  async function refresh() {
    const version = ++generation;
    try {
      const snapshot = await request<TimerSnapshot>("/timers");
      if (stopped || generation !== version) return;
      clockOffset = snapshot.now - performance.now();
      setNow(snapshot.now);
      setTimers(snapshot.timers);
      setConnectionFailed(false);
    } catch {
      if (!stopped && generation === version) setConnectionFailed(true);
    }
  }

  async function pollTimers() {
    if (!changing) await refresh();
    if (!stopped) poll = window.setTimeout(pollTimers, 1500);
  }

  async function change(operation: () => Promise<unknown>) {
    if (changing) return false;
    changing = true;
    generation++;
    setBusy(true);
    setOperationError("");
    try {
      await operation();
      if (stopped) return false;
      await refresh();
      return !stopped;
    } catch (reason) {
      if (!stopped) {
        const code = reason instanceof Error && "code" in reason ? reason.code : undefined;
        if (typeof code === "string" && Object.hasOwn(text.errors, code)) {
          setOperationError(text.errors[code as keyof typeof text.errors]);
        } else {
          setConnectionFailed(true);
        }
      }
      return false;
    } finally {
      changing = false;
      if (!stopped) setBusy(false);
    }
  }

  async function submit(event: SubmitEvent & { currentTarget: HTMLFormElement }) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const seconds = Number(data.get("minutes")) * 60 + Number(data.get("seconds"));
    const name = String(data.get("name") ?? "");
    if (await change(() => request("/timers", "POST", { seconds, name: name.trim() })) && nameInput.value === name) {
      nameInput.value = "";
    }
  }

  const clock = window.setInterval(() => {
    if (!connectionFailed()) setNow(performance.now() + clockOffset);
  }, 250);
  onCleanup(() => {
    stopped = true;
    window.clearInterval(clock);
    window.clearTimeout(poll);
  });
  void pollTimers();

  return (
    <div class="timers-page gw-stack">
      <gw-card>
        <form onSubmit={event => void submit(event)}>
          <label class="gw-field timer-duration">
            <span class="gw-field-label">{text.minutes}</span>
            <input class="gw-control" name="minutes" type="number" min={0} max={10080} step={1} value="5" />
          </label>
          <label class="gw-field timer-duration">
            <span class="gw-field-label">{text.seconds}</span>
            <input class="gw-control" name="seconds" type="number" min={0} max={59} step={1} value="0" />
          </label>
          <label class="gw-field timer-name">
            <span class="gw-field-label">{text.name}</span>
            <input class="gw-control" ref={nameInput} name="name" type="text" maxlength={64} />
          </label>
          <button class="gw-button" data-variant="primary" type="submit" disabled={busy() || connectionFailed()}>
            {text.start}
          </button>
        </form>
      </gw-card>
      <Show when={operationError()}><InlineError>{operationError()}</InlineError></Show>
      <section>
        <h2>{text.active}</h2>
        <Show when={connectionFailed()}>
          <div class="timer-list-error"><InlineError>{text.failed} {timers() ? text.stale : ""}</InlineError></div>
        </Show>
        <div class="timer-list-status" role="status">
          <Show when={!connectionFailed()}>
            <Show when={timers()} fallback={<StatusIndicator tone="pending">{text.loading}</StatusIndicator>}>
              <Show when={active().length === 0}><EmptyState>{text.empty}</EmptyState></Show>
            </Show>
          </Show>
        </div>
        <Show when={active().length}>
          <ul>
            <For each={active()} keyed={timer => timer.id}>
              {timer => {
                const remaining = createMemo(() => Math.max(0, Math.ceil((timer().expiresAt - now()) / 1000)));
                return <li class="timer-active">
                  <span>{timer().name || text.unnamed}</span>
                  <time datetime={`PT${remaining()}S`}>{countdown(remaining())}</time>
                  <button class="gw-button" type="button" disabled={busy() || connectionFailed()}
                    aria-label={text.cancelNamed(timer().name || text.unnamed)} onClick={() => {
                      if (window.confirm(text.cancelNamed(timer().name || text.unnamed))) {
                        void change(() => request(`/timers/${timer().id}`, "DELETE"));
                      }
                    }}>
                    {text.cancel}
                  </button>
                </li>;
              }}
            </For>
          </ul>
        </Show>
      </section>
      <Show when={completed().length > 0}>
        <section>
          <h2>{text.completed}</h2>
          <ul>
            <For each={completed()} keyed={timer => timer.id}>
              {timer => (
                <li class="timer-completed">
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
