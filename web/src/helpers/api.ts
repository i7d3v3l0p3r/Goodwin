import type { Stage } from "./words";

export type Capability = "transcription" | "chat" | "speech";
export interface DeviceCapabilities {
  platform: string;
  device: string;
  wake_runtimes: string[];
  wake_catalog?: boolean;
  panel_updates: "events" | "poll";
  provider_configuration: "plugins" | "builtin";
  plugin_packages?: boolean;
}

export interface BuiltinProvider {
  base_url: string;
  has_key: boolean;
  stt_model: string;
  chat_model: string;
  tts_model: string;
  voice: string;
  speech_streaming: boolean;
}

export type WifiStatus = { status: "connecting" } | {
  status: "setup" | "connected";
  ssid: string;
  address: string;
  error: string | null;
};
export interface ModelSelection {
  model?: string;
  parameters?: ModelParameters;
}

export interface ModelParameters {
  temperature?: number;
  top_p?: number;
  prompt?: string;
  voice?: string;
  speed?: number;
  instructions?: string;
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
  runtime: "livekit-v2-standard-onnx" | "livekit-v2-micro-onnx" | "livekit-v2-micro-tflite";
  sha256: string;
}

export interface WakeModelCatalog {
  tag: string;
  models: WakeModel[];
}

export interface Report {
  stage: Stage;
  source: "chat" | "wake" | "manual_test" | "wake_test" | "push_to_talk";
  timings: { stage: Stage; ms: number }[];
  error: string | null;
  message_id?: number;
  chat_test?: { model: VoiceModel; answered: boolean };
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
  provider_ready?: boolean;
  voice_provider_ready?: boolean;
  settings: Settings;
  system_prompt: string;
  microphone: boolean;
  default_microphone_gain: number;
  model: boolean;
  wake_model: WakeModel | null;
  active: Report | null;
  last: Report | null;
  last_chat_test?: Report | null;
  chat_test_current?: boolean;
  last_voice: VoiceReport | null;
  last_voice_test?: VoiceReport | null;
  voice_test_current?: boolean;
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
  settings_revision?: number;
  web: { entry: string; styles: string[] } | null;
  pages: PluginPage[];
  providers: Capability[];
  model_catalog: boolean;
  speech_streaming?: boolean;
  has_settings: boolean;
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
  const binary = body instanceof Blob;
  const contentType = body instanceof File ? "application/gzip"
    : binary ? body.type || "application/octet-stream" : "application/json";
  try {
    response = await fetch(`/api/${route}`, {
      method,
      headers: body === undefined ? undefined : { "Content-Type": contentType },
      body: binary ? body : body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(timeout ?? (binary ? 60_000 : 10_000)),
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

export async function apiOperation<T>(
  route: string, method: string, body: unknown, timeout: number, resultRoute = `${route}/result`,
): Promise<T> {
  const deadline = performance.now() + timeout;
  const start = await api<T | { operation: string }>(route, method, body, timeout);
  if (typeof start !== "object" || !start || !("operation" in start)) return start as T;
  if (typeof start.operation !== "string") throw new ApiError("internal");
  const ticket = { operation: start.operation };
  let remaining: number;
  while ((remaining = deadline - performance.now()) > 0) {
    const result = await api<T | { pending: true }>(resultRoute, "POST", ticket, Math.min(10_000, Math.ceil(remaining)));
    if (typeof result !== "object" || !result || !("pending" in result)) return result as T;
    await new Promise(resolve => window.setTimeout(resolve, 750));
  }
  throw new ApiError("connection");
}
