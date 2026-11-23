import { Button } from "./Button";

export function SaveActions(props: {
  labels: { save: string; saving: string; saved: string; unsaved: string };
  label?: string;
  disabled?: boolean;
  dirty: boolean;
  saving: boolean;
  saved: boolean;
}) {
  const success = () => props.saved && !props.dirty && !props.saving;
  const status = () => {
    if (props.saving) return props.labels.saving;
    if (props.dirty) return props.labels.unsaved;
    if (success()) return props.labels.saved;
    return "";
  };

  return <div class="gw-save-actions">
    <Button variant="primary" type="submit" disabled={props.disabled || props.saving}>{props.label ?? props.labels.save}</Button>
    <span class="gw-save-status" role="status" data-saved={success() ? "true" : undefined}>{status()}</span>
  </div>;
}
