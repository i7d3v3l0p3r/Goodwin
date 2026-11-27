import { createSignal, For, onCleanup, Show } from "solid-js";
import { EmptyState, InlineError, StatusIndicator } from "../../../web/src/components/ui-kit/Feedback";
import type { PluginContext } from "../../../web/src/helpers/plugins";
import { PlaybackControls, type Command } from "./PlaybackControls";
import { TrackTitle } from "./TrackTitle";
export { NowCard } from "./NowCard";

const copy = {
  ru: {
    loading: "Подключаемся к плееру…", connect: "Подключить Spotify к колонке",
    wifi: "Откройте Spotify на телефоне или компьютере в той же сети Wi-Fi и включите любой трек.",
    choose: (name: string) => `В списке устройств воспроизведения выберите ${name}.`,
    generic: "ваш приёмник Soloist", paired: "Колонка сохранит привязку. После этого можно просить Гудвина включать музыку голосом.",
    offline: "Не удалось получить состояние Soloist. Проверьте, запущен ли плеер и верно ли указан его адрес в настройках Spotify.",
    invalid: "Проверьте адрес WebSocket в настройках Spotify.", retry: "Проверить снова",
    playing: "Сейчас играет", paused: "На паузе", buffering: "Буферизация", idle: "Воспроизведение остановлено",
    inactive: "Музыка на этой колонке сейчас не играет", unknown: "Без названия",
    progress: "Позиция в треке", queue: "Очередь", history: "История",
    emptyQueue: "В очереди пока нет треков.", emptyHistory: "В этой очереди ещё нет предыдущих треков.",
    historyHint: "Предыдущие треки из очереди Soloist, начиная с последнего.",
    actionError: "Плеер не подтвердил команду.", help: "Как подключить колонку",
  },
  en: {
    loading: "Connecting to the player…", connect: "Connect Spotify to the speaker",
    wifi: "Open Spotify on a phone or computer on the same Wi-Fi network and play a track.",
    choose: (name: string) => `In the device picker, select ${name}.`,
    generic: "your Soloist receiver", paired: "The speaker will remember the pairing. You can then ask Goodwin to play music by voice.",
    offline: "Could not read the Soloist player. Check that it is running and that the receiver address is correct in Spotify settings.",
    invalid: "Check the WebSocket address in Spotify settings.", retry: "Check again",
    playing: "Now playing", paused: "Paused", buffering: "Buffering", idle: "Playback stopped",
    inactive: "This speaker is not currently playing", unknown: "Untitled",
    progress: "Track position", queue: "Queue", history: "History",
    emptyQueue: "There are no upcoming tracks.", emptyHistory: "No previous tracks in this queue yet.",
    historyHint: "Previous tracks in the Soloist queue, most recent first.",
    actionError: "The player did not confirm the command.", help: "How to connect the speaker",
  },
};

type Status = "playing" | "paused" | "idle" | "buffering";
interface Track { title: string | null; artists: string[]; album: string | null; duration: number | null; uri: string | null }
interface Player {
  status: Status; active: boolean | null; track: Track | null; controls: Command[];
  artwork: string | null;
  position: number | null; timestamp: number | null; speed: number | null;
}
type Snapshot = { connection: "pairing"; device: string | null } |
  { connection: "ready"; device: string | null; player: Player; upcoming: Track[]; previous: Track[] };

