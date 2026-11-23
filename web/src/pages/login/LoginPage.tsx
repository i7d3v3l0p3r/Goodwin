import { createSignal, Show } from "solid-js";
import { Brand } from "../../components/Brand";
import { Button } from "../../components/ui-kit/Button";
import { Field } from "../../components/ui-kit/Field";
import { InlineError } from "../../components/ui-kit/Feedback";
import { usePanel } from "../../helpers/panel";
import { errorLabel } from "../../helpers/words";
import styles from "./LoginPage.module.css";

export function LoginPage() {
  const panel = usePanel();

  return <main class={styles.entry}>
    <Brand class={styles.brand} />
    <Show when={panel.access()?.protected} fallback={<p role="status">{panel.status()}</p>}>
      <Show when={panel.access()?.setup_required} fallback={<PasswordForm setup={false} />}>
        <PasswordForm setup />
      </Show>
    </Show>
  </main>;
}

function PasswordForm(props: { setup: boolean }) {
  const panel = usePanel();
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
    if (!error && props.setup) {
      const bytes = new TextEncoder().encode(password()).length;
      if (bytes < 8 || bytes > 256) error = "invalid_password";
      else if (password() !== confirmation()) error = "password_mismatch";
    }
    if (error) {
      setValidation(error);
      focusError(error);
      return;
    }
    const ok = props.setup ? await panel.setupPassword(password(), confirmation()) : await panel.login(password());
    if (!ok) focusError(panel.error());
  }

  return <>
    <h1>{props.setup ? panel.t().setupTitle : panel.t().login}</h1>
    <p class={styles.note}>{props.setup ? panel.t().setupAccount : panel.t().account}</p>
    <form novalidate onSubmit={event => void submit(event)}>
      <Field label={panel.t().password} hint={props.setup && code() !== "invalid_password" ? panel.t().passwordHint : undefined}
        error={passwordIssue(code()) ? errorLabel(panel.t(), code()) : undefined}>
        {control => <input {...control} ref={passwordInput} required name="password" type="password"
          autocomplete={props.setup ? "new-password" : "current-password"} maxlength={256} value={password()}
          onInput={event => { setPassword(event.currentTarget.value); clearError(); }} />}
      </Field>
      <Show when={props.setup}>
        <Field label={panel.t().confirmPassword} error={confirmationIssue(code()) ? errorLabel(panel.t(), code()) : undefined}>
          {control => <input {...control} ref={node => { confirmationInput = node; }} required name="confirmation"
            type="password" autocomplete="new-password" maxlength={256} value={confirmation()}
            onInput={event => { setConfirmation(event.currentTarget.value); clearError(); }} />}
        </Field>
      </Show>
      <Button variant="primary" type="submit" disabled={panel.pending()}>
        {props.setup ? panel.t().createPassword : panel.t().login}
      </Button>
      <Show when={code() && !passwordIssue(code()) && !confirmationIssue(code())}>
        <InlineError>{errorLabel(panel.t(), code())}</InlineError>
      </Show>
    </form>
  </>;
}
