import { createSignal, Show } from "solid-js";
import { Button } from "../../components/ui-kit/Button";
import { ConfirmDialog } from "../../components/ui-kit/ConfirmDialog";
import { Icon } from "../../components/icons";
import { PageHeader } from "../../components/PageHeader";
import { usePanel } from "../../helpers/panel";
import { Conversation } from "./components/Conversation";
import styles from "./ChatPage.module.css";

export function ChatPage() {
  const panel = usePanel();
  const [confirming, setConfirming] = createSignal(false);
  const disabled = () => panel.busy() || !panel.reachable();
  return <>
    <PageHeader title={panel.t().chat} showRequestStatus={panel.snapshot()?.active?.source !== "chat"}>
      <Show when={panel.snapshot()?.messages.length}>
        <Button type="button" class={styles.newChat} aria-label={panel.t().newChat} disabled={disabled()}
          onClick={() => setConfirming(true)}>
          <Icon name="plus" size={17} /><span>{panel.t().newChat}</span>
        </Button>
      </Show>
    </PageHeader>
    <Conversation t={panel.t()} snapshot={panel.snapshot()} draftReply={panel.chatDraft()} reachable={panel.reachable()} disabled={disabled()}
      error={panel.error()} send={panel.send} />
    <ConfirmDialog open={confirming()} title={panel.t().clearChat} description={panel.t().clearChatDescription}
      cancel={panel.t().cancel} confirm={panel.t().clearChat} disabled={disabled() || !panel.snapshot()?.messages.length}
      onCancel={() => setConfirming(false)} onConfirm={() => {
        setConfirming(false);
        void panel.clearChat();
      }} />
  </>;
}
