import { createEffect, createSignal, Show } from "solid-js";
import type { PluginContext } from "../../../web/src/helpers/plugins";
import { livePlugin } from "../../../web/src/helpers/plugin-live";
import { CoverArt } from "./CoverArt";
import { PlaybackControls, type Command } from "./PlaybackControls";
import { TrackTitle } from "./TrackTitle";

interface State {
  connection: string; device: string | null;
  player?: { status: string; active: boolean; artwork: string | null; controls: Command[];
    track: { title: string | null; artists: string[] } | null };
}

const copy = {
  ru: { idle: "Музыка не играет", paused: "На паузе", loading: "Подключение…",
    offline: "Плеер недоступен", device: "эту колонку", pair: (name: string) => `Выберите ${name} в устройствах Spotify` },
  en: { idle: "Nothing playing", paused: "Paused", loading: "Connecting…",
    offline: "Player unavailable", device: "this speaker", pair: (name: string) => `Select ${name} in Spotify Devices` },
};

export function NowCard({ language, request, setNowActive }: PluginContext) {
  const t = copy[language];
  const feed = livePlugin<State>(request, "/player");
  const player = () => feed.data()?.player;
  const playing = () => player()?.active && player()?.status === "playing";
  createEffect(() => Boolean(playing() && !feed.error()), value => { setNowActive?.(value); });
  const status = () => feed.error() ? t.offline : !feed.data() ? t.loading
    : feed.data()?.connection === "pairing" ? t.pair(feed.data()?.device || t.device) : t.idle;
  const [pending, setPending] = createSignal<Command>();
  async function control(command: Command) {
    setPending(command);
    try { await feed.act("/player/" + command); }
    finally { setPending(undefined); }
  }
  return <div class="spotify-now">
    <span class="spotify-now-cover" aria-hidden="true">
      <CoverArt artwork={player()?.artwork ?? (player()?.status === "buffering" ? undefined : null)} request={request} />
    </span>
    <div class="spotify-now-track">
      <Show when={player()?.track?.title} keyed fallback={<strong class="spotify-now-status">{status()}</strong>}>
        {title => <strong><TrackTitle title={title} /></strong>}
      </Show>
      <small>{[player()?.status === "paused" ? t.paused : "", player()?.track?.artists.join(", ")].filter(Boolean).join(" · ")}</small>
    </div>
    <PlaybackControls language={language} controls={player()?.controls} action={playing() ? "pause" : "play"}
      pending={pending()} disabled={!!feed.error()} onCommand={command => void control(command)} />
    <Show when={feed.actionError() || (feed.error() && feed.data())}><small class="spotify-now-error" role="status">{t.offline}</small></Show>
  </div>;
}
