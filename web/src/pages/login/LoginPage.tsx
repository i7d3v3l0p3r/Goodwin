import { createSignal, Show } from "solid-js";
import { Brand } from "../../components/Brand";
import { Button } from "../../components/Button";
import { usePanel } from "../../helpers/panel";
import { errorLabel } from "../../helpers/words";
import styles from "./LoginPage.module.css";

export function LoginPage() {
  const panel = usePanel();
  const [password, setPassword] = createSignal("");

  return <main class={styles.entry}>
    <Brand class={styles.brand} />
    <Show when={panel.access()?.protected} fallback={<p role="status">{panel.status()}</p>}>
      <p>{panel.t().account}</p>
      <form onSubmit={(event) => {
        event.preventDefault();
        void panel.login(password());
      }}>
        <label>
          {panel.t().password}
          <input required type="password" autocomplete="current-password" maxlength={256}
            value={password()} onInput={(event) => setPassword(event.currentTarget.value)} />
        </label>
        <Button variant="primary" type="submit" disabled={panel.pending()}>{panel.t().login}</Button>
        <Show when={panel.error()}><p class={styles.error} role="alert">{errorLabel(panel.t(), panel.error())}</p></Show>
      </form>
    </Show>
  </main>;
}
