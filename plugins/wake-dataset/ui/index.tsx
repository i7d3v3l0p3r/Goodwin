import { createSignal, For, onCleanup, Show } from "solid-js";
import { EmptyState, InlineError, StatusIndicator } from "../../../web/src/components/ui-kit/Feedback";
import type { PluginContext } from "../../../web/src/helpers/plugins";
import { DatasetArchive, wavBytes } from "./archive";

const copy = {
  en: {
    start: "Start collection", stop: "Stop collection",
    collecting: "Collection enabled", stopped: "Collection stopped",
    hint: "Recordings stay on this device. The minimum score applies to dataset collection.",
    minimum: "Minimum score to save a clip", minimumHint: "Use 0 to keep every clip.", save: "Save",
    capacity: "Storage limit (MiB)", usage: (used: string, limit: number) => `${used} of ${limit} MiB used`,
    pending: "Unlabelled", positive: "Wake word heard", negative: "No wake word",
    download: "Download labelled dataset", exporting: "Preparing archive…",
    load: "Loading recordings…", failed: "The request failed. Try again.",
    login: "Open the panel in password mode and sign in to access recordings.",
    full: "Storage limit reached. Increase the limit or delete recordings to resume collection.",
    clipsFull: "Recording count limit reached. Delete recordings to resume collection.",
    storage: "Collection stopped because the plugin could not save a recording.",
    empty: "No recordings with this label.", previous: "Previous", next: "Next", filter: "Filter by label",
    listen: "Listen", listening: "Loading audio…", stopAudio: "Stop audio", peak: "Peak score", threshold: "Threshold",
    triggered: "Threshold reached", below: "Below threshold",
    clear: "Clear label", remove: "Delete", confirm: "Delete this recording?",
    removeAll: "Delete all", confirmAll: "Permanently delete all recordings, including labelled ones? Collection will stop.",
    recordingManagement: "Manage recordings",
  },
  ru: {
    start: "Начать сбор", stop: "Остановить сбор",
    collecting: "Сбор включён", stopped: "Сбор остановлен",
    hint: "Записи хранятся на устройстве. Минимальная оценка действует при сборе датасета.",
    minimum: "Минимальная оценка для записи", minimumHint: "Укажите 0, чтобы сохранять все фрагменты.", save: "Сохранить",
    capacity: "Лимит хранения (МиБ)", usage: (used: string, limit: number) => `Занято ${used} из ${limit} МиБ`,
    pending: "Без разметки", positive: "Слово прозвучало", negative: "Слова не было",
    download: "Скачать размеченный датасет", exporting: "Готовим архив…",
    load: "Загружаем записи…", failed: "Не удалось выполнить запрос. Попробуйте ещё раз.",
    login: "Для доступа к записям включите режим с паролем и войдите в панель.",
    full: "Достигнут лимит хранения. Увеличьте его или удалите часть записей, чтобы продолжить сбор.",
    clipsFull: "Достигнут лимит количества записей. Удалите часть записей, чтобы продолжить сбор.",
    storage: "Сбор остановлен. Плагину не удалось сохранить запись.",
    empty: "Записей с такой меткой нет.", previous: "Назад", next: "Вперёд", filter: "Фильтр по метке",
    listen: "Прослушать", listening: "Загружаем звук…", stopAudio: "Остановить звук", peak: "Макс. оценка", threshold: "Порог",
    triggered: "Порог достигнут", below: "Ниже порога",
    clear: "Снять метку", remove: "Удалить", confirm: "Удалить эту запись?",
    removeAll: "Удалить все", confirmAll: "Удалить все записи без возможности восстановления, включая размеченные? Сбор остановится.",
    recordingManagement: "Управление записями",
  },
};

type Label = "pending" | "positive" | "negative";
type Filter = Label | "labelled";
const labels: Label[] = ["pending", "positive", "negative"];

interface Clip {
  id: number;
  label: Label;
  peak: number;
  info: {
    model: { id: string; wakeWord: string; kind: string; sha256: string };
    capturedAtMs: number;
    sampleRate: number;
    frameSamples: number;
    gain: number;
    gainApplied: boolean;
    threshold: number;
    scores: number[];
  };
}

interface Status {
  collecting: boolean;
  storageFailed: boolean;
  storageFull: boolean;
  storageMiB: number;
  usedBytes: number;
  limit: number;
  minimumScore: number;
  pending: number;
  positive: number;
  negative: number;
}

interface Page {
  clips: Clip[];
  next: number | null;
  status: Status;
}

