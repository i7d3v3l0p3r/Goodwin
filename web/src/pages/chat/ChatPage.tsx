import { Show } from "solid-js";
import { Button } from "../../components/Button";
import { Icon } from "../../components/icons";
import { PageHeader } from "../../components/PageHeader";
import { usePanel } from "../../helpers/panel";
import { Conversation } from "./components/Conversation";
import styles from "./ChatPage.module.css";

export function ChatPage() {
  const panel = usePanel();
  const disabled = () => panel.busy() || !panel.reachable();
  const error = () => {
    const last = panel.snapshot()?.last;
    return panel.error() || (last?.source === "chat" ? last.error ?? "" : "");
  };

  return <>
    <PageHeader title={panel.t().chat}>
      <Show when={panel.snapshot()?.messages.length}>
        <Button type="button" class={styles.newChat} aria-label={panel.t().newChat} disabled={disabled()}
          onClick={() => void panel.clearChat()}>
          <Icon name="plus" size={17} /><span>{panel.t().newChat}</span>
        </Button>
      </Show>
    </PageHeader>
    <Conversation t={panel.t()} snapshot={panel.snapshot()} disabled={disabled()}
      error={error()} send={panel.send} />
  </>;
}
