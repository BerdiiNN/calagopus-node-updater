import { faArrowUpRightFromSquare } from '@fortawesome/free-solid-svg-icons';
import { useEffect } from 'react';
import { useAdminCan } from '@/plugins/usePermissions.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import { getOverview } from '../lib/api/index.ts';
import { navigateTo } from '../lib/navigate.ts';
import { getExtTranslations } from '../translations';

const STORAGE_KEY = 'nodeupdater:noticed';
const MIN_INTERVAL_MS = 10 * 60_000;

/** Last time any mount of this component asked the panel; shared across mounts. */
let lastFetchAt = 0;

/**
 * Mounted through page container slots that live INSIDE the panel's `AuthProvider`
 * (dashboard home, admin home) — never through the global slot, which renders outside it
 * and where `useAuth` throws and an unauthenticated API call trips the session-expired
 * handler. For administrators holding `node-updater.read` it fetches the cached overview
 * (at most every ten minutes) and shows a toast once per distinct set of pending versions.
 */
export default function UpdateNotice(): null {
  const can = useAdminCan('node-updater.read');
  const { addToast } = useToast();

  useEffect(() => {
    if (!can) return;
    if (Date.now() - lastFetchAt < MIN_INTERVAL_MS) return;
    lastFetchAt = Date.now();
    let active = true;
    const timer = setTimeout(() => {
      getOverview()
        .then((overview) => {
          if (!active) return;
          const outdated = overview.components.filter((c) => c.outdated);
          const panelOutdated = overview.panel.outdated;
          if (outdated.length === 0 && !panelOutdated) return;
          const signature = [
            panelOutdated ? `panel:${overview.panel.latest ?? ''}` : '',
            ...outdated.map((c) => `${c.kind}:${c.refUuid}:${c.latest ?? ''}`),
          ]
            .filter(Boolean)
            .sort()
            .join('|');
          try {
            if (sessionStorage.getItem(STORAGE_KEY) === signature) return;
            sessionStorage.setItem(STORAGE_KEY, signature);
          } catch {
            /* storage unavailable: still show the toast */
          }
          const { t } = getExtTranslations();
          const message = panelOutdated
            ? t('notice.panel', { latest: overview.panel.latest ?? '', installed: overview.panel.installed })
            : t('notice.updates', { n: outdated.length });
          addToast(message, 'info', [
            {
              name: t('notice.open', {}),
              icon: faArrowUpRightFromSquare,
              onClick: () => navigateTo('/admin/node-updater'),
            },
          ]);
        })
        .catch(() => {
          /* not fatal; the admin page shows the error */
        });
    }, 1500);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [can, addToast]);

  return null;
}

export { UpdateNotice };
