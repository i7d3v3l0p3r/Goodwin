import { For, Show } from "solid-js";
import type { PluginContext } from "../../../web/src/helpers/plugins";

export type Command = "play" | "pause" | "skip_next" | "skip_prev";

const copy = {
  ru: { play: "Продолжить", pause: "Пауза", skip_prev: "Предыдущий трек", skip_next: "Следующий трек" },
  en: { play: "Resume", pause: "Pause", skip_prev: "Previous track", skip_next: "Next track" },
};

export function PlaybackControls(props: {
  language: PluginContext["language"];
  controls: Command[] | undefined;
  action: "play" | "pause";
  disabled?: boolean;
  pending: Command | undefined;
  onCommand: (command: Command) => void;
}) {
  const commands = (): Command[] => ["skip_prev", props.action, "skip_next"];

  return <div class="spotify-controls">
    <For each={commands()} keyed={false}>
      {command => <Show when={props.controls?.includes(command())}>
        <button class="spotify-control" data-command={command()} aria-label={copy[props.language][command()]}
          title={copy[props.language][command()]} type="button" disabled={props.disabled}
          aria-disabled={props.pending ? "true" : undefined} aria-busy={props.pending === command() ? "true" : undefined}
          onClick={() => {
            if (!props.pending) props.onCommand(command());
          }}><span aria-hidden="true" /></button>
      </Show>}
    </For>
  </div>;
}
