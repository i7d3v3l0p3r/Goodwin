import { createEffect, createSignal, For, Show } from "solid-js";
import { usePanel } from "../helpers/panel";
import { Icon } from "./icons";
import styles from "./PanelLayout.module.css";

export interface NavigationItem {
  href: string;
  title: string;
  icon: Parameters<typeof Icon>[0]["name"];
  current?: "page" | "location";
  primary?: boolean;
}

export function NavigationLinks(props: { items: NavigationItem[] }) {
  return <For each={props.items} keyed={item => item.href}>{item =>
    <a href={item().href} aria-current={item().current}>
      <Icon name={item().icon} /><span>{item().title}</span>
    </a>
  }</For>;
}

export function MobileNavigation(props: { items: NavigationItem[]; pathname: string; onSignOut: () => void }) {
  const panel = usePanel();
  const [open, setOpen] = createSignal(false);
  const secondary = () => props.items.filter(item => !item.primary);
  let sheet!: HTMLDialogElement;

  createEffect(() => props.pathname, () => { if (sheet.open) sheet.close(); });

  return <>
    <nav class={styles.bottomBar} aria-label={panel.t().nav}>
      <NavigationLinks items={props.items.filter(item => item.primary)} />
      <button type="button" aria-haspopup="dialog" aria-controls="mobile-menu" aria-expanded={open() ? "true" : "false"}
        aria-current={secondary().some(item => item.current) ? "location" : undefined}
        onClick={() => { sheet.showModal(); setOpen(true); }}>
        <Icon name="menu" /><span>{panel.t().menuButton}</span>
      </button>
    </nav>
    <dialog id="mobile-menu" ref={sheet} class={styles.sheet} aria-labelledby="mobile-menu-title"
      onClose={() => setOpen(false)} onClick={event => { if (event.target === sheet) sheet.close(); }}>
      <div class={styles.sheetContent}>
        <header class={styles.sheetHeader}>
          <h2 id="mobile-menu-title">{panel.t().menuButton}</h2>
          <button type="button" aria-label={panel.t().close} onClick={() => sheet.close()}><Icon name="close" /></button>
        </header>
        <div class={styles.sheetBody}>
          <nav class={styles.navigation} aria-label={panel.t().menu} onClick={event => {
            if (event.target instanceof Element && event.target.closest("a") && !event.ctrlKey && !event.metaKey
              && !event.shiftKey && !event.altKey) sheet.close();
          }}>
            <NavigationLinks items={secondary()} />
          </nav>
          <Show when={panel.access()?.protected}>
            <button type="button" class={styles.signOut} disabled={panel.pending()} onClick={() => {
              sheet.close();
              props.onSignOut();
            }}>{panel.t().logout}</button>
          </Show>
        </div>
      </div>
    </dialog>
  </>;
}
