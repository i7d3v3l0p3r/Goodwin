import { createSignal, Show } from "solid-js";
import { Brand } from "../../components/Brand";
import { Button } from "../../components/Button";
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

  return <>
    <p>{props.setup ? panel.t().setupAccount : panel.t().account}</p>
    <form onSubmit={(event) => {
      event.preventDefault();
      if (props.setup) void panel.setupPassword(password(), confirmation());
      else void panel.login(password());
    }}>
      <label>
        {panel.t().password}
        <input required name="password" type="password" autocomplete={props.setup ? "new-password" : "current-password"}
          maxlength={256} value={password()} onInput={(event) => setPassword(event.currentTarget.value)} />
      </label>
      <Show when={props.setup}>
        <label>
          {panel.t().confirmPassword}
          <input required name="confirmation" type="password" autocomplete="new-password" maxlength={256}
            value={confirmation()} onInput={(event) => setConfirmation(event.currentTarget.value)} />
        </label>
      </Show>
      <Button variant="primary" type="submit" disabled={panel.pending()}>
        {props.setup ? panel.t().createPassword : panel.t().login}
      </Button>
      <Show when={panel.error()}><p class={styles.error} role="alert">{errorLabel(panel.t(), panel.error())}</p></Show>
    </form>
  </>;
}
