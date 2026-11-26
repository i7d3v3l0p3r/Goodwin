import { createSignal, Show } from "solid-js";
import { usePanel } from "../helpers/panel";
import { errorLabel } from "../helpers/words";
import { Button } from "./ui-kit/Button";
import { Field } from "./ui-kit/Field";
import { InlineError } from "./ui-kit/Feedback";

export function PanelPasswordForm(props: {
  mode: "login" | "setup" | "enable";
  onSuccess?: () => void | Promise<void>;
  onSkip?: () => void | Promise<void>;
}) {
  const panel = usePanel();
  const create = () => props.mode !== "login";
  const [password, setPassword] = createSignal("");
  const [confirmation, setConfirmation] = createSignal("");
  const [validation, setValidation] = createSignal("");
  const code = () => validation() || panel.error();
  const passwordIssue = (error: string) => error === "password_required" || error === "invalid_password" || error === "wrong_password";
  const confirmationIssue = (error: string) => error === "password_mismatch";
  let passwordInput!: HTMLInputElement;
  let confirmationInput: HTMLInputElement | undefined;

  function focusError(error: string) {
    if (passwordIssue(error)) passwordInput.focus();
    else if (confirmationIssue(error)) confirmationInput?.focus();
  }

  function clearError() {
    setValidation("");
    panel.clearError();
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (panel.pending()) return;
    clearError();
    let error = password() ? "" : "password_required";
    if (!error && create()) {
      const characters = Array.from(password()).length;
      if (characters < 8 || characters > 256) error = "invalid_password";
      else if (password() !== confirmation()) error = "password_mismatch";
    }
    if (error) {
      setValidation(error);
      focusError(error);
      return;
    }
    const mode = props.mode;
    const ok = mode === "login" ? await panel.login(password()) : await panel.setupPassword(password(), confirmation(), mode);
    if (ok) await props.onSuccess?.();
    else focusError(panel.error());
  }

  return <form class="gw-stack" novalidate onSubmit={event => void submit(event)}>
    <Field label={panel.t().password} hint={create() && code() !== "invalid_password" ? panel.t().passwordHint : undefined}
      error={passwordIssue(code()) ? errorLabel(panel.t(), code()) : undefined}>
      {control => <input {...control} ref={passwordInput} required name="password" type="password"
        autocomplete={create() ? "new-password" : "current-password"} maxlength={512} value={password()}
        onInput={event => { setPassword(event.currentTarget.value); clearError(); }} />}
    </Field>
    <Show when={create()}>
      <Field label={panel.t().confirmPassword} error={confirmationIssue(code()) ? errorLabel(panel.t(), code()) : undefined}>
        {control => <input {...control} ref={node => { confirmationInput = node; }} required name="confirmation"
          type="password" autocomplete="new-password" maxlength={512} value={confirmation()}
          onInput={event => { setConfirmation(event.currentTarget.value); clearError(); }} />}
      </Field>
    </Show>
    <Button variant="primary" type="submit" disabled={panel.pending()}>
      {panel.pending() ? panel.t().saving : create() ? panel.t().createPassword : panel.t().login}
    </Button>
    <Show when={props.onSkip}>
      <Button type="button" disabled={panel.pending()} onClick={() => {
        clearError();
        void props.onSkip?.();
      }}>{panel.t().continueWithoutPassword}</Button>
    </Show>
    <Show when={code() && !passwordIssue(code()) && !confirmationIssue(code())}>
      <InlineError>{errorLabel(panel.t(), code())}</InlineError>
    </Show>
  </form>;
}
