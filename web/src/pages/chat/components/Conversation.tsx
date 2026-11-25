import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { SymbolMark } from "../../../components/Brand";
import { Button } from "../../../components/ui-kit/Button";
import { InlineError, StatusIndicator } from "../../../components/ui-kit/Feedback";
import { Icon } from "../../../components/icons";
import type { Message, Snapshot } from "../../../helpers/api";
import { errorLabel, type Words } from "../../../helpers/words";
import { EmptyConversation } from "./EmptyConversation";
import styles from "./Conversation.module.css";

function ThreadMessage(props: { entry: Message; t: Words; progress?: string; streaming: boolean }) {
  return <article class={styles.message} data-from={props.entry.role} aria-busy={props.streaming ? "true" : "false"}>
    <Show when={props.entry.role === "assistant"}><SymbolMark class={styles.messageMark} /></Show>
    <div class={styles.messageContent}>
      <div class={styles.messageMeta}>{props.entry.role === "assistant" ? "Goodwin" : props.t.you}</div>
      <p data-streaming={props.streaming ? "true" : undefined}>{props.entry.text}</p>
      <Show when={props.progress || props.entry.error}>
        <div class={styles.messageFeedback}>
          <Show when={props.progress}><StatusIndicator tone="pending">{props.progress}</StatusIndicator></Show>
          <Show when={props.entry.error}>
            {code => <InlineError announce={false}>{errorLabel(props.t, code())}</InlineError>}
          </Show>
        </div>
      </Show>
    </div>
  </article>;
}

export function Conversation(props: {
  t: Words; snapshot?: Snapshot; draftReply?: Message; reachable: boolean; disabled: boolean; error: string;
  draft: string; editDraft: (text: string) => void; wakeReady: boolean; setupRequired: boolean; setupHref: string;
  send: (text: string) => Promise<boolean>;
}) {
  const [sending, setSending] = createSignal(false);
  const [unread, setUnread] = createSignal(false);
  let scroll!: HTMLDivElement;
  let following = true;
  const messages = createMemo(() => {
    const history = props.snapshot?.messages ?? [];
    const reply = props.draftReply;
    return reply && history.at(-1)?.id !== reply.id ? [...history, reply] : history;
  });
  const lastId = createMemo(() => messages().at(-1)?.id);
  const lastError = createMemo(() => messages().at(-1)?.error);
  const request = () => props.snapshot?.active?.source === "chat" ? props.snapshot.active : undefined;
  const progress = createMemo(() => {
    const stage = request()?.stage;
    return stage && !props.draftReply ? props.t.stages[stage] : undefined;
  });
  const awaitingMessage = () => request()?.message_id === undefined;

  function followLatest() {
    following = true;
    setUnread(false);
    scroll.scrollTop = scroll.scrollHeight;
  }

  function trackScroll() {
    following = scroll.scrollHeight - scroll.clientHeight - scroll.scrollTop <= 64;
    if (following) setUnread(false);
  }

  createEffect(() => [lastId(), lastError(), props.draftReply?.text.length, progress(), sending()] as const, ([id, error, length], previous) => {
    if (id === undefined) {
      following = true;
      setUnread(false);
    }
    const updated = id !== undefined && (id !== previous?.[0] || Boolean(error && error !== previous?.[1])
      || (length !== undefined && length !== previous?.[2]));
    if (!following) {
      if (updated) setUnread(true);
      return;
    }
    const frame = requestAnimationFrame(() => {
      if (!scroll.isConnected) return;
      if (following) followLatest();
      else if (updated) setUnread(true);
    });
    return () => cancelAnimationFrame(frame);
  });

  async function send() {
    const text = props.draft.trim();
    if (!text || props.disabled || props.setupRequired || sending()) return;
    setSending(true);
    try {
      if (await props.send(text) && props.draft.trim() === text) props.editDraft("");
    } finally {
      setSending(false);
    }
  }

  return <section class={styles.chat} aria-label={props.t.chat}>
    <div class={styles.threadArea}>
      <div class={styles.threadScroll} ref={scroll} tabindex={0} onScroll={trackScroll}>
        <div class={styles.thread} role="log" aria-label={props.t.chatHistory} aria-relevant="additions">
          <Show when={messages().length} fallback={<EmptyConversation t={props.t} snapshot={props.snapshot}
            wakeReady={props.wakeReady} setupRequired={props.setupRequired && Boolean(props.snapshot) && props.reachable} />}>
            <For each={messages()} keyed={entry => entry.id}>{entry =>
              <ThreadMessage entry={entry()} t={props.t} streaming={props.draftReply?.id === entry().id}
                progress={request()?.message_id === entry().id ? progress() : undefined} />
            }</For>
          </Show>
        </div>
      </div>
      <Show when={unread()}>
        <Button class={styles.jump} type="button" onClick={() => {
          followLatest();
          scroll.focus({ preventScroll: true });
        }}>{props.t.jumpToLatest}</Button>
      </Show>
    </div>
    <div class={styles.composeArea}>
      <div class={styles.composeFeedback} role="status">
        <Show when={awaitingMessage() && (progress() || (sending() && props.t.sendingMessage))}>
          {status => <StatusIndicator tone="pending">{status()}</StatusIndicator>}
        </Show>
      </div>
      <Show when={props.error}>
        <div class={styles.composeFeedback}><InlineError>{errorLabel(props.t, props.error)}</InlineError></div>
      </Show>
      <Show when={props.setupRequired && props.snapshot && props.reachable}>
        <div class={[styles.composeFeedback, styles.setupHint]} id="chat-setup-hint">
          <a href={props.setupHref}>{props.t.firstRun.resume}</a>
        </div>
      </Show>
      <form class={styles.compose} onSubmit={(event) => { event.preventDefault(); void send(); }}>
        <textarea rows={1} maxlength={2000} aria-label={props.t.draft} placeholder={props.t.draft} value={props.draft}
          aria-describedby={props.setupRequired && props.snapshot && props.reachable ? "chat-setup-hint" : undefined}
          onInput={(event) => props.editDraft(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); void send(); }
          }} />
        <Button variant="primary" type="submit" class={styles.send} aria-label={props.t.send}
          disabled={props.disabled || props.setupRequired || sending() || !props.draft.trim()}>
          <Icon name="send" size={21} />
        </Button>
      </form>
    </div>
  </section>;
}


