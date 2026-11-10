// In-memory activity feed fed by the event stream, deduplicated by seq, capped at 500.
import { subscribe as subscribeEvents } from './events.js';
import { get } from './api.js';

const CAP = 500;
let items = []; // newest first (descending seq)
const seen = new Set();
const listeners = new Set();
let filled = false;

export function getItems() {
  return items;
}

export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function notify() {
  for (const fn of [...listeners]) fn(items);
}

function scanIdOf(env) {
  return env.data && env.data.scan;
}

function insert(env) {
  if (typeof env.seq !== 'number' || seen.has(env.seq)) return false;
  seen.add(env.seq);
  // Collapse scan progress to the latest per scan.
  if (env.type === 'scan.progress') {
    const id = scanIdOf(env);
    const idx = items.findIndex((e) => e.type === 'scan.progress' && scanIdOf(e) === id);
    if (idx >= 0) {
      if (items[idx].seq > env.seq) return false;
      seen.delete(items[idx].seq);
      items.splice(idx, 1);
      seen.add(env.seq);
    }
  }
  items.push(env);
  return true;
}

function finish() {
  items.sort((a, b) => b.seq - a.seq);
  if (items.length > CAP) {
    for (const dropped of items.slice(CAP)) seen.delete(dropped.seq);
    items = items.slice(0, CAP);
  }
  notify();
}

export function startActivity() {
  subscribeEvents('resync', () => { filled = false; });
  subscribeEvents('*', (env) => {
    if (insert(env)) finish();
  });
}

// Initial fill from /events/recent, merged with anything the stream already delivered.
export async function fillActivity(signal, force = false) {
  if (filled && !force) return;
  const res = await get('/events/recent', { limit: 200 }, signal);
  filled = true;
  // Items from the recent endpoint are ascending; insertion order does not matter, we sort.
  let changed = false;
  for (const env of res.items || []) if (insert(env)) changed = true;
  if (changed) finish();
}
