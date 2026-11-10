// Keeps store.status current from the event stream.
import { subscribe } from './events.js';
import { getState, setState, loadStatus, invalidateCatalog } from './store.js';
import { throttle } from './dom.js';

const reload = throttle(() => { loadStatus().catch(() => {}); }, 1500);

export function startStatusSync() {
  subscribe('*', (env) => {
    const { status } = getState();
    const d = env.data || {};
    switch (env.type) {
      case 'scan.started':
        if (status) {
          setState({ status: { ...status, scan: { id: d.scan, reason: d.reason, phase: 'discover', done: 0, total: d.files, started_at: env.at } } });
        }
        return;
      case 'scan.progress':
        if (status) {
          const prev = status.scan || { id: d.scan, started_at: env.at };
          setState({ status: { ...status, scan: { ...prev, phase: d.phase, done: d.done, total: d.total } } });
        }
        return;
      case 'scan.completed':
        invalidateCatalog();
        if (status) setState({ status: { ...status, scan: null } });
        reload();
        return;
      case 'document.indexed':
      case 'document.removed':
      case 'source.indexed':
      case 'source.removed':
        invalidateCatalog();
        return;
      default:
        // Any other event can move counts or semantic queue numbers; reload is throttled.
        reload();
    }
  });
}

export function reloadStatusNow() {
  invalidateCatalog();
  return loadStatus();
}
