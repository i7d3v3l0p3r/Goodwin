import { createRouter } from "@solidjs/router";
import { ChatPage } from "./pages/chat/ChatPage";
import { SettingsPage } from "./pages/settings/SettingsPage";
import { settingsPath, settingsSectionPath, settingsSections } from "./pages/settings/sections";
import { PluginsPage } from "./pages/plugins/PluginsPage";
import { ExtensionPage } from "./pages/plugins/ExtensionPage";
import { PluginSettingsRoute } from "./pages/plugins/PluginSettingsPage";

export const chatPath = "/";
export const pluginsPath = "/plugins";

export const PanelRouter = createRouter({
  routes: [
    { path: chatPath, component: ChatPage },
    // Sharing the route keeps settings drafts mounted while switching sections.
    { path: [settingsPath, ...settingsSections.map(section => settingsSectionPath(section.id))], component: SettingsPage },
    { path: pluginsPath, component: PluginsPage },
    { path: "/plugins/:id/settings", component: PluginSettingsRoute },
    { path: ["/plugins/:id", "/plugins/:id/*rest"], component: ExtensionPage },
  ],
});

