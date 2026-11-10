// App state with subscribe/notify, plus small caches for catalogs used by filters.
import { get } from './api.js';

const state = {
  status: null,
  conn: 'connecting', // connecting | live | reconnecting
  resynced: false,
};
const listeners = new Set();

export function getState() {
  return state;
}

export function setState(patch) {
  Object.assign(state, patch);
  for (const fn of [...listeners]) fn(state);
}

export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export async function loadStatus(signal) {
  const status = await get('/status', null, signal);
  setState({ status });
  return status;
}

// Catalogs (documents, crates) are cached until a scan or document event invalidates them.
let catalog = null;

export function invalidateCatalog() {
  catalog = null;
}

export function loadCatalog(signal) {
  if (!catalog) {
    catalog = Promise.all([
      get('/documents', { limit: 500 }, signal),
      get('/crates', null, signal),
    ]).then(([documents, crates]) => ({
      documents: documents.items || [],
      crates: crates.items || [],
    }));
    catalog.catch(() => { catalog = null; });
  }
  return catalog;
}
