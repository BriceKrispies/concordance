// Thin fetch wrapper for the JSON API. Errors carry the server's {error:{code,message}}.

const BASE = '/api/v1';

export class ApiError extends Error {
  constructor(message, code, status) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = status;
  }
}

export function isAbort(err) {
  return err && err.name === 'AbortError';
}

function buildQuery(query) {
  const q = new URLSearchParams();
  if (query) {
    for (const [k, v] of Object.entries(query)) {
      if (v === undefined || v === null || v === '' || v === false) continue;
      q.set(k, String(v));
    }
  }
  const s = q.toString();
  return s ? '?' + s : '';
}

export async function api(path, { method = 'GET', query, body, signal } = {}) {
  const init = { method, signal, headers: { Accept: 'application/json' } };
  if (body !== undefined) {
    init.headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  let res;
  try {
    res = await fetch(BASE + path + buildQuery(query), init);
  } catch (e) {
    if (isAbort(e)) throw e;
    throw new ApiError('Cannot reach the Concordance server.', 'network', 0);
  }
  let data = null;
  const text = await res.text();
  if (text) {
    try { data = JSON.parse(text); } catch { data = null; }
  }
  if (!res.ok) {
    const err = data && data.error;
    throw new ApiError(
      (err && err.message) || `Request failed (${res.status})`,
      (err && err.code) || 'http_' + res.status,
      res.status,
    );
  }
  return data;
}

export const get = (path, query, signal) => api(path, { query, signal });
export const post = (path, body, signal) => api(path, { method: 'POST', body, signal });
