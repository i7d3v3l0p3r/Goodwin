const copy = {
  en: {
    minutes: "Minutes", seconds: "Seconds", name: "Name (optional)", start: "Start timer",
    active: "Active", completed: "Completed", unnamed: "Timer", cancel: "Cancel",
    empty: "No active timers.", failed: "Could not reach the timer plugin.",
    invalid_timer_duration: "Enter a duration between one second and seven days.",
    invalid_timer_name: "Enter a name of up to 64 characters.", timer_limit: "There are already 32 active timers.",
    timer_not_active: "This timer is already finished or cancelled.",
  },
  ru: {
    minutes: "Минуты", seconds: "Секунды", name: "Имя (необязательно)", start: "Поставить таймер",
    active: "Активные", completed: "Завершённые", unnamed: "Таймер", cancel: "Отменить",
    empty: "Активных таймеров нет.", failed: "Не удалось обратиться к плагину таймеров.",
    invalid_timer_duration: "Укажите длительность от одной секунды до семи дней.",
    invalid_timer_name: "Введите имя длиной до 64 символов.", timer_limit: "Уже запущено 32 таймера.",
    timer_not_active: "Этот таймер уже завершён или отменён.",
  },
};

function element(name, text) {
  const node = document.createElement(name);
  if (text !== undefined) node.textContent = text;
  return node;
}

function field(text, settings) {
  const label = element("label");
  const input = Object.assign(element("input"), settings);
  label.append(element("span", text), input);
  return { label, input };
}

function list(text) {
  const section = element("section");
  const rows = element("ul");
  section.append(element("h2", text), rows);
  return { section, rows };
}

function countdown(ms) {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const seconds = String(total % 60).padStart(2, "0");
  return hours ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}

export function mount(container, { language, request }) {
  const text = copy[language];
  const root = element("div");
  root.className = "timers-page gw-stack";
  const form = element("form");
  const card = element("gw-card");
  const minutes = field(text.minutes, { type: "number", min: 0, max: 10080, step: 1, value: "5" });
  const seconds = field(text.seconds, { type: "number", min: 0, max: 59, step: 1, value: "0" });
  const name = field(text.name, { type: "text", maxLength: 64 });
  const start = element("gw-button", text.start);
  start.setAttribute("variant", "primary");
  start.addEventListener("click", () => form.requestSubmit());
  form.append(minutes.label, seconds.label, name.label, start);
  card.append(form);
  const error = element("p");
  error.className = "timers-error";
  error.setAttribute("role", "alert");
  error.hidden = true;
  const active = list(text.active);
  const empty = element("p", text.empty);
  active.section.append(empty);
  const completed = list(text.completed);
  completed.section.hidden = true;
  root.append(card, error, active.section, completed.section);
  container.append(root);

  const formatter = new Intl.DateTimeFormat(language, { dateStyle: "short", timeStyle: "short" });
  const rows = new Map();
  let now = 0;
  let measured = 0;
  let alive = true;
  let busy = false;
  let generation = 0;

  function row(timer) {
    let current = rows.get(timer.id);
    if (!current) {
      const li = element("li");
      const name = element("span", timer.name || text.unnamed);
      const time = element("time");
      li.append(name, time);
      const cancel = element("gw-button", text.cancel);
      cancel.addEventListener("click", () => void change(() => request(`/timers/${timer.id}`, "DELETE")));
      current = { li, time, cancel, timer };
      rows.set(timer.id, current);
    }
    current.timer = timer;
    if (timer.completed) {
      current.cancel.remove();
      current.time.dateTime = new Date(timer.expiresAt).toISOString();
      current.time.textContent = formatter.format(timer.expiresAt);
    } else if (!current.cancel.isConnected) {
      current.li.append(current.cancel);
    }
    return current.li;
  }

  function tick() {
    const time = now + performance.now() - measured;
    for (const { timer, time: label } of rows.values()) {
      if (!timer.completed) label.textContent = countdown(timer.expiresAt - time);
    }
  }

  function reconcile(list, timers) {
    let next = list.firstChild;
    for (const timer of timers) {
      const node = row(timer);
      if (next === node) next = next.nextSibling;
      else list.insertBefore(node, next);
    }
  }

  function message(text) {
    error.textContent = text;
    error.hidden = !text;
  }

  async function refresh() {
    const version = ++generation;
    try {
      const snapshot = await request("/timers");
      if (!alive || version !== generation) return;
      now = snapshot.now;
      measured = performance.now();
      const present = new Set(snapshot.timers.map(timer => timer.id));
      for (const [id, row] of rows) {
        if (!present.has(id)) { row.li.remove(); rows.delete(id); }
      }
      const running = snapshot.timers.filter(timer => !timer.completed);
      const finished = snapshot.timers.filter(timer => timer.completed).sort((a, b) => b.expiresAt - a.expiresAt);
      reconcile(active.rows, running);
      reconcile(completed.rows, finished);
      empty.hidden = running.length > 0;
      completed.section.hidden = finished.length === 0;
      tick();
      return true;
    } catch {
      if (alive && version === generation) message(text.failed);
      return false;
    }
  }

  async function change(operation) {
    if (busy) return false;
    busy = true;
    generation++;
    start.setAttribute("disabled", "");
    message("");
    try {
      await operation();
      if (alive) await refresh();
      return true;
    } catch (reason) {
      if (alive) message(text[reason?.code] || text.failed);
      return false;
    } finally {
      busy = false;
      start.removeAttribute("disabled");
    }
  }

  form.addEventListener("submit", async event => {
    event.preventDefault();
    const duration = Number(minutes.input.value) * 60 + Number(seconds.input.value);
    if (await change(() => request("/timers", "POST", { seconds: duration, name: name.input.value.trim() }))) name.input.value = "";
  });
  const clock = window.setInterval(tick, 250);
  const poll = window.setInterval(() => { if (!busy) void refresh(); }, 1500);
  void refresh();
  return () => {
    alive = false;
    window.clearInterval(clock);
    window.clearInterval(poll);
    root.remove();
  };
}
