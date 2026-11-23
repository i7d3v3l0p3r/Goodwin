import { createUniqueId, Show } from "solid-js";

export function Switch(props: {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  const id = createUniqueId();
  const hintId = `${id}-hint`;

  return <div class="gw-field">
    <div class="gw-field-heading">
      <label class="gw-field-label" for={id} id={`${id}-label`}>{props.label}</label>
      <button type="button" class="gw-switch" id={id} role="switch" aria-labelledby={`${id}-label`}
        aria-describedby={props.hint ? hintId : undefined} aria-checked={props.checked ? "true" : "false"}
        disabled={props.disabled} onClick={() => props.onChange(!props.checked)}>
        <span class="gw-switch-track" aria-hidden="true" />
      </button>
    </div>
    <Show when={props.hint}><p class="gw-field-hint" id={hintId}>{props.hint}</p></Show>
  </div>;
}
