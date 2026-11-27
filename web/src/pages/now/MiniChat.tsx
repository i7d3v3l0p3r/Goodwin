import { createSignal, Show } from "solid-js";
import { Button } from "../../components/ui-kit/Button";
import { ConfirmDialog } from "../../components/ui-kit/ConfirmDialog";
import { Switch } from "../../components/ui-kit/Switch";
import { Icon } from "../../components/icons";
import { usePanel } from "../../helpers/panel";
import { assistantPath, setupPath } from "../../helpers/readiness";
import { Conversation } from "./components/Conversation";
import styles from "./NowPage.module.css";

export function MiniChat() {
  const panel = usePanel();
  const preferenceKey = "goodwin-chat-system-messages";
  const [showSystem, setShowSystem] = createSignal(localStorage.getItem(preferenceKey) !== "false");
  const [confirming, setConfirming] = createSignal(false);
  const disabled = () => panel.busy() || !panel.reachable();
  const hasHistory = () => Boolean(panel.snapshot()?.messages.length || panel.snapshot()?.system_events?.length);
  return <section id="conversation" class={styles.conversation} aria-labelledby="conversation-title">
    <header class={styles.chatHeader}><h2 id="conversation-title"><Icon name="chat" size={19} />{panel.t().chat}</h2>
      <div class={styles.chatTools}><Switch label={panel.t().now.systemMessages} checked={showSystem()} onChange={show => {
        setShowSystem(show);
        try { localStorage.setItem(preferenceKey, String(show)); } catch { /* Keep the current preference in memory. */ }
      }} />
      <Show when={hasHistory()}>
        <Button type="button" class={styles.newChat} variant="quiet" title={panel.t().newChat} aria-label={panel.t().newChat} disabled={disabled()}
          onClick={() => setConfirming(true)}>
          <Icon name="plus" size={17} /><span>{panel.t().newChat}</span>
        </Button>
      </Show></div>
    </header>
    <Conversation t={panel.t()} showSystem={showSystem()} snapshot={panel.snapshot()} draftReply={panel.chatDraft()} reachable={panel.reachable()} disabled={disabled()}
      draft={panel.draftMessage()} editDraft={panel.setDraftMessage} wakeReady={panel.wakeReady()} setupRequired={Boolean(panel.stageIssue("chat"))}
      setupHref={panel.builtinProvider() ? assistantPath : setupPath} error={panel.error()} send={panel.send} />
    <ConfirmDialog open={confirming()} title={panel.t().clearChat} description={panel.t().clearChatDescription}
      cancel={panel.t().cancel} confirm={panel.t().clearChat} disabled={disabled() || !hasHistory()}
      onCancel={() => setConfirming(false)} onConfirm={() => {
        setConfirming(false);
        void panel.clearChat();
      }} />
  </section>;
}
