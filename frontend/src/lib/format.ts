import { getExtTranslations } from '../translations';

export function relativeTime(iso: string | undefined): string {
  const { t } = getExtTranslations();
  if (!iso) return '—';
  const at = Date.parse(iso);
  if (!Number.isFinite(at)) return '—';
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 10) return t('format.justNow', {});
  if (seconds < 60) return t('format.ago.seconds', { n: seconds });
  if (seconds < 3600) return t('format.ago.minutes', { n: Math.round(seconds / 60) });
  if (seconds < 86400) return t('format.ago.hours', { n: Math.round(seconds / 3600) });
  return t('format.ago.days', { n: Math.round(seconds / 86400) });
}

export function duration(start: string | undefined, end: string | undefined): string {
  if (!start) return '—';
  const a = Date.parse(start);
  const b = end ? Date.parse(end) : Date.now();
  if (!Number.isFinite(a) || !Number.isFinite(b)) return '—';
  const seconds = Math.max(0, Math.round((b - a) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes < 60) return rest ? `${minutes}m ${rest}s` : `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

export function kindLabel(kind: string): string {
  const { t } = getExtTranslations();
  switch (kind) {
    case 'db_agent':
      return t('common.dbAgent', {});
    case 'panel':
      return t('common.panel', {});
    default:
      return t('common.wings', {});
  }
}

export function strategyLabel(strategy: string): string {
  const { t } = getExtTranslations();
  switch (strategy) {
    case 'api':
      return t('overview.strategyApi', {});
    case 'ssh_docker':
      return t('overview.strategySshDocker', {});
    case 'ssh_package':
      return t('overview.strategySshPackage', {});
    case 'ssh_binary':
      return t('overview.strategySshBinary', {});
    case 'auto':
      return t('overview.strategyAuto', {});
    default:
      return strategy || '—';
  }
}

export function runStatusColor(status: string): string {
  switch (status) {
    case 'succeeded':
      return 'green';
    case 'failed':
      return 'red';
    case 'running':
      return 'blue';
    case 'queued':
      return 'yellow';
    case 'cancelled':
      return 'gray';
    default:
      return 'gray';
  }
}

export function runStatusLabel(status: string): string {
  const { t } = getExtTranslations();
  switch (status) {
    case 'succeeded':
      return t('runs.succeeded', {});
    case 'failed':
      return t('runs.failed', {});
    case 'running':
      return t('runs.running', {});
    case 'queued':
      return t('runs.queued', {});
    case 'cancelled':
      return t('runs.cancelledStatus', {});
    default:
      return status;
  }
}

/** `1.2.0` > `1.1.5` → 1, equal → 0, unparsable → 0. */
export function compareVersions(a: string | undefined, b: string | undefined): number {
  if (!a || !b) return 0;
  const pa = a.split(/[.-]/).map((p) => Number(p));
  const pb = b.split(/[.-]/).map((p) => Number(p));
  for (let i = 0; i < Math.max(pa.length, pb.length); i += 1) {
    const x = Number.isFinite(pa[i]) ? pa[i] : 0;
    const y = Number.isFinite(pb[i]) ? pb[i] : 0;
    if (x !== y) return x > y ? 1 : -1;
  }
  return 0;
}
