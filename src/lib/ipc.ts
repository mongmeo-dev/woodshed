import { invoke } from "@tauri-apps/api/core";

/** True inside the Tauri webview; false in the browser dev server and tests. */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Invokes a Tauri command, or resolves `fallback` outside Tauri. */
export async function call<T>(
  command: string,
  args: Record<string, unknown>,
  fallback: T,
): Promise<T> {
  if (!inTauri()) return fallback;
  return invoke<T>(command, args);
}
