import { createEffect, createSignal, For, Show } from "solid-js";
import { SymbolMark } from "../../../components/Brand";
import { Button } from "../../../components/Button";
import { Icon } from "../../../components/icons";
import type { Message, Snapshot } from "../../../helpers/api";
import { errorLabel, type Words } from "../../../helpers/words";
import styles from "./Conversation.module.css";

function ThreadMessage(props: { entry: Message; t: Words }) {
  return <article class={styles.message} data-from={props.entry.role}>
    <Show when={props.entry.role === "assistant"}><SymbolMark class={styles.messageMark} /></Show>
    <div class={styles.messageContent}>
      <div class={styles.messageMeta}>{props.entry.role === "assistant" ? "Goodwin" : props.t.you}</div>
      <p>{props.entry.text}</p>
    </div>
  </article>;
}

export function Conversation(props: {
  t: Words; snapshot?: Snapshot; disabled: boolean; error: string;
  send: (text: string) => Promise<boolean>;
}) {
  const [draft, setDraft] = createSignal("");
  let scroll!: HTMLDivElement;
  const lastMessage = () => props.snapshot?.messages.at(-1)?.id;
  const messages = () => props.snapshot?.messages ?? [];

  createEffect(lastMessage, () => {
    requestAnimationFrame(() => { if (scroll?.isConnected) scroll.scrollTop = scroll.scrollHeight; });
  });

  async function send() {
    const text = draft().trim();
    if (!text || props.disabled) return;
    if (await props.send(text) && draft().trim() === text) setDraft("");
  }

  return <section class={styles.chat} aria-label={props.t.chat}>
    <div class={styles.threadScroll} ref={scroll}>
      <div class={styles.thread} role="log" aria-live="polite" aria-relevant="additions">
        <Show when={messages().length} fallback={<div class={styles.emptyChat}>
          <SymbolMark class={styles.emptyMark} />
          <h2>{props.t.empty}</h2>
          <p>{props.snapshot?.model && props.snapshot.microphone && props.snapshot.settings.listening
            ? props.t.sayOrWrite(props.snapshot.wake_model?.wake_word ?? props.t.defaultWakeWord) : props.t.write}</p>
        </div>}>
          <For each={messages()} keyed={(entry) => entry.id}>{(entry) => <ThreadMessage entry={entry()} t={props.t} />}</For>
        </Show>
      </div>
    </div>
    <div class={styles.composeArea}>
      <Show when={props.error}><p class={styles.error} role="alert">{errorLabel(props.t, props.error)}</p></Show>
      <form class={styles.compose} onSubmit={(event) => { event.preventDefault(); void send(); }}>
        <textarea rows={1} maxlength={2000} aria-label={props.t.draft} placeholder={props.t.draft} value={draft()}
          onInput={(event) => setDraft(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); void send(); }
          }} />
        <Button variant="primary" type="submit" class={styles.send} aria-label={props.t.send} disabled={props.disabled || !draft().trim()}>
          <Icon name="send" size={21} />
        </Button>
      </form>
    </div>
  </section>;
}


