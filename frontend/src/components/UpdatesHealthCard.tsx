import { faArrowUpRightFromSquare, faCloudArrowDown } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Group, Loader, Stack } from '@mantine/core';
import { useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Badge from '@/elements/Badge.tsx';
import Button from '@/elements/Button.tsx';
import Code from '@/elements/Code.tsx';
import Text from '@/elements/Text.tsx';
import TitleCard from '@/elements/TitleCard.tsx';
import { useAdminCan } from '@/plugins/usePermissions.ts';
import { getOverview } from '../lib/api/index.ts';
import { kindLabel, relativeTime } from '../lib/format.ts';
import { navigateTo } from '../lib/navigate.ts';
import type { Overview } from '../lib/schemas.ts';
import SafeBoundary from '../SafeBoundary.tsx';
import { useExtTranslations } from '../translations';

/**
 * Card on Admin → Overview → Health: the update state of the panel and every daemon at a
 * glance, with a jump to the Node Updater page. Rendered inside `AuthProvider`.
 */
function Inner() {
  const { t: tExt } = useExtTranslations();
  const can = useAdminCan('node-updater.read');
  const [overview, setOverview] = useState<Overview | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!can) return;
    let active = true;
    getOverview()
      .then((result) => active && setOverview(result))
      .catch((err) => active && setError(httpErrorToHuman(err)));
    return () => {
      active = false;
    };
  }, [can]);

  if (!can) return null;

  const outdated = overview?.components.filter((c) => c.outdated) ?? [];
  const unreachable = overview?.components.filter((c) => c.error) ?? [];
  const allGood = overview && outdated.length === 0 && unreachable.length === 0 && !overview.panel.outdated;

  return (
    <TitleCard
      title={tExt('common.nodeUpdater', {})}
      icon={<FontAwesomeIcon icon={faCloudArrowDown} />}
      rightSection={
        <Button
          size='compact-sm'
          variant='default'
          leftSection={<FontAwesomeIcon icon={faArrowUpRightFromSquare} />}
          onClick={() => navigateTo('/admin/node-updater')}
        >
          {tExt('card.open', {})}
        </Button>
      }
    >
      {!overview && !error && (
        <Group justify='center' className='py-4'>
          <Loader size='sm' />
        </Group>
      )}
      {error && (
        <Text size='sm' c='red'>
          {error}
        </Text>
      )}
      {overview && (
        <Stack gap='xs'>
          <Group gap='xs' wrap='wrap'>
            {allGood ? (
              <Badge color='green' variant='light'>
                {tExt('overview.upToDate', {})}
              </Badge>
            ) : (
              <>
                {overview.panel.outdated && overview.panel.latest && (
                  <Badge color='yellow' variant='light'>
                    {tExt('common.panel', {})}: {tExt('overview.updateAvailable', { version: overview.panel.latest })}
                  </Badge>
                )}
                {outdated.length > 0 && (
                  <Badge color='orange' variant='light'>
                    {outdated.length} update{outdated.length === 1 ? '' : 's'}
                  </Badge>
                )}
                {unreachable.length > 0 && (
                  <Badge color='red' variant='light'>
                    {unreachable.length} {tExt('overview.unreachable', {}).toLowerCase()}
                  </Badge>
                )}
              </>
            )}
            <Text size='xs' c='dimmed'>
              {tExt('overview.checked', { when: relativeTime(overview.checkedAt) })}
            </Text>
          </Group>
          {outdated.slice(0, 6).map((component) => (
            <Group key={`${component.kind}:${component.refUuid}`} gap='xs' wrap='nowrap'>
              <Badge size='xs' variant='light' color={component.kind === 'db_agent' ? 'violet' : 'blue'}>
                {kindLabel(component.kind)}
              </Badge>
              <Text size='sm' className='truncate'>
                {component.name}
              </Text>
              <Code>
                {component.installed ?? '?'} → {component.latest ?? '?'}
              </Code>
            </Group>
          ))}
          {outdated.length > 6 && (
            <Text size='xs' c='dimmed'>
              +{outdated.length - 6}
            </Text>
          )}
        </Stack>
      )}
    </TitleCard>
  );
}

export default function UpdatesHealthCard() {
  return (
    <SafeBoundary>
      <Inner />
    </SafeBoundary>
  );
}

export { UpdatesHealthCard };
