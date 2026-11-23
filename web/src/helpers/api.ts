import type { Stage } from "./words";

export type Capability = "transcription" | "chat" | "speech";
export interface ModelSelection {
  model?: string;
  parameters?: Record<string, unknown>;
}

export interface PipelineStage {
  provider: string;
  selection: ModelSelection;
}

export interface AssistantSettings {
  engine: "pipeline";
  pipeline: Record<Capability, PipelineStage> & { speech_streaming?: boolean };
}

export interface ModelCatalog {
  models: { id: string; name: string | null; voices?: string[] }[];
  default: string | null;
  available: boolean;
}

export interface Settings {
  assistant: AssistantSettings;
  listening: boolean;
  sensitivity: number;
  wake_gain: number | null;
  transcription_gain: number | null;
}

export interface Report {
  stage: Stage;
  source: "chat" | "wake" | "manual_test" | "wake_test";
  timings: { stage: Stage; ms: number }[];
  error: string | null;
}

export interface Message {
  id: number;
  role: "user" | "assistant";
  text: string;
}

export interface Snapshot {
  settings: Settings;
  system_prompt: string;
  microphone: boolean;
  default_microphone_gain: number;
  model: boolean;
  active: Report | null;
  last: Report | null;
  messages: Message[];
  plugins: Plugin[];
}

export type Label = Record<"en" | "ru", string>;

export interface PluginPage {
  id: string;
  path: string;
  title: Label;
  navigation: boolean;
}

export interface Plugin {
  id: string;
  title: Label;
  version: string;
  status: "starting" | "ready" | "updating" | "removing" | "error";
  error: string | null;
  revision: string | null;
  web: { entry: string; styles: string[] } | null;
  pages: PluginPage[];
  providers: Capability[];
  model_catalog: boolean;
  speech_streaming?: boolean;
  has_settings: boolean;
}

export interface PluginSettingField {
  id: string;
  label: Label;
  description?: Label;
  type: "text" | "url" | "secret" | "boolean" | "integer";
  required: boolean;
  maxLength?: number;
}

export interface PluginSettings {
  fields: PluginSettingField[];
  values: Record<string, string | boolean | number>;
  configuredSecrets: string[];
}

export interface Access {
  protected: boolean;
  authorized: boolean;
  setup_required: boolean;
}

export class ApiError extends Error {
  constructor(public code: string) { super(code); }
}

export async function api<T>(route: string, method = "GET", body?: unknown): Promise<T> {
  let response: Response;
  const archive = body instanceof File;
  try {
    response = await fetch(`/api/${route}`, {
      method,
      headers: body === undefined ? undefined : { "Content-Type": archive ? "application/gzip" : "application/json" },
      body: archive ? body : body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(archive ? 60_000 : 10_000),
    });
  } catch {
    throw new ApiError("connection");
  }

  if (!response.ok) {
    const problem = await response.json().catch(() => null);
    throw new ApiError(problem?.error ?? "internal");
  }
  if (response.headers.get("content-type")?.includes("application/json")) return response.json();
  return undefined as T;
}
