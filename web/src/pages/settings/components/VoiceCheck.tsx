import { For, Show } from "solid-js";
import { Button } from "../../../components/Button";
import type { Report, Snapshot } from "../../../helpers/api";
import { errorLabel, type Words } from "../../../helpers/words";
import styles from "./VoiceCheck.module.css";

function RequestDetails(props: { t: Words; report: Report | null }) {
  return <div class={styles.details}>
    <h3>{props.t.last}</h3>
    <Show when={props.report} fallback={<p class={styles.note}>{props.t.noRequest}</p>}>
      {(report) => <>
        <p class={report().error ? styles.error : styles.note}>
          {report().error ? errorLabel(props.t, report().error!) : props.t.completed}
        </p>
        <dl>
          <For each={report().timings}>{(timing) => <>
            <dt>{props.t.stages[timing.stage]}</dt>
            <dd>{timing.ms} {props.t.ms}</dd>
          </>}</For>
        </dl>
      </>}
    </Show>
  </div>;
}

export function VoiceCheck(props: {
  t: Words;
  snapshot: Snapshot;
  disabled: boolean;
  test: (mode: "manual" | "wake") => Promise<boolean>;
}) {
  return <section aria-labelledby="test-title">
    <h2 id="test-title">{props.t.voiceCheck}</h2>
    <p class={styles.note}>{props.t.testHint}</p>
    <div class={styles.actions}>
      <Button type="button" disabled={props.disabled || !props.snapshot.microphone}
        onClick={() => void props.test("manual")}>{props.t.manual}</Button>
      <Button type="button" disabled={props.disabled || !props.snapshot.microphone || !props.snapshot.model}
        onClick={() => void props.test("wake")}>{props.t.byWake}</Button>
    </div>
    <RequestDetails t={props.t} report={props.snapshot.last} />
  </section>;
}

