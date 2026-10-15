let sessionPromise: Promise<string> | undefined;

export class ApiError extends Error {
  constructor(message: string, readonly status: number) { super(message) }
}

export async function apiRequest<T>(path: string, init: RequestInit = {}): Promise<T> {
  sessionPromise ??= (async () => {
    const session = await fetch('/api/session', { credentials: 'same-origin' });

    if (!session.ok) throw new ApiError('Unable to start local session', session.status);

    return ((await session.json()) as { csrfToken: string }).csrfToken;
  })();
  const csrfToken = await sessionPromise;

  const headers = new Headers(init.headers);

  if (init.body) headers.set('content-type', 'application/json');
  if (init.method && 'GET' !== init.method) headers.set('x-csrf-token', csrfToken);
  const response = await fetch(path, { ...init, headers, credentials: 'same-origin' });
  const body = await response.json() as T | { error: string };

  if (!response.ok) throw new ApiError('error' in (body as object) ? (body as { error: string }).error : 'Request failed', response.status);

  return body as T;
}
