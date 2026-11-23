import type { Plugin } from "../../helpers/api";
import { usePanel } from "../../helpers/panel";
import { pluginSettingsPath } from "../../helpers/plugins";
import styles from "./PluginSettingsPage.module.css";

export function PluginSettingsLink(props: { plugin: Plugin }) {
  const panel = usePanel();
  return <a class={styles.link} href={pluginSettingsPath(props.plugin.id)}
    aria-label={`${panel.t().settings} · ${props.plugin.title[panel.language()]}`}>
    {panel.t().settings}
  </a>;
}
