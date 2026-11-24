import { SymbolMark } from "../../../components/Brand";
import type { Snapshot } from "../../../helpers/api";
import type { Words } from "../../../helpers/words";
import styles from "./Conversation.module.css";

export function EmptyConversation(props: { t: Words; snapshot?: Snapshot; wakeReady: boolean; setupRequired: boolean }) {
  return <div class={styles.emptyChat}>
    <SymbolMark class={styles.emptyMark} />
    <h2>{props.setupRequired ? props.t.chatSetupTitle : props.t.empty}</h2>
    <p>{props.setupRequired ? props.t.firstRun.chatUnavailable
      : props.wakeReady ? props.t.sayOrWrite(props.snapshot?.wake_model?.wake_word ?? props.t.defaultWakeWord) : props.t.write}</p>
  </div>;
}
