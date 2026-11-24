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
  microphone_muted: boolean;
  sensitivity: number;
  wake_gain: number | null;
  transcription_gain: number | null;
}

export interface WakeModel {
  id: string;
  wake_word: string;
  tier: "standard" | "micro";
  runtime?: "livekit-v2-standard-onnx" | "livekit-v2-micro-onnx" | "livekit-v2-micro-tflite";
  sha256: string;
}

export interface WakeModelCatalog {
  tag: string;
  models: WakeModel[];
}

export interface Report {
  stage: Stage;
  source: "chat" | "wake" | "manual_test" | "wake_test";
  timings: { stage: Stage; ms: number }[];
  error: string | null;
  message_id?: number;
}

export interface VoiceReport extends Report {
  started_at_ms: number;
  transcription: VoiceModel;
  chat: VoiceModel;
  speech: VoiceModel;
  transcript: string | null;
  answer: string | null;
  playback: "streaming" | "buffered" | "announcement" | null;
  error_stage: Stage | null;
}

interface VoiceModel {
  provider: string;
  model: string | null;
}

export interface Message {
  id: number;
  role: "user" | "assistant";
  text: string;
  error?: string;
}

export interface Snapshot {
  settings: Settings;
  system_prompt: string;
  microphone: boolean;
  default_microphone_gain: number;
  model: boolean;
  wake_model: WakeModel | null;
  active: Report | null;
  last: Report | null;
  last_voice: VoiceReport | null;
  last_voice_test?: VoiceReport | null;
  messages: Message[];
  plugins: Plugin[];
  chat_draft?: Message;
}

export type Label = Record<"en" | "ru", string>;

export interface PluginPage {
  id: string;
  path: string;
  title: Label;
  navigation: boolean;
  icon?: string;
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
  settings_redirect?: string;
  connection_settings?: boolean;
  connections?: ProviderConnection[];
}

export interface ProviderConnection {
  id: string;
  providerId: string;
  name: string;
  protocol: "openai" | "openrouter";
  baseUrl: string;
  hasKey: boolean;
  speechStreaming: boolean;
  revision: number;
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

export async function api<T>(route: string, method = "GET", body?: unknown, timeout?: number): Promise<T> {
  let response: Response;
  const archive = body instanceof File;
  try {
    response = await fetch(`/api/${route}`, {
      method,
      headers: body === undefined ? undefined : { "Content-Type": archive ? "application/gzip" : "application/json" },
      body: archive ? body : body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(timeout ?? (archive ? 60_000 : 10_000)),
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
