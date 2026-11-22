import type { Stage } from "./words";

export type Models = Record<"transcription" | "chat" | "speech" | "voice", string>;

export interface Settings {
  models: Models;
  listening: boolean;
  sensitivity: number;
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
  key_configured: boolean;
  microphone: boolean;
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
}

export interface Access {
  protected: boolean;
  authorized: boolean;
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
