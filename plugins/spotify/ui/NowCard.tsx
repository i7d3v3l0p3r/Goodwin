import { createEffect, createMemo, createSignal, Show } from "solid-js";
import type { PluginContext } from "../../../web/src/helpers/plugins";
import { livePlugin } from "../../../web/src/helpers/plugin-live";

interface State {
  connection: string; device: string | null;
  player?: { status: string; active: boolean; artwork: string | null; controls: string[];
    track: { title: string | null; artists: string[] } | null };
}

const copy = {
  ru: { play: "Продолжить", pause: "Пауза", idle: "Музыка не играет", paused: "На паузе", loading: "Подключение…",
    offline: "Плеер недоступен", device: "эту колонку", pair: (name: string) => `Выберите ${name} в устройствах Spotify` },
  en: { play: "Resume", pause: "Pause", idle: "Nothing playing", paused: "Paused", loading: "Connecting…",
    offline: "Player unavailable", device: "this speaker", pair: (name: string) => `Select ${name} in Spotify Devices` },
};

export function NowCard({ language, request, setNowActive }: PluginContext) {
  const t = copy[language];
  const feed = livePlugin<State>(request, "/player");
  const player = () => feed.data()?.player;
  const playing = () => player()?.active && player()?.status === "playing";
  createEffect(() => Boolean(playing() && !feed.error()), value => { setNowActive?.(value); });
  const action = () => playing() ? "pause" : "play";
  const [image, setImage] = createSignal("");
  const artId = createMemo(() => player()?.artwork);
  createEffect(artId, id => {
    let current = true;
    setImage("");
    if (id && /^[0-9a-f]{32,64}$/i.test(id)) void request<{ image: string }>("/artwork/" + id)
      .then(art => { if (current) setImage(art.image); }).catch(() => {});
    return () => { current = false; };
  });
  return <div class="spotify-now">
    <span class="spotify-now-cover" aria-hidden="true"><Show when={image()}>{src => <img src={src()} alt="" />}</Show></span>
    <div class="spotify-now-track">
      <strong>{player()?.track?.title ?? (feed.error() ? t.offline : !feed.data() ? t.loading : feed.data()?.connection === "pairing" ? t.pair(feed.data()?.device || t.device) : t.idle)}</strong>
      <small>{[player()?.status === "paused" ? t.paused : "", player()?.track?.artists.join(", ")].filter(Boolean).join(" · ")}</small>
    </div>
    <Show when={player()?.controls.includes(action())}><button type="button" class="spotify-now-control" disabled={feed.busy() || !!feed.error()}
      aria-label={playing() ? t.pause : t.play} title={playing() ? t.pause : t.play} onClick={() => void feed.act("/player/" + action())}>
      <i aria-hidden="true" data-action={action()} /></button></Show>
    <Show when={feed.actionError() || (feed.error() && feed.data())}><small class="spotify-now-error" role="status">{t.offline}</small></Show>
  </div>;
}
