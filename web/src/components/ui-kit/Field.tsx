import type { JSX } from "@solidjs/web/jsx-runtime";
import { createUniqueId, Show } from "solid-js";

type ControlAttributes = {
  id: string;
  class: "gw-control";
  "aria-label": string | undefined;
  "aria-describedby": string | undefined;
  "aria-invalid": "true" | undefined;
};

export function Field(props: {
  id?: string;
  class?: string;
  label: string;
  hideLabel?: boolean;
  hint?: string;
  error?: string;
  valueLabel?: string;
  children: (control: ControlAttributes) => JSX.Element;
}) {
  const id = props.id ?? createUniqueId();
  const hintId = `${id}-hint`;
  const errorId = `${id}-error`;
  const control: ControlAttributes = {
    id,
    class: "gw-control",
    get "aria-label"() { return props.hideLabel ? props.label : undefined; },
    get "aria-describedby"() {
      return [props.hint && hintId, props.error && errorId].filter(Boolean).join(" ") || undefined;
    },
    get "aria-invalid"() { return props.error ? "true" : undefined; },
  };

  return <div class={["gw-field", props.class]}>
    <Show when={!props.hideLabel}>
      <div class="gw-field-heading">
        <label class="gw-field-label" for={id}>{props.label}</label>
        <Show when={props.valueLabel}><output class="gw-field-value" aria-hidden="true">{props.valueLabel}</output></Show>
      </div>
    </Show>
    {props.children(control)}
    <Show when={props.hint}><p class="gw-field-hint" id={hintId}>{props.hint}</p></Show>
    <Show when={props.error}><p class="gw-field-error" id={errorId} role="alert">{props.error}</p></Show>
  </div>;
}
