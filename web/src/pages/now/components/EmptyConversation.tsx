import { Show } from "solid-js";
import { Icon } from "../../../components/icons";
import type { Snapshot } from "../../../helpers/api";
import type { Words } from "../../../helpers/words";
import styles from "./Conversation.module.css";

export function EmptyConversation(props: {
  t: Words; snapshot?: Snapshot; wakeReady: boolean; setupRequired: boolean;
  showExamples: boolean; choose: (text: string) => void;
}) {
  return <div class={styles.emptyChat}>
    <p>{props.setupRequired ? props.t.firstRun.chatUnavailable
      : props.wakeReady ? props.t.sayOrWrite(props.snapshot?.wake_model?.wake_word ?? props.t.defaultWakeWord) : props.t.write}</p>
    <Show when={props.showExamples}>
      <div class={styles.examples}>
        <button type="button" onClick={() => props.choose(props.t.chatExamples.dinner)}>
          {props.t.chatExamples.dinner}<Icon name="arrowRight" size={15} />
        </button>
        <button type="button" onClick={() => props.choose(props.t.chatExamples.sunset)}>
          {props.t.chatExamples.sunset}<Icon name="arrowRight" size={15} />
        </button>
      </div>
    </Show>
  </div>;
}
