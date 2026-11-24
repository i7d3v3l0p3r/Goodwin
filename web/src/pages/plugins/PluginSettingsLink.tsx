import type { Plugin } from "../../helpers/api";
import { ButtonLink } from "../../components/ui-kit/Button";
import { usePanel } from "../../helpers/panel";
import { pluginSettingsPath } from "../../helpers/plugins";

export function PluginSettingsLink(props: { plugin: Plugin; quiet?: boolean }) {
  const panel = usePanel();
  return <ButtonLink variant={props.quiet ? "quiet" : "secondary"} href={pluginSettingsPath(props.plugin.settings_redirect ?? props.plugin.id)}
    aria-label={`${panel.t().plugins.configure} · ${props.plugin.title[panel.language()]}`}>
    {panel.t().plugins.configure}
  </ButtonLink>;
}
