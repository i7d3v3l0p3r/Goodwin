import { createMemo, createSignal, For, Show } from "solid-js";
import { PageHeader } from "../../components/PageHeader";
import { usePanel } from "../../helpers/panel";
import { PluginCard } from "./PluginCard";
import { MiniChat } from "./MiniChat";
import styles from "./NowPage.module.css";

export function NowPage() {
  const panel = usePanel();
  let rack: HTMLDivElement | undefined;
  let hovering = false;
  let active = new Set<string>();
  const [priorities, setPriorities] = createSignal(active);
  function refreshOrder() {
    if (!hovering && !rack?.matches(":focus-within, :active")) setPriorities(active);
  }
  function activity(id: string, value: boolean) {
    if (active.has(id) === value) return;
    active = new Set(active);
    if (value) active.add(id); else active.delete(id);
    refreshOrder();
  }
  const cards = createMemo(() => panel.snapshot()?.plugins.filter(p => p.status === "ready" && p.revision && p.web?.now)
    .sort((a, b) => Number(priorities().has(b.id)) - Number(priorities().has(a.id)) || a.id.localeCompare(b.id)) ?? []);
  return <>
    <PageHeader title={panel.t().now.title} />
    <div class={["gw-page", styles.page]}>
      <Show when={panel.snapshot()} fallback={<p role="status">{panel.status()}</p>}>
        <Show when={panel.assistantNeedsSetup() && !panel.setupDismissed()}>
          <a class={styles.setup} href={panel.builtinProvider() ? "/settings/assistant" : "/setup"}>{panel.t().now.install} →</a>
        </Show>
        <div class={styles.workspace}>
          <MiniChat />
          <Show when={cards().length}>
            <div class={styles.rack} ref={node => {
              rack = node;
              const afterInteraction = () => requestAnimationFrame(refreshOrder);
              node.addEventListener("focusout", afterInteraction);
              node.addEventListener("pointerup", afterInteraction);
              node.addEventListener("pointercancel", afterInteraction);
            }} onPointerEnter={e => { hovering = e.pointerType === "mouse"; }}
              onPointerLeave={() => { hovering = false; refreshOrder(); }}>
              <div class={styles.activity}>
                <For each={cards()} keyed={plugin => plugin.id}>{plugin => <PluginCard plugin={plugin()} activity={value => activity(plugin().id, value)} />}</For>
              </div>
            </div>
          </Show>
        </div>
        <Show when={!panel.builtinProvider() || panel.hasWakeRuntime()}>
          <a class={styles.link} href="/settings/activation#wake-probe-title">{panel.t().now.probe} →</a>
        </Show>
      </Show>
    </div>
  </>;
}