function time(ms: number) {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function Sleeve(props: { image?: string }) {
  return <div class="spotify-sleeve" aria-hidden="true">
    <span class="spotify-vinyl" />
    <Show when={props.image}>{src => <img src={src()} alt="" />}</Show>
  </div>;
}

export default function SpotifyPage({ language, request }: PluginContext) {
  const t = copy[language];
  const [snapshot, setSnapshot] = createSignal<Snapshot>();
  const [error, setError] = createSignal("");
  const [commandError, setCommandError] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [history, setHistory] = createSignal(false);
  const [now, setNow] = createSignal(Date.now());
  const [image, setImage] = createSignal("");
  let requestedArt: string | null = null;
  let retryArtAfter = 0;
  let stopped = false;
  let revision = 0;
  let poll = 0;

  async function updateArtwork(id: string | null) {
    if (id === requestedArt && (!id || Date.now() < retryArtAfter)) return;
    if (id !== requestedArt) setImage("");
    requestedArt = id;
    retryArtAfter = Infinity;
    if (!id) return;
    try {
      const art = await request<{ image: string }>(`/artwork/${id}`);
      if (!stopped && requestedArt === id) setImage(art.image);
    } catch {
      if (!stopped && requestedArt === id) retryArtAfter = Date.now() + 60000;
    }
  }

  async function refresh() {
    const version = ++revision;
    clearTimeout(poll);
    try {
      const data = await request<Snapshot>("/player");
      if (stopped || version !== revision) return;
      setNow(Date.now());
      setSnapshot(data);
      setError("");
      void updateArtwork(data.connection === "ready" ? data.player.artwork : null);
    } catch (failure) {
      if (!stopped && version === revision) {
        const code = failure instanceof Error && "code" in failure ? failure.code : "";
        setError(code === "invalid_soloist_address" ? t.invalid : t.offline);
      }
    } finally {
      if (!stopped && version === revision && !document.hidden) poll = window.setTimeout(refresh, 5000);
    }
  }

  function visibility() {
    clearTimeout(poll);
    if (!document.hidden && !busy()) void refresh();
  }
  document.addEventListener("visibilitychange", visibility);
  const clock = window.setInterval(() => { if (!document.hidden && player()?.status === "playing") setNow(Date.now()); }, 1000);
  onCleanup(() => {
    stopped = true;
    clearTimeout(poll);
    clearInterval(clock);
    document.removeEventListener("visibilitychange", visibility);
  });
  void refresh();

  async function control(command: Command) {
    if (busy()) return;
    setBusy(true);
    setCommandError(false);
    clearTimeout(poll);
    revision++;
    try { await request(`/player/${command}`, "POST"); }
    catch { if (!stopped) setCommandError(true); }
    finally {
      if (!stopped) { await refresh(); setBusy(false); }
    }
  }

  const ready = () => { const s = snapshot(); return s?.connection === "ready" ? s : undefined; };
  const player = () => ready()?.player;
  const statusText = () => {
    switch (player()?.status) {
      case "playing": return t.playing;
      case "paused": return t.paused;
      case "buffering": return t.buffering;
      default: return t.idle;
    }
  };
  const track = () => player()?.track;
  const device = () => snapshot()?.device;
  const list = () => (history() ? ready()?.previous : ready()?.upcoming) ?? [];
  const position = () => {
    const p = player();
    if (!p || p.position == null) return 0;
    const elapsed = p.status === "playing" && p.timestamp ? Math.max(0, now() - p.timestamp) * (p.speed ?? 1) : 0;
    return Math.min(track()?.duration ?? Infinity, p.position + elapsed);
  };
  const instructions = () => <div class="spotify-pairing">
    <ol><li>{t.wifi}</li><li>{t.choose(device() ? `«${device()}»` : t.generic)}</li></ol>
    <p>{t.paired}</p>
  </div>;

  return <div class="spotify-page gw-stack">
    <Show when={!error()} fallback={<gw-card><div class="spotify-card"><InlineError>{error()}</InlineError>
      <button class="gw-button" type="button" onClick={() => void refresh()}>{t.retry}</button></div></gw-card>}>
      <Show when={snapshot()} fallback={<StatusIndicator tone="pending">{t.loading}</StatusIndicator>}>
        <Show when={ready()} fallback={<section class="spotify-setup">
          <header class="spotify-setup-head"><Sleeve /><div><span class="spotify-device">{device()}</span><h2>{t.connect}</h2></div></header>
          {instructions()}
        </section>}>
          <section class="spotify-player">
            <Sleeve image={image()} />
            <div class="spotify-info">
              <header class="spotify-caption"><h2>{statusText()}</h2><span class="spotify-device">{device()}</span></header>
              <Show when={track()}>{item => <>
                <Show when={item().title || item().uri || t.unknown} keyed>
                  {title => <h3><TrackTitle title={title} /></h3>}
                </Show>
                <Show when={item().artists.length}><p class="spotify-artist">{item().artists.join(", ")}</p></Show>
                <Show when={item().album}><p class="spotify-album">{item().album}</p></Show>
              </>}</Show>
              <Show when={player()?.active === false}><p class="spotify-album">{t.inactive}</p></Show>
            </div>
            <div class="spotify-transport">
              <Show when={track()?.duration && player()?.position != null}>
                <div class="spotify-progress"><span>{time(position())}</span>
                  <progress aria-label={t.progress} value={position()} max={track()!.duration!} />
                  <span>{time(track()!.duration!)}</span></div>
              </Show>
              <PlaybackControls language={language} controls={player()?.controls}
                action={player()?.status === "playing" || player()?.status === "buffering" ? "pause" : "play"}
                disabled={busy()} onCommand={command => void control(command)} />
              <Show when={commandError()}><p class="spotify-control-error" role="alert">{t.actionError}</p></Show>
            </div>
          </section>
          <section class="spotify-list">
            <div class="spotify-switch">
              <button type="button" aria-pressed={history() ? "false" : "true"} onClick={() => setHistory(false)}>{t.queue}<span>{ready()?.upcoming.length}</span></button>
              <button type="button" aria-pressed={history() ? "true" : "false"} onClick={() => setHistory(true)}>{t.history}<span>{ready()?.previous.length}</span></button>
            </div>
            <Show when={history() && list().length > 0}><p>{t.historyHint}</p></Show>
            <Show when={list().length} fallback={<EmptyState>{history() ? t.emptyHistory : t.emptyQueue}</EmptyState>}>
              <ol aria-label={history() ? t.history : t.queue}>
                <For each={list()} keyed={false}>{(item, i) => <li>
                  <span class="spotify-index" aria-hidden="true">{i + 1}</span>
                  <div><strong>{item().title ?? item().uri ?? t.unknown}</strong>
                    <Show when={item().artists.length > 0}><small>{item().artists.join(", ")}</small></Show></div>
                  <Show when={item().duration}>{duration => <span>{time(duration())}</span>}</Show>
                </li>}</For>
              </ol>
            </Show>
          </section>
          <details><summary>{t.help}</summary>{instructions()}</details>
        </Show>
      </Show>
    </Show>
  </div>;
}
