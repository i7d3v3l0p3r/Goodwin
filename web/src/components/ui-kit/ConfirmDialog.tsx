import { createEffect, createUniqueId } from "solid-js";
import { Button } from "./Button";
import { InlineError } from "./Feedback";
import styles from "./ConfirmDialog.module.css";

export function ConfirmDialog(props: {
  open: boolean;
  title: string;
  description: string;
  cancel: string;
  confirm: string;
  disabled?: boolean;
  busy?: boolean;
  error?: string;
  onCancel: () => void;
  onConfirm: () => void;
  onClose?: () => void;
}) {
  const descriptionId = createUniqueId();
  let dialog!: HTMLDialogElement;
  let cancelButton!: HTMLButtonElement;

  createEffect(() => props.open, open => {
    if (open && !dialog.open) dialog.showModal();
    else if (!open && dialog.open) dialog.close();
  });
  createEffect(() => props.busy, (busy, wasBusy) => {
    if (wasBusy && !busy && props.open && dialog.open) cancelButton.focus();
  });

  function closed() {
    if (dialog.open) return;
    if (props.open && props.busy) {
      dialog.showModal();
      return;
    }
    if (props.open) props.onCancel();
    props.onClose?.();
  }

  return <dialog ref={dialog} class={styles.dialog} aria-label={props.title} aria-describedby={descriptionId}
    onClose={closed} onKeyDown={event => {
      if (props.busy && event.key === "Escape") event.preventDefault();
    }} onCancel={event => {
      event.preventDefault();
      if (!props.busy) props.onCancel();
    }}>
    <h2>{props.title}</h2>
    <p id={descriptionId}>{props.description}</p>
    {props.error && <InlineError>{props.error}</InlineError>}
    <div class={styles.actions}>
      <Button ref={cancelButton} type="button" autofocus disabled={props.busy} onClick={props.onCancel}>{props.cancel}</Button>
      <Button type="button" variant="primary" class={styles.danger} disabled={props.disabled || props.busy}
        onClick={props.onConfirm}>{props.confirm}</Button>
    </div>
  </dialog>;
}
