import { createSignal, For, Show } from "solid-js";
import { Button } from "../../components/Button";
import { PageHeader } from "../../components/PageHeader";
import { usePanel } from "../../helpers/panel";
import { errorLabel } from "../../helpers/words";
import { PluginSettingsLink } from "./PluginSettingsLink";
import styles from "./PluginsPage.module.css";

export function PluginsPage() {
  const panel = usePanel();
  const [removing, setRemoving] = createSignal<string>();
  const words = () => panel.t().plugins;
  const installed = () => panel.snapshot()?.plugins;
  const allowed = () => panel.access()?.protected;
  const pending = () => panel.pending() || !panel.reachable();
  let archive!: HTMLInputElement;

  return <>
    <PageHeader title={words().title} />
    <section class={styles.body} aria-label={words().title}>
      <Show when={allowed()} fallback={<p class={styles.note}>{words().password}</p>}>
        <div class={styles.upload}>
          <div>
            <Button type="button" variant="primary" disabled={pending()} onClick={() => archive.click()}>{words().upload}</Button>
            <input ref={archive} type="file" hidden accept=".tar.gz,.tgz,application/gzip" onChange={event => {
              const file = event.currentTarget.files?.[0];
              event.currentTarget.value = "";
              if (file) void panel.uploadPlugin(file);
            }} />
          </div>
          <p class={styles.note}>{words().archive}</p>
        </div>
      </Show>
      <Show when={panel.error()}><p class={styles.error} role="alert">{errorLabel(panel.t(), panel.error())}</p></Show>
      <Show when={installed()}>
        <ul class={styles.list}>
          <For each={installed()} fallback={<li class={styles.empty}>{words().empty}</li>}>
            {plugin => <li class={styles.item}>
              <div class={styles.details}>
                <h2>{plugin.title[panel.language()]}</h2>
                <p class={styles.note}>{plugin.version} · {words().status[plugin.status]}</p>
                <Show when={plugin.error}><p class={styles.error}>{errorLabel(panel.t(), plugin.error!)}</p></Show>
              </div>
              <Show when={allowed() && removing() === plugin.id} fallback={
                <div class="gw-row">
                  <Show when={plugin.has_settings}><PluginSettingsLink plugin={plugin} /></Show>
                  <Show when={allowed()}>
                    <Button type="button" disabled={pending()} onClick={() => setRemoving(plugin.id)}>{words().remove}</Button>
                  </Show>
                </div>
              }>
                <div class={styles.confirm} role="group" aria-label={words().confirm}>
                  <p>{words().confirm}</p>
                  <div class="gw-row">
                    <Button type="button" disabled={pending()} onClick={() => setRemoving(undefined)}>{words().cancel}</Button>
                    <Button class={styles.danger} type="button" disabled={pending()} onClick={async () => {
                      if (await panel.removePlugin(plugin.id)) setRemoving(undefined);
                    }}>{words().remove}</Button>
                  </div>
                </div>
              </Show>
            </li>}
          </For>
        </ul>
      </Show>
    </section>
  </>;
}
