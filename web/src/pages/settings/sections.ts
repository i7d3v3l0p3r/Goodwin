export const settingsPath = "/settings";

export const settingsSections = [
  { id: "activation", title: "activation", description: "activationDescription", icon: "mic" },
  { id: "assistant", title: "assistant", description: "assistantDescription", icon: "sparkles" },
  { id: "diagnostics", title: "voiceCheck", description: "diagnosticsDescription", icon: "activity" },
  { id: "panel", title: "panelPreferences", description: "panelPreferencesDescription", icon: "monitor" },
] as const;

export function settingsSectionPath(section: typeof settingsSections[number]["id"]) {
  return `${settingsPath}/${section}` as const;
}

export function isSettingsPath(pathname: string) {
  const path = pathname.replace(/\/$/, "");
  return path === settingsPath || settingsSections.some(section => path === settingsSectionPath(section.id));
}
