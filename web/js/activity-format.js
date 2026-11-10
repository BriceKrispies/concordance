// Human-readable one-line descriptions of events, grouped into filterable categories.
import { h, route, nodeLink } from './dom.js';
import { titleCase, duration } from './format.js';

export const CATEGORIES = [
  ['scan', 'Scans'],
  ['files', 'Files'],
  ['findings', 'Findings'],
  ['evidence', 'Evidence'],
  ['analysis', 'Analysis'],
  ['system', 'System'],
];

export function categoryOf(type) {
  if (type.startsWith('scan.') || type === 'repository.discovered') return 'scan';
  if (type.startsWith('finding.')) return 'findings';
  if (type.startsWith('evidence.')) return 'evidence';
  if (type.startsWith('analysis.')) return 'analysis';
  if (type.startsWith('system.')) return 'system';
  return 'files';
}

const findingLink = (id, label) => h('a', { href: route('findings/' + id) }, label || id);

// Returns { level: 'info'|'warn'|'error'|'good', body: [nodes] }.
export function describe(env) {
  const d = env.data || {};
  switch (env.type) {
    case 'repository.discovered':
      return { level: 'info', body: [`Repository discovered: ${d.markdown_files} Markdown files, ${d.rust_files} Rust files, ${d.crates} crates`] };
    case 'scan.started':
      return { level: 'info', body: [`Scan #${d.scan} started (${d.reason}, ${d.files} files)`] };
    case 'scan.progress':
      return { level: 'info', body: [`Scan #${d.scan}: ${titleCase(d.phase)} ${d.done}/${d.total}`] };
    case 'scan.completed':
      return { level: 'good', body: [`Scan #${d.scan} completed at revision ${d.revision} in ${duration(d.duration_ms)}; ${d.changed_files} files changed. ${d.counts ? `${d.counts.violated} violated, ${d.counts.stale} stale, ${d.counts.ambiguous} ambiguous.` : ''}`] };
    case 'document.indexed':
      return { level: 'info', body: ['Indexed ', h('a', { href: route('specs/' + d.document) }, d.path), ` (${d.claims} claims)`] };
    case 'document.removed':
      return { level: 'info', body: [`Document removed: ${d.path}`] };
    case 'source.indexed':
      return { level: d.parse_error ? 'warn' : 'info', body: [`Indexed ${d.path} (${d.symbols} symbols)`, d.parse_error ? ` - parse error: ${d.parse_error}` : ''] };
    case 'source.removed':
      return { level: 'info', body: [`Source removed: ${d.path}`] };
    case 'watch.change_detected': {
      const ch = d.changes || [];
      const shown = ch.slice(0, 4).map(([p, k]) => `${p} (${k})`).join(', ');
      return { level: 'info', body: [`File changes detected: ${shown}${ch.length > 4 ? ` and ${ch.length - 4} more` : ''}`] };
    }
    case 'claim.created':
    case 'claim.updated':
      return { level: 'info', body: [`Claim ${env.type.endsWith('created') ? 'created' : 'updated'}: `, nodeLink(d.claim.id, truncate(d.claim.text, 100), 'plain-link')] };
    case 'claim.removed':
      return { level: 'info', body: ['Claim removed: ', h('span', { class: 'mono' }, d.claim)] };
    case 'symbol.created':
    case 'symbol.updated':
      return { level: 'info', body: [`Symbol ${env.type.endsWith('created') ? 'created' : 'updated'}: `, nodeLink(d.symbol.id, d.symbol.path), d.signature_changed ? ' (signature changed)' : ''] };
    case 'symbol.removed':
      return { level: 'info', body: ['Symbol removed: ', h('span', { class: 'mono' }, d.symbol)] };
    case 'relationship.created':
      return { level: 'info', body: [`Relationship created: ${titleCase(d.edge.kind)} ${d.edge.target_text}`] };
    case 'relationship.invalidated':
      return { level: 'warn', body: [`Relationship invalidated: ${d.reason}`] };
    case 'evidence.created':
      return { level: 'info', body: [`Evidence recorded (${d.evidence.stance}, ${d.evidence.strength}): ${truncate(d.evidence.summary, 120)}`] };
    case 'evidence.invalidated':
      return { level: 'warn', body: ['Evidence invalidated: ', d.reason, ' (', nodeLink(d.subject), ')'] };
    case 'analysis.queued':
      return { level: 'info', body: ['Analysis queued for ', nodeLink(d.claim)] };
    case 'analysis.started':
      return { level: 'info', body: [`${d.challenge ? 'Adversarial challenge' : 'Semantic analysis'} started with ${d.model} for `, nodeLink(d.claim)] };
    case 'analysis.progress':
      return { level: 'info', body: [`Analysis stage: ${d.stage}`] };
    case 'analysis.completed': {
      const bad = d.outcome === 'failed';
      return { level: bad ? 'error' : d.outcome === 'applied' ? 'good' : 'info',
        body: [`Analysis ${d.outcome} for `, nodeLink(d.claim), d.verdict ? `: ${d.verdict}` : '', d.detail ? ` (${d.detail})` : ''] };
    }
    case 'finding.created':
      return { level: d.finding.status === 'violated' ? 'warn' : 'info', body: ['Finding created: ', findingLink(d.finding.id, d.finding.title), ` [${d.finding.status}, ${d.finding.severity}]`] };
    case 'finding.updated':
      return { level: d.finding.status === 'violated' ? 'warn' : 'info', body: ['Finding updated: ', findingLink(d.finding.id, d.finding.title), ` [${d.previous_status} → ${d.finding.status}]`] };
    case 'finding.resolved':
      return { level: 'good', body: ['Finding resolved: ', findingLink(d.finding), ` (was ${d.previous_status})`] };
    case 'system.warning':
      return { level: 'warn', body: [`Warning: ${d.message}`] };
    case 'system.error':
      return { level: 'error', body: [`Error: ${d.message}`] };
    default:
      return { level: 'info', body: [titleCase(env.type)] };
  }
}

function truncate(t, n) {
  const s = String(t ?? '');
  return s.length > n ? s.slice(0, n - 1) + '…' : s;
}
