import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import type { Words } from "../../helpers/words";
import { Icon } from "../icons";
import styles from "./ChoicePicker.module.css";

interface Choice {
  value: string;
  label: string;
  detail?: string;
}

export function ChoicePicker(props: {
  id: string;
  title: string;
  value: string;
  options: Choice[];
  action?: Choice & { queryLabel?: string };
  disabled?: boolean;
  "aria-describedby"?: string;
  t: Words;
  searchHint?: string;
  onSelect: (value: string, query?: string) => void;
}) {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [cursor, setCursor] = createSignal(0);
  const listId = `${props.id}-list`;
  const optionId = (index: number) => `${listId}-${index}`;
  let trigger!: HTMLButtonElement;
  let dialog!: HTMLDialogElement;
  let search!: HTMLInputElement;

  const selected = createMemo(() => props.options.find(item => item.value === props.value)
    ?? (props.value === props.action?.value ? props.action : undefined));
  const matches = createMemo(() => {
    if (!open()) return [];
    const words = query().trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return props.options;
    return props.options.filter(item => {
      const text = `${item.label} ${item.detail ?? ""} ${item.value}`.toLowerCase();
      return words.every(word => text.includes(word));
    });
  });
  const active = () => Math.min(cursor(), matches().length - 1);
  const customQuery = () => props.action?.queryLabel && !matches().length ? query().trim() || undefined : undefined;
  createEffect(() => [active(), matches()] as const, ([index]) => {
    if (index >= 0) document.getElementById(optionId(index))?.scrollIntoView({ block: "nearest" });
  });

  function show() {
    setQuery("");
    setCursor(Math.max(0, props.options.findIndex(item => item.value === props.value)));
    setOpen(true);
    dialog.showModal();
    search.focus();
  }

  function choose(value: string, query?: string) {
    dialog.close();
    props.onSelect(value, query);
  }

  function navigate(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      setCursor(Math.max(0, Math.min(matches().length - 1, active() + (event.key === "ArrowDown" ? 1 : -1))));
    } else if (event.key === "Enter") {
      event.preventDefault();
      const item = matches()[active()];
      if (item) choose(item.value);
    }
  }

  return <Show when={props.action || props.options.length > 8} fallback={
    <select id={props.id} class="gw-control" aria-describedby={props["aria-describedby"]}
      value={props.value} disabled={props.disabled} onChange={event => {
        props.onSelect(event.currentTarget.value);
        event.currentTarget.value = props.value;
      }}>
      <For each={props.options}>{item => <option value={item.value} selected={item.value === props.value}>{item.label}</option>}</For>
    </select>
  }>
    <button ref={trigger} type="button" id={props.id} class={["gw-control", styles.trigger]} disabled={props.disabled}
      title={[selected()?.label ?? props.value, selected()?.detail].filter(Boolean).join(" · ")}
      aria-label={`${props.title} · ${selected()?.label ?? props.value}`} aria-describedby={props["aria-describedby"]}
      aria-haspopup="dialog" aria-expanded={open() ? "true" : "false"} onClick={show}>
      <span>{selected()?.label ?? props.value}</span><Icon name="chevronDown" size={16} />
    </button>
    <dialog ref={dialog} class={styles.dialog} aria-labelledby={`${listId}-title`}
      onClose={() => {
        if (dialog.open) return;
        setOpen(false);
        trigger.focus({ preventScroll: true });
      }} onClick={event => { if (event.target === dialog) dialog.close(); }}>
      <div class={styles.content}>
        <header class={styles.header}>
          <h2 id={`${listId}-title`}>{props.title}</h2>
          <button class={styles.close} type="button" aria-label={props.t.close} onClick={() => dialog.close()}><Icon name="close" size={18} /></button>
        </header>
        <div class={styles.search}>
          <Icon name="search" size={18} />
          <input ref={search} role="combobox" aria-label={props.searchHint ?? props.t.searchOptions}
            aria-expanded={open() ? "true" : "false"} aria-controls={listId} aria-autocomplete="list"
            aria-activedescendant={active() >= 0 ? optionId(active()) : undefined}
            placeholder={props.searchHint ?? props.t.searchOptions} autocomplete="off" spellcheck={false}
            value={query()} onInput={event => { setQuery(event.currentTarget.value); setCursor(0); }} onKeyDown={navigate} />
        </div>
        <div class={styles.results}>
          <ul class={styles.list} role="listbox" id={listId} aria-label={props.title}>
            <For each={matches()} keyed={item => item.value}>{(item, index) =>
              <li role="option" id={optionId(index())} aria-selected={props.value === item().value ? "true" : "false"}
                data-active={index() === active() ? "true" : undefined} onMouseDown={event => event.preventDefault()} onClick={() => choose(item().value)}>
                <span><strong>{item().label}</strong><Show when={item().detail}><small>{item().detail}</small></Show></span>
                <Show when={props.value === item().value}><Icon name="check" size={17} /></Show>
              </li>
            }</For>
          </ul>
          <Show when={open() && !matches().length}><p class={styles.empty} role="status">{props.t.noOptions}</p></Show>
        </div>
        <Show when={props.action}>{action => <button class={styles.action} type="button"
          onClick={() => choose(action().value, customQuery())}>{customQuery() ? action().queryLabel : action().label}<Icon name="arrowRight" size={16} /></button>}</Show>
      </div>
    </dialog>
  </Show>;
}
