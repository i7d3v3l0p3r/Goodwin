import { createSignal, For, Show } from "solid-js";
import { Button } from "../../components/ui-kit/Button";
import { Icon } from "../../components/icons";
import { api, ApiError, type SystemMessage } from "../../helpers/api";
import { usePanel } from "../../helpers/panel";
import { errorLabel } from "../../helpers/words";
import styles from "./SystemEvent.module.css";

export function SystemEvent(props: { event: SystemMessage }) {
  const panel = usePanel();
  const t = () => panel.t().now;
  const [editing, setEditing] = createSignal(false);
  const [text, setText] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [notice, setNotice] = createSignal("");
  const last = () => panel.snapshot()?.last_voice;
  const correctable = () => props.event.request_id !== undefined && last()?.request_id === props.event.request_id;
  const actions = () => props.event.actions ?? [];
  const pairing = () => actions().some(a => a.status === "select_soloist_in_spotify");
  function pluginName(id: string) {
    return panel.snapshot()?.plugins.find(p => p.id === id)?.title[panel.language()] ?? id;
  }
  function outcome(action: NonNullable<SystemMessage["actions"]>[number]) {
    if (action.status === "select_soloist_in_spotify") return t().pair;
    if (action.failed) return t().failed;
    if (action.status === "pending") return t().pending;
    return action.status === "accepted_by_soloist" ? t().accepted : t().done;
  }
  const summary = () => {
    const e = props.event;
    if (e.error) return panel.t().timingStages[e.stage] + " · " + errorLabel(panel.t(), e.error);
    const action = actions().find(a => a.failed || a.status === "select_soloist_in_spotify") ?? actions().at(-1);
    if (action) return action.status === "select_soloist_in_spotify" ? t().pair : pluginName(action.tool.split("__")[0]) + " · " + outcome(action);
    return (e.source === "chat" ? t().textTurn : t().voiceTurn) + " · " + t().done;
  };
  async function repeat() {
    if (busy() || !correctable() || !text().trim()) return;
    setBusy(true);
    setNotice("");
    try {
      await api("voice/correct", "POST", { request_id: props.event.request_id, text: text().trim() });
      setEditing(false);
      setNotice(t().submitted);
    } catch (error) {
      setNotice(errorLabel(panel.t(), error instanceof ApiError ? error.code : "internal"));
    } finally { setBusy(false); }
  }
  return <div class={styles.event} data-failed={props.event.error || actions().some(action => action.failed) ? "true" : undefined}>
    <details class={styles.log}>
      <summary><Icon name="activity" size={15} /><strong>{t().system}</strong><span>{summary()}</span><Icon name="chevronDown" size={14} /></summary>
      <ol>
        <For each={props.event.timings}>{timing => <li><span>{panel.t().timingStages[timing.stage]}</span>
          <small>{timing.ms.toLocaleString(panel.language())} {panel.t().ms}</small></li>}</For>
        <For each={actions()}>{action => {
          const [id, method] = action.tool.split("__");
          return <li><span>{pluginName(id)} · {t().actions[method as keyof ReturnType<typeof t>["actions"]] ?? method ?? action.tool}</span>
            <small>{outcome(action)}{action.failed ? ` · ${action.status}` : ""}</small></li>;
        }}</For>
        <Show when={props.event.error}>{error => <li>{errorLabel(panel.t(), error())}</li>}</Show>
      </ol>
    </details>
    <Show when={pairing()}><a href="/plugins/spotify">{panel.t().plugins.open} Spotify →</a></Show>
    <Show when={correctable()}>
      <Show when={editing()} fallback={<button type="button" class={styles.edit} disabled={!panel.reachable() || !!panel.snapshot()?.active}
        onClick={() => { setText(last()?.transcript ?? props.event.transcript ?? ""); setEditing(true); }}>{t().edit}</button>}>
        <form onSubmit={e => { e.preventDefault(); void repeat(); }}>
          <label>{t().correction}<textarea autofocus required maxlength={1000} value={text()} onInput={e => setText(e.currentTarget.value)} /></label>
          <div class="gw-row"><Button type="submit" disabled={busy() || !text().trim() || !panel.reachable() || !!panel.snapshot()?.active}>{t().run}</Button>
            <Button type="button" variant="quiet" onClick={() => setEditing(false)}>{panel.t().cancel}</Button></div>
        </form>
      </Show>
    </Show>
    <Show when={notice()}><p role="status">{notice()}</p></Show>
  </div>;
}
