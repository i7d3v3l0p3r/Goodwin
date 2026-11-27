import { For, Show } from "solid-js";
import type { PluginContext } from "../../../web/src/helpers/plugins";

export type Command = "play" | "pause" | "skip_next" | "skip_prev";
export type PlaybackStatus = "playing" | "paused" | "idle" | "buffering";

const copy = {
  ru: { play: "Продолжить", pause: "Пауза", skip_prev: "Предыдущий трек", skip_next: "Следующий трек" },
  en: { play: "Resume", pause: "Pause", skip_prev: "Previous track", skip_next: "Next track" },
};

export function PlaybackControls(props: {
  language: PluginContext["language"];
  controls: Command[] | undefined;
  status: PlaybackStatus | null | undefined;
  disabled?: boolean;
  pending: Command | undefined;
  onCommand: (command: Command) => void;
}) {
  const buffering = () => props.status === "buffering";
  const action = () => props.status === "playing" || buffering() ? "pause" : "play";
  const commands = (): Command[] => ["skip_prev", action(), "skip_next"];
  const available = (command: Command) => props.controls?.includes(command) ?? false;

  return <div class="spotify-controls">
    <For each={commands()} keyed={false}>
      {command => <Show when={available(command()) || (buffering() && command() === "pause")}>
        <button class="spotify-control" data-command={command()} aria-label={copy[props.language][command()]}
          title={copy[props.language][command()]} type="button" disabled={props.disabled || !available(command())}
          aria-disabled={props.pending ? "true" : undefined} aria-busy={props.pending === command() ? "true" : undefined}
          onClick={() => {
            if (!props.pending) props.onCommand(command());
          }}><span aria-hidden="true" /></button>
      </Show>}
    </For>
  </div>;
}
