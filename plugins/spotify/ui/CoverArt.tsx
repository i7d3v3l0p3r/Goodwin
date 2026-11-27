import { createEffect, createSignal, For } from "solid-js";
import type { PluginContext } from "../../../web/src/helpers/plugins";

export function CoverArt(props: {
  artwork: string | null | undefined;
  request: PluginContext["request"];
}) {
  const [covers, setCovers] = createSignal<{ src: string }[]>([]);

  createEffect(() => props.artwork, id => {
    if (id === undefined) return;
    if (!id || !/^[0-9a-f]{32,64}$/i.test(id)) {
      setCovers([]);
      return;
    }

    let current = true;
    let retry = 0;
    async function load() {
      try {
        const art = await props.request<{ image: string }>(`/artwork/${id}`);
        if (!current) return;
        const image = new Image();
        image.src = art.image;
        await image.decode();
        if (current) setCovers(previous => previous.at(-1)?.src === art.image
          ? previous : [...previous.slice(-1), { src: art.image }]);
      } catch {
        if (!current) return;
        setCovers([]);
        retry = window.setTimeout(load, 60000);
      }
    }

    void load();
    return () => {
      current = false;
      clearTimeout(retry);
    };
  });

  return <For each={covers()} keyed={cover => cover}>
    {cover => <img class="spotify-cover-image" src={cover().src} alt=""
      onAnimationEnd={event => {
        if (event.animationName === "spotify-cover-in") {
          setCovers(previous => previous.length === 2 && previous.at(-1) === cover()
            ? previous.slice(-1) : previous);
        }
      }} />}
  </For>;
}
