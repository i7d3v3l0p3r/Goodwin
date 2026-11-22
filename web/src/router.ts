import { createRouter } from "@solidjs/router";
import { ChatPage } from "./pages/chat/ChatPage";
import { SettingsPage } from "./pages/settings/SettingsPage";
import { PluginsPage } from "./pages/plugins/PluginsPage";
import { ExtensionPage } from "./pages/plugins/ExtensionPage";

export const chatPath = "/";
export const settingsPath = "/settings";
export const pluginsPath = "/plugins";

export const PanelRouter = createRouter({
  routes: [
    { path: chatPath, component: ChatPage },
    { path: settingsPath, component: SettingsPage },
    { path: pluginsPath, component: PluginsPage },
    { path: ["/plugins/:id", "/plugins/:id/*rest"], component: ExtensionPage },
  ],
});

