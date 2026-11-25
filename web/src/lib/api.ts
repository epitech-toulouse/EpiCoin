// The only place that talks to the backend.
//
// The session lives in an HttpOnly cookie the page cannot read. The CSRF token, which the
// API hands out with the session, is kept in memory only and sent with every request that
// changes something. No key, and nothing secret, is ever stored by the page.

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly details: Record<string, unknown>;

  constructor(status: number, code: string, message: string, details: Record<string, unknown>) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

let csrfToken: string | null = null;

export function setCsrfToken(token: string | null): void {
  csrfToken = token;
}

interface Options {
  method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  /** Sent as JSON. */
  body?: unknown;
  /** Sent as is (file uploads). */
  rawBody?: Blob;
  /** Makes a retry of the same action harmless (payments, distributions). */
  idempotencyKey?: string;
}

const NETWORK_MESSAGE = 'Le serveur est injoignable. Vérifiez votre connexion et réessayez.';

export async function api<T>(path: string, options: Options = {}): Promise<T> {
  const method = options.method ?? 'GET';
  const headers: Record<string, string> = { Accept: 'application/json' };
  if (method !== 'GET' && csrfToken) headers['X-CSRF-Token'] = csrfToken;
  if (options.idempotencyKey) headers['Idempotency-Key'] = options.idempotencyKey;

  let body: BodyInit | undefined;
  if (options.rawBody) {
    headers['Content-Type'] = 'application/octet-stream';
    body = options.rawBody;
  } else if (options.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(options.body);
  }

  let response: Response;
  try {
    response = await fetch(path, { method, headers, body, credentials: 'same-origin' });
  } catch {
    throw new ApiError(0, 'network', NETWORK_MESSAGE, {});
  }

  if (response.status === 204) return undefined as T;
  const text = await response.text();
  let json: unknown = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    // Not JSON: a proxy error page, for instance.
  }

  if (!response.ok) {
    const error = (json ?? {}) as { error?: string; message?: string; details?: Record<string, unknown> };
    throw new ApiError(
      response.status,
      error.error ?? 'unknown',
      error.message ?? 'Une erreur est survenue. Réessayez plus tard.',
      error.details ?? {},
    );
  }
  const token = (json as { csrf_token?: unknown } | null)?.csrf_token;
  if (typeof token === 'string') csrfToken = token;
  return json as T;
}

/** The message to show for any thrown value. */
export function errorMessage(error: unknown): string {
  if (error instanceof ApiError) return error.message;
  return 'Une erreur inattendue est survenue.';
}

/** Builds a query string, leaving out empty values. */
export function query(params: Record<string, string | number | undefined | null>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== '') search.set(key, String(value));
  }
  const text = search.toString();
  return text ? `?${text}` : '';
}
