// SSE client. Reconnects manually so the cursor in the URL is always the current lastSeq
// (EventSource's own auto-reconnect would reuse the stale URL).

export const EVENT_TYPES = [
  'repository.discovered', 'scan.started', 'scan.progress', 'scan.completed',
  'document.indexed', 'document.removed',
  'claim.created', 'claim.updated', 'claim.removed',
  'source.indexed', 'source.removed',
  'symbol.created', 'symbol.updated', 'symbol.removed',
  'relationship.created', 'relationship.invalidated',
  'evidence.created', 'evidence.invalidated',
  'analysis.queued', 'analysis.started', 'analysis.progress', 'analysis.completed',
  'finding.created', 'finding.updated', 'finding.resolved',
  'watch.change_detected', 'system.warning', 'system.error',
];

const listeners = new Map(); // type | '*' -> Set<fn>
let source = null;
let lastSeq = 0;
let backoff = 1000;
let retryTimer = null;
let stopped = false;
let onState = () => {};

export function subscribe(type, fn) {
  if (!listeners.has(type)) listeners.set(type, new Set());
  listeners.get(type).add(fn);
  return () => listeners.get(type).delete(fn);
}

export function getLastSeq() {
  return lastSeq;
}

function emit(type, envelope) {
  for (const key of [type, '*']) {
    const set = listeners.get(key);
    if (!set) continue;
    for (const fn of [...set]) {
      try { fn(envelope); } catch (e) { console.error('event listener failed', e); }
    }
  }
}

function parse(e) {
  try { return JSON.parse(e.data); } catch { return null; }
}

function handleEnvelope(e) {
  const env = parse(e);
  if (!env || typeof env.seq !== 'number') return;
  if (env.seq <= lastSeq) return; // already applied (replay overlap)
  lastSeq = env.seq;
  emit(env.type || e.type, env);
}

function handleHello() {
  backoff = 1000;
  onState('live');
}

function handleResync(e) {
  const d = parse(e);
  if (d && typeof d.latest === 'number') lastSeq = Math.max(lastSeq, d.latest);
  // Synthetic envelope-shaped message for subscribers; not a stream event, no seq.
  emit('resync', { type: 'resync', data: d || {} });
}

function connect() {
  if (stopped) return;
  const es = new EventSource(`/api/v1/events?cursor=${encodeURIComponent(lastSeq)}`);
  source = es;
  es.addEventListener('hello', handleHello);
  es.addEventListener('resync', handleResync);
  for (const t of EVENT_TYPES) es.addEventListener(t, handleEnvelope);
  es.onerror = () => {
    // Close so the browser does not retry with the stale cursor; retry ourselves.
    es.close();
    if (source === es) source = null;
    onState('reconnecting');
    clearTimeout(retryTimer);
    retryTimer = setTimeout(connect, backoff);
    backoff = Math.min(backoff * 2, 15000);
  };
}

export function start(initialSeq, stateCallback) {
  lastSeq = initialSeq || 0;
  onState = stateCallback || onState;
  stopped = false;
  onState('connecting');
  connect();
}

export function stop() {
  stopped = true;
  clearTimeout(retryTimer);
  if (source) source.close();
  source = null;
}
