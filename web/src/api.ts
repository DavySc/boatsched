export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

let onUnauthorized: (() => void) | null = null;

/** Called when any request other than login answers 401 (session expired). */
export function setUnauthorizedHandler(fn: (() => void) | null) {
  onUnauthorized = fn;
}

interface Options {
  method?: string;
  json?: unknown;
  form?: FormData;
}

export async function api<T = unknown>(path: string, opts: Options = {}): Promise<T> {
  const init: RequestInit = { method: opts.method ?? (opts.json || opts.form ? "POST" : "GET") };
  if (opts.json !== undefined) {
    init.headers = { "Content-Type": "application/json" };
    init.body = JSON.stringify(opts.json);
  } else if (opts.form) {
    init.body = opts.form;
  }
  let res: Response;
  try {
    res = await fetch(path, init);
  } catch {
    throw new ApiError("Verbindingsfout. Probeer opnieuw.", 0);
  }
  let body: unknown = null;
  try {
    body = await res.json();
  } catch {
    // non-JSON body (e.g. CSV); callers that need it use fetch directly
  }
  if (!res.ok) {
    if (res.status === 401 && path !== "/api/auth/login") onUnauthorized?.();
    const b = body as { error?: string; detail?: string } | null;
    throw new ApiError(b?.error ?? b?.detail ?? res.statusText, res.status);
  }
  return body as T;
}

export const errMsg = (e: unknown) => (e instanceof Error ? e.message : String(e));