export default function DatasetPage({ language, request, protectDraft }: PluginContext) {
  const text = copy[language];
  const decimal = new Intl.NumberFormat(language, { maximumFractionDigits: 4 });
  const date = new Intl.DateTimeFormat(language, { dateStyle: "short", timeStyle: "medium" });
  const [status, setStatus] = createSignal<Status>();
  const [minimumDraft, setMinimumDraft] = createSignal<string>();
  const [capacityDraft, setCapacityDraft] = createSignal<string>();
  const [page, setPage] = createSignal<Page>();
  const savedFilter = new URLSearchParams(window.location.search).get("filter");
  const [filter, setFilter] = createSignal<Label>(labels.find(label => label === savedFilter) ?? "pending");
  const [busy, setBusy] = createSignal(false);
  const [exported, setExported] = createSignal<number>();
  const [error, setError] = createSignal("");
  const [playing, setPlaying] = createSignal<number>();
  const [loadingAudio, setLoadingAudio] = createSignal<number>();
  const [back, setBack] = createSignal<number[]>([]);
  let playback: AudioContext | undefined;
  let source: AudioBufferSourceNode | undefined;
  let audioRequest = 0;
  let listRequest = 0;
  let cursor = 0;
  let stopped = false;
  let poll = 0;
  let changing = false;
  const total = () => {
    const state = status();
    return state ? state.pending + state.positive + state.negative : 0;
  };
  const clipsFull = () => {
    const limit = status()?.limit;
    return limit !== undefined && total() >= limit;
  };
  const full = () => status()?.storageFull || clipsFull();
  const minimumChanged = () => {
    const draft = minimumDraft();
    return draft !== undefined && draft !== "" && Number(draft) !== status()?.minimumScore;
  };
  const capacityChanged = () => {
    const draft = capacityDraft();
    return draft !== undefined && draft !== "" && Number(draft) !== status()?.storageMiB;
  };
  const settingsChanged = () => minimumChanged() || capacityChanged();
  const unregisterDraft = protectDraft(settingsChanged);
  const list = (label: Filter, after: number) => request<Page>(`/clips?filter=${label}&after=${after}`);
  const audio = async (id: number) => wavBytes((await request<{ wav: string }>(`/clips/${id}/audio`)).wav);

  function report(reason: unknown) {
    const code = reason instanceof Error && "code" in reason ? reason.code : undefined;
    setError(code === "plugin_password_required" || code === "login_required" ? text.login : text.failed);
  }

  async function refresh(label: Label, after: number) {
    const token = ++listRequest;
    try {
      const result = await list(label, after);
      if (stopped || token !== listRequest) return;
      setPage(result);
      setStatus(result.status);
      setError("");
    } catch (reason) {
      if (!stopped && token === listRequest) report(reason);
    }
  }

  async function pollList() {
    if (!changing) await refresh(filter(), cursor);
    if (!stopped) poll = window.setTimeout(pollList, 2500);
  }

  function stopAudio() {
    audioRequest++;
    source?.stop();
    source = undefined;
    setPlaying(undefined);
    setLoadingAudio(undefined);
  }

  function showPage(label: Label, after: number) {
    stopAudio();
    cursor = after;
    setPage(undefined);
    void refresh(label, after);
  }

  async function listen(id: number) {
    stopAudio();
    const token = audioRequest;
    setLoadingAudio(id);
    setError("");
    try {
      playback ??= new AudioContext();
      const [bytes] = await Promise.all([audio(id), playback.resume()]);
      if (stopped || token !== audioRequest) return;
      const buffer = await playback.decodeAudioData(bytes.buffer);
      if (stopped || token !== audioRequest) return;
      const node = playback.createBufferSource();
      node.buffer = buffer;
      node.connect(playback.destination);
      node.onended = () => {
        node.disconnect();
        if (source === node) {
          source = undefined;
          setPlaying(undefined);
        }
      };
      node.start();
      source = node;
      setPlaying(id);
    } catch (reason) {
      if (!stopped && token === audioRequest) report(reason);
    } finally {
      if (!stopped && token === audioRequest) setLoadingAudio(undefined);
    }
  }

  async function change(operation: () => Promise<unknown>) {
    if (changing) return;
    changing = true;
    listRequest++;
    setBusy(true);
    setError("");
    try {
      await operation();
      if (!stopped) await refresh(filter(), cursor);
    } catch (reason) {
      if (!stopped) report(reason);
    } finally {
      changing = false;
      if (!stopped) setBusy(false);
    }
  }

  function classify(id: number, label: Label) {
    stopAudio();
    void change(() => request(`/clips/${id}`, "PUT", { label }));
  }

  function remove(id: number) {
    if (!window.confirm(text.confirm)) return;
    stopAudio();
    void change(() => request(`/clips/${id}`, "DELETE"));
  }

  function removeAll() {
    if (!window.confirm(text.confirmAll)) return;
    stopAudio();
    void change(async () => {
      await request("/clips", "DELETE");
      setBack([]);
      cursor = 0;
    });
  }

  function saveCaptureSettings(event: SubmitEvent) {
    event.preventDefault();
    if (!settingsChanged()) return;
    const update: { minimumScore?: number; storageMiB?: number } = {};
    if (minimumChanged()) update.minimumScore = Number(minimumDraft());
    if (capacityChanged()) update.storageMiB = Number(capacityDraft());
    void change(async () => {
      await request("/capture-settings", "PUT", update);
      setMinimumDraft(undefined);
      setCapacityDraft(undefined);
    });
  }

  async function download() {
    if (changing) return;
    changing = true;
    listRequest++;
    setBusy(true);
    setError("");
    setExported(0);
    const archive = new DatasetArchive();
    const metadata: (Clip & { path: string })[] = [];
    try {
      let after = 0;
      do {
        const result = await list("labelled", after);
        if (stopped) return;
        for (const clip of result.clips) {
          const path = `${clip.label}/${clip.id}.wav`;
          const bytes = await audio(clip.id);
          if (stopped) return;
          archive.add(path, bytes, clip.info.capturedAtMs);
          metadata.push({ ...clip, path });
          setExported(metadata.length);
        }
        after = result.next ?? 0;
      } while (after);
      if (metadata.length) {
        const manifest = { formatVersion: 1, audioFormat: "wav-pcm16le-mono", clips: metadata };
        archive.add("manifest.json", new TextEncoder().encode(JSON.stringify(manifest, null, 2)));
        archive.download();
      }
    } catch (reason) {
      if (!stopped) report(reason);
    } finally {
      changing = false;
      if (!stopped) {
        setExported(undefined);
        setBusy(false);
      }
    }
  }

  onCleanup(() => {
    unregisterDraft();
    stopped = true;
    window.clearTimeout(poll);
    stopAudio();
    void playback?.close();
  });
  void pollList();

  return (
    <div class="wake-dataset gw-stack">
      <gw-card>
        <div class="wake-dataset__controls">
          <div class="gw-stack">
            <Show when={status()}>
              {state => <span class="gw-status" data-tone={state().collecting ? "success" : undefined}>
                {state().collecting ? text.collecting : text.stopped}
              </span>}
            </Show>
            <p>{text.hint}</p>
          </div>
          <button type="button" class="gw-button" data-variant="primary" disabled={busy() || !status() || full()}
            onClick={() => void change(() => request("/capture", "PUT", { enabled: !status()?.collecting }))}>
            {status()?.collecting ? text.stop : text.start}
          </button>
        </div>
        <Show when={status()}>
          {state => <form class="wake-dataset__minimum" onSubmit={saveCaptureSettings}>
            <label class="gw-field">
              <span class="gw-field-label">{text.minimum}</span>
              <input class="gw-control" type="number" inputmode="decimal" min="0" max="1" step="any"
                required disabled={busy()} value={minimumDraft() ?? state().minimumScore}
                aria-describedby="wake-dataset-minimum-hint"
                onInput={event => setMinimumDraft(event.currentTarget.value)} />
            </label>
            <label class="gw-field">
              <span class="gw-field-label">{text.capacity}</span>
              <input class="gw-control" type="number" inputmode="numeric" min="1" max="128" step="1"
                required disabled={busy()} value={capacityDraft() ?? state().storageMiB}
                onInput={event => setCapacityDraft(event.currentTarget.value)} />
            </label>
            <button type="submit" class="gw-button" disabled={busy() || !settingsChanged()}>{text.save}</button>
            <p id="wake-dataset-minimum-hint" class="gw-field-hint">{text.minimumHint}</p>
            <p class="gw-field-hint">{text.usage(decimal.format(state().usedBytes / (1024 * 1024)), state().storageMiB)}</p>
          </form>}
        </Show>
        <Show when={full()}><InlineError>{clipsFull() ? text.clipsFull : text.full}</InlineError></Show>
        <Show when={status()?.storageFailed}><InlineError>{text.storage}</InlineError></Show>
      </gw-card>
      <div class="wake-dataset__toolbar">
        <label class="gw-field">
          <span class="gw-field-label">{text.filter}</span>
          <select class="gw-control" value={filter()} disabled={busy()} onChange={event => {
            const label = event.currentTarget.value as Label;
            setFilter(label);
            const url = new URL(window.location.href);
            if (label === "pending") url.searchParams.delete("filter");
            else url.searchParams.set("filter", label);
            window.history.replaceState(window.history.state, "", url);
            setBack([]);
            showPage(label, 0);
          }}>
            {labels.map(label => <option value={label}>{text[label]} ({status()?.[label] ?? 0})</option>)}
          </select>
        </label>
        <div class="wake-dataset__labels">
          <button type="button" class="gw-button" disabled={busy() || !((status()?.positive ?? 0) + (status()?.negative ?? 0))}
            onClick={() => void download()}>{text.download}</button>
        </div>
      </div>
      <Show when={error()}><InlineError>{error()}</InlineError></Show>
      <div role="status">
        <Show when={exported() !== undefined}>
          <StatusIndicator tone="pending">{text.exporting} {exported()}</StatusIndicator>
        </Show>
        <Show when={!error() && !page()}>
          <StatusIndicator tone="pending">{text.load}</StatusIndicator>
        </Show>
        <Show when={page()?.clips.length === 0}><EmptyState>{text.empty}</EmptyState></Show>
      </div>
      <ul class="wake-dataset__clips">
        <For each={page()?.clips ?? []} keyed={clip => clip.id}>
          {clip => <li>
            <gw-card>
              <div class="wake-dataset__clip-head">
                <div>
                  <strong>{clip().info.model.wakeWord}</strong>
                  <p>{clip().info.model.id} · {date.format(clip().info.capturedAtMs)} · {decimal.format(
                    clip().info.scores.length * clip().info.frameSamples / clip().info.sampleRate,
                  )} {language === "ru" ? "с" : "s"}</p>
                </div>
                <div>
                  <strong>{clip().peak >= clip().info.threshold ? text.triggered : text.below}</strong>
                  <p>{text.peak} {decimal.format(clip().peak)} · {text.threshold} {decimal.format(clip().info.threshold)}</p>
                </div>
              </div>
              <div class="wake-dataset__actions">
                <button class="gw-button" type="button" disabled={loadingAudio() === clip().id}
                  onClick={() => playing() === clip().id ? stopAudio() : void listen(clip().id)}>
                  {loadingAudio() === clip().id ? text.listening : playing() === clip().id ? text.stopAudio : text.listen}
                </button>
                <div class="wake-dataset__labels">
                  <Show when={clip().label !== "positive"}>
                    <button class="gw-button" type="button" disabled={busy()} onClick={() => classify(clip().id, "positive")}>{text.positive}</button>
                  </Show>
                  <Show when={clip().label !== "negative"}>
                    <button class="gw-button" type="button" disabled={busy()} onClick={() => classify(clip().id, "negative")}>{text.negative}</button>
                  </Show>
                  <Show when={clip().label !== "pending"}>
                    <button class="gw-button" type="button" data-variant="quiet" disabled={busy()}
                      onClick={() => classify(clip().id, "pending")}>{text.clear}</button>
                  </Show>
                  <button class="gw-button" type="button" data-variant="quiet" disabled={busy()}
                    onClick={() => remove(clip().id)}>{text.remove}</button>
                </div>
              </div>
            </gw-card>
          </li>}
        </For>
      </ul>
      <Show when={page()?.next || back().length}>
        <div class="wake-dataset__pagination">
          <button type="button" class="gw-button" disabled={busy() || !back().length} onClick={() => {
            const history = back();
            const previous = history.at(-1);
            if (previous === undefined) return;
            setBack(history.slice(0, -1));
            showPage(filter(), previous);
          }}>{text.previous}</button>
          <button type="button" class="gw-button" disabled={busy() || !page()?.next} onClick={() => {
            const next = page()?.next;
            if (next == null) return;
            setBack([...back(), cursor]);
            showPage(filter(), next);
          }}>{text.next}</button>
        </div>
      </Show>
      <details class="gw-disclosure">
        <summary>{text.recordingManagement}</summary>
        <button type="button" class="gw-button" data-variant="quiet" disabled={busy() || total() === 0}
          onClick={removeAll}>{text.removeAll}</button>
      </details>
    </div>
  );
}
