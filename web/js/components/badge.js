// Badges carry text so meaning never depends on color alone.
import { h } from '../dom.js';
import { STATUS_HELP, titleCase } from '../format.js';

export function statusBadge(status) {
  if (!status) return h('span', { class: 'badge badge-informative', title: 'Informative claim: not an obligation' }, 'informative');
  return h('span', { class: `badge badge-${status}`, title: STATUS_HELP[status] || '' }, status);
}

export function severityBadge(sev) {
  return h('span', { class: `sev sev-${sev}` }, sev);
}

export function pill(text, cls) {
  return h('span', { class: `pill ${cls || ''}`.trim() }, text);
}

export function methodPills(methods) {
  return (methods || []).map((m) => pill(titleCase(m), 'pill-method'));
}
