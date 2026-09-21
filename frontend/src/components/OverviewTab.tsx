import {
  faArrowUpRightFromSquare,
  faCircleCheck,
  faCircleExclamation,
  faCloudArrowDown,
  faGear,
  faRotate,
  faTerminal,
  faTriangleExclamation,
} from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Anchor, Group, Loader, SimpleGrid, Stack, Table, Tooltip } from '@mantine/core';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Alert from '@/elements/Alert.tsx';
import Badge from '@/elements/Badge.tsx';
import Button from '@/elements/Button.tsx';
import Card from '@/elements/Card.tsx';
import Code from '@/elements/Code.tsx';
import Select from '@/elements/input/Select.tsx';
import Switch from '@/elements/input/Switch.tsx';
import { Modal } from '@/elements/modals/Modal.tsx';
import Text from '@/elements/Text.tsx';
import { useAdminCan } from '@/plugins/usePermissions.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import { checkNow, getOverview, queueBulk, queueUpdate } from '../lib/api/index.ts';
import { compareVersions, kindLabel, relativeTime, strategyLabel } from '../lib/format.ts';
import type { ComponentLatest, ComponentStatus, Overview } from '../lib/schemas.ts';
import { useExtTranslations } from '../translations';
import RunLogModal from './RunLogModal.tsx';
import TargetModal from './TargetModal.tsx';

const POLL_MS = 15_000;

function LatestCard({ title, info, installedLabel }: { title: string; info: ComponentLatest; installedLabel?: string }) {
  const { t: tExt } = useExtTranslations();
  return (
    <Card>
      <Stack gap={4}>
        <Text fw={600}>{title}</Text>
        {installedLabel && (
          <Text size='sm'>
            {tExt('overview.installed', {})}: <Code>{installedLabel}</Code>
          </Text>
        )}
        <Group gap='xs' wrap='wrap'>
          <Text size='sm'>{tExt('overview.latest', {})}:</Text>
          {info.latest ? (
            <>
              <Badge size='sm' color='blue' variant='light'>
                {info.latest.version}
              </Badge>
              {info.latest.prerelease && (
                <Badge size='xs' color='orange' variant='light'>
                  {tExt('overview.prerelease', {})}
                </Badge>
              )}
              {info.latest.htmlUrl.startsWith('https://github.com/') && (
                <Anchor href={info.latest.htmlUrl} target='_blank' rel='noreferrer noopener' size='xs'>
                  {tExt('overview.releaseNotes', {})} <FontAwesomeIcon icon={faArrowUpRightFromSquare} size='xs' />
                </Anchor>
              )}
            </>
          ) : (
            <Text size='sm' c='dimmed'>
              {tExt('common.unknown', {})}
            </Text>
          )}
        </Group>
        {info.recommended && info.recommended !== info.latest?.version && (
          <Text size='xs' c='dimmed'>
            {tExt('overview.recommended', { version: info.recommended })}
          </Text>
        )}
        {info.error && (
          <Text size='xs' c='red' className='break-words'>
            {tExt('overview.githubError', { kind: title, error: info.error })}
          </Text>
        )}
      </Stack>
    </Card>
  );
}

function StatusBadge({ component }: { component: ComponentStatus }) {
  const { t: tExt } = useExtTranslations();
  if (component.activeRun) {
    const queued = component.lastRun?.status === 'queued';
    return (
      <Badge size='sm' color={queued ? 'yellow' : 'blue'} variant='light'>
        {queued ? tExt('overview.queued', {}) : tExt('overview.updating', {})}
      </Badge>
    );
  }
  if (component.error) {
    return (
      <Tooltip label={component.error} multiline maw={420} withArrow>
        <span>
          <Badge size='sm' color='red' variant='light' leftSection={<FontAwesomeIcon icon={faCircleExclamation} size='xs' />}>
            {tExt('overview.unreachable', {})}
          </Badge>
        </span>
      </Tooltip>
    );
  }
  if (component.outdated) {
    return (
      <Badge size='sm' color='orange' variant='light' leftSection={<FontAwesomeIcon icon={faCloudArrowDown} size='xs' />}>
        {tExt('overview.updateAvailable', { version: component.latest ?? '' })}
      </Badge>
    );
  }
  return (
    <Badge size='sm' color='green' variant='light' leftSection={<FontAwesomeIcon icon={faCircleCheck} size='xs' />}>
      {tExt('overview.upToDate', {})}
    </Badge>
  );
}

function UpdateModal({
  component,
  overview,
  onClose,
  onQueued,
}: {
  component: ComponentStatus | null;
  overview: Overview;
  onClose: () => void;
  onQueued: () => void;
}) {
  const { t: tExt } = useExtTranslations();
  const { addToast } = useToast();
  const [version, setVersion] = useState<string | null>(null);
  const [force, setForce] = useState(false);
  const [busy, setBusy] = useState(false);

  const info = component ? (component.kind === 'db_agent' ? overview.latest.dbAgent : overview.latest.wings) : null;
  const options = useMemo(() => {
    if (!info) return [];
    const seen = new Set<string>();
    const list: { value: string; label: string }[] = [];
    for (const release of [info.latest, ...info.recent]) {
      if (!release || seen.has(release.version)) continue;
      seen.add(release.version);
      list.push({
        value: release.version,
        label:
          release.version === info.latest?.version
            ? tExt('updateModal.latestLabel', { version: release.version })
            : `${release.version}${release.prerelease ? ` (${tExt('overview.prerelease', {})})` : ''}`,
      });
    }
    return list;
  }, [info, tExt]);

  const latestVersion = info?.latest?.version ?? null;
  const componentKey = component ? `${component.kind}:${component.refUuid}` : null;
  useEffect(() => {
    setVersion(latestVersion);
    setForce(false);
  }, [componentKey, latestVersion]);

  const selected = version ?? info?.latest?.version ?? null;
  const aheadOfPanel = selected ? compareVersions(selected, overview.panel.installed) > 0 && !overview.allowAheadOfPanel : false;

  const confirm = async () => {
    if (!component || !selected) return;
    setBusy(true);
    try {
      await queueUpdate(component.kind, component.refUuid, selected, aheadOfPanel && force);
      addToast(tExt('updateModal.queued', { name: component.name }), 'success');
      onQueued();
      onClose();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal opened={component !== null} onClose={onClose} title={tExt('updateModal.title', { name: component?.name ?? '' })}>
      {component && (
        <Stack gap='md'>
          <Text size='sm'>
            {tExt('updateModal.body', {
              kind: kindLabel(component.kind),
              name: component.name,
              installed: component.installed ?? (component.installedRaw || tExt('common.unknown', {})),
            })}
          </Text>
          {options.length === 0 ? (
            <Text size='sm' c='dimmed'>
              {tExt('updateModal.noRelease', {})}
            </Text>
          ) : (
            <Select
              label={tExt('updateModal.version', {})}
              data={options}
              value={selected}
              onChange={(v) => setVersion(v)}
              allowDeselect={false}
            />
          )}
          <Text size='xs' c='dimmed'>
            {tExt('updateModal.strategy', { strategy: strategyLabel(component.effectiveStrategy) })}
          </Text>
          {aheadOfPanel && (
            <Alert color='orange' title={tExt('overview.aheadOfPanel', {})}>
              <Stack gap='xs'>
                <Text size='sm'>{tExt('overview.forceHelp', {})}</Text>
                <Switch label={tExt('overview.forceLabel', {})} checked={force} onChange={(ev) => setForce(ev.currentTarget.checked)} />
              </Stack>
            </Alert>
          )}
          <Group justify='flex-end'>
            <Button variant='default' onClick={onClose}>
              {tExt('common.cancel', {})}
            </Button>
            <Button loading={busy} disabled={!selected || (aheadOfPanel && !force)} onClick={confirm}>
              {tExt('updateModal.confirm', {})}
            </Button>
          </Group>
        </Stack>
      )}
    </Modal>
  );
}

export default function OverviewTab({ onShowRuns }: { onShowRuns: () => void }) {
  const { t: tExt } = useExtTranslations();
  const { addToast } = useToast();
  const canUpdate = useAdminCan('node-updater.update');
  const canSettings = useAdminCan('node-updater.settings');

  const [overview, setOverview] = useState<Overview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [checking, setChecking] = useState(false);
  const [updateFor, setUpdateFor] = useState<ComponentStatus | null>(null);
  const [configureFor, setConfigureFor] = useState<ComponentStatus | null>(null);
  const [logRun, setLogRun] = useState<string | null>(null);
  const [bulkOpen, setBulkOpen] = useState(false);
  const [bulkForce, setBulkForce] = useState(false);
  const [bulkBusy, setBulkBusy] = useState(false);

  const load = useCallback((quiet = false) => {
    if (!quiet) setLoading(true);
    getOverview()
      .then((result) => {
        setOverview(result);
        setError(null);
      })
      .catch((err) => {
        if (!quiet) setError(httpErrorToHuman(err));
      })
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const anyActive = (overview?.components ?? []).some((c) => c.activeRun);
  useEffect(() => {
    if (!anyActive) return;
    const timer = setInterval(() => load(true), POLL_MS);
    return () => clearInterval(timer);
  }, [anyActive, load]);

  const check = async () => {
    setChecking(true);
    try {
      const result = await checkNow();
      setOverview(result);
      setError(null);
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setChecking(false);
    }
  };

  const bulk = async () => {
    setBulkBusy(true);
    try {
      const result = await queueBulk(undefined, bulkForce);
      addToast(tExt('overview.bulkQueued', { n: result.queued }), 'success');
      if (result.skipped.length > 0) {
        addToast(
          tExt('overview.bulkSkipped', { list: result.skipped.map((s) => `${s.name}: ${s.reason}`).join('; ') }),
          'warning',
        );
      }
      setBulkOpen(false);
      load(true);
      if (result.queued > 0) onShowRuns();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setBulkBusy(false);
    }
  };

  const sorted = useMemo(() => {
    const order = (c: ComponentStatus) => (c.error ? 0 : c.outdated ? 1 : 2);
    return [...(overview?.components ?? [])].sort((a, b) => {
      const byState = order(a) - order(b);
      if (byState !== 0) return byState;
      const byKind = a.kind.localeCompare(b.kind);
      return byKind !== 0 ? byKind : a.name.localeCompare(b.name);
    });
  }, [overview]);

  if (loading && !overview) {
    return (
      <Group justify='center' className='py-10'>
        <Loader size='sm' />
      </Group>
    );
  }

  if (error && !overview) {
    return (
      <Alert color='red' title={tExt('overview.couldNotLoad', {})}>
        <Text size='sm'>{error}</Text>
      </Alert>
    );
  }

  if (!overview) return null;

  const outdated = overview.components.filter((c) => c.outdated);
  const anyNeedsForce = outdated.some((c) => c.needsForce);
  const wingsCount = overview.components.filter((c) => c.kind === 'wings').length;
  const dbAgentCount = overview.components.filter((c) => c.kind === 'db_agent').length;

  return (
    <Stack gap='md'>
      <Card>
        <Group justify='space-between' wrap='wrap' gap='sm'>
          <Group gap='sm' wrap='wrap'>
            <Badge size='lg' color={outdated.length > 0 ? 'orange' : 'green'} variant='light'>
              {outdated.length > 0
                ? `${outdated.length} update${outdated.length === 1 ? '' : 's'} available`
                : tExt('overview.upToDate', {})}
            </Badge>
            {overview.runsActive > 0 && (
              <span className='cursor-pointer' onClick={onShowRuns}>
                <Badge size='lg' color='blue' variant='light'>
                  {overview.runsActive} active run{overview.runsActive === 1 ? '' : 's'}
                </Badge>
              </span>
            )}
            {!overview.panelPrimary && (
              <Tooltip label={tExt('overview.replicaHelp', {})} withArrow multiline maw={320}>
                <span>
                  <Badge size='lg' color='yellow' variant='light'>
                    {tExt('overview.replicaInstance', {})}
                  </Badge>
                </span>
              </Tooltip>
            )}
            <Text size='xs' c='dimmed'>
              {tExt('overview.checked', { when: relativeTime(overview.checkedAt) })}
            </Text>
          </Group>
          <Group gap='xs'>
            <Button
              size='compact-sm'
              variant='default'
              loading={checking}
              onClick={check}
              leftSection={<FontAwesomeIcon icon={faRotate} />}
            >
              {checking ? tExt('overview.checking', {}) : tExt('overview.checkNow', {})}
            </Button>
            {canUpdate && outdated.length > 0 && (
              <Button
                size='compact-sm'
                color='orange'
                onClick={() => {
                  setBulkForce(false);
                  setBulkOpen(true);
                }}
                leftSection={<FontAwesomeIcon icon={faCloudArrowDown} />}
              >
                {tExt('overview.updateAll', {})}
              </Button>
            )}
          </Group>
        </Group>
      </Card>

      {overview.panel.outdated && overview.panel.latest && (
        <Alert color='yellow' title={tExt('common.panel', {})} icon={<FontAwesomeIcon icon={faTriangleExclamation} />}>
          <Text size='sm'>
            {tExt('overview.panelOutdated', { latest: overview.panel.latest, installed: overview.panel.installed })}
          </Text>
        </Alert>
      )}
      {overview.latest.feedError && (
        <Alert color='slate'>
          <Text size='sm'>{tExt('overview.feedError', { error: overview.latest.feedError })}</Text>
        </Alert>
      )}

      <SimpleGrid cols={{ base: 1, md: 3 }}>
        <LatestCard title={tExt('common.panel', {})} info={overview.latest.panel} installedLabel={overview.panel.installedRaw} />
        <LatestCard title={`${tExt('common.wings', {})} (${wingsCount})`} info={overview.latest.wings} />
        <LatestCard title={`${tExt('common.dbAgent', {})} (${dbAgentCount})`} info={overview.latest.dbAgent} />
      </SimpleGrid>

      <Card>
        {sorted.length === 0 ? (
          <Text size='sm' c='dimmed'>
            {tExt('overview.noComponents', {})}
          </Text>
        ) : (
          <Table.ScrollContainer minWidth={900}>
            <Table striped highlightOnHover verticalSpacing={6} fz='sm'>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th>{tExt('overview.node', {})}</Table.Th>
                  <Table.Th>{tExt('overview.kind', {})}</Table.Th>
                  <Table.Th>{tExt('overview.version', {})}</Table.Th>
                  <Table.Th>{tExt('overview.status', {})}</Table.Th>
                  <Table.Th>{tExt('overview.strategy', {})}</Table.Th>
                  <Table.Th>{tExt('overview.lastRun', {})}</Table.Th>
                  <Table.Th>{tExt('overview.actions', {})}</Table.Th>
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {sorted.map((component) => (
                  <Table.Tr key={`${component.kind}:${component.refUuid}`}>
                    <Table.Td>
                      <Stack gap={0}>
                        <Text size='sm' fw={500}>
                          {component.name}
                        </Text>
                        <Text size='xs' c='dimmed' className='font-mono'>
                          {component.host ?? component.url}
                          {component.location ? ` · ${component.location}` : ''}
                        </Text>
                      </Stack>
                    </Table.Td>
                    <Table.Td>
                      <Badge size='xs' variant='light' color={component.kind === 'db_agent' ? 'violet' : 'blue'}>
                        {kindLabel(component.kind)}
                      </Badge>
                    </Table.Td>
                    <Table.Td>
                      <Group gap={6} wrap='nowrap'>
                        <Code>{component.installed ?? (component.installedRaw || '?')}</Code>
                        {component.architecture && (
                          <Text size='xs' c='dimmed'>
                            {component.architecture}
                            {component.containerType && component.containerType !== 'none' ? ' · container' : ''}
                          </Text>
                        )}
                      </Group>
                    </Table.Td>
                    <Table.Td>
                      <StatusBadge component={component} />
                    </Table.Td>
                    <Table.Td>
                      {component.effectiveStrategy ? (
                        <Tooltip label={component.blockedReason ?? ''} disabled={!component.blockedReason} multiline maw={360} withArrow>
                          <span>
                            <Text size='sm' c={component.canUpdate || component.activeRun ? undefined : 'dimmed'}>
                              {strategyLabel(component.effectiveStrategy)}
                              {component.target?.autoUpdate ? ' · auto' : ''}
                              {component.target?.profileName ? ` · ${component.target.profileName}` : ''}
                            </Text>
                          </span>
                        </Tooltip>
                      ) : (
                        <Tooltip label={component.blockedReason ?? ''} disabled={!component.blockedReason} multiline maw={360} withArrow>
                          <span>
                            <Text size='sm' c='dimmed'>
                              {tExt('overview.notConfigured', {})}
                            </Text>
                          </span>
                        </Tooltip>
                      )}
                    </Table.Td>
                    <Table.Td>
                      {component.lastRun ? (
                        <Button
                          size='compact-xs'
                          variant='subtle'
                          color={component.lastRun.status === 'failed' ? 'red' : 'gray'}
                          leftSection={<FontAwesomeIcon icon={faTerminal} />}
                          onClick={() => setLogRun(component.lastRun?.id ?? null)}
                        >
                          {component.lastRun.status} · {relativeTime(component.lastRun.finishedAt ?? component.lastRun.created)}
                        </Button>
                      ) : (
                        <Text size='xs' c='dimmed'>
                          —
                        </Text>
                      )}
                    </Table.Td>
                    <Table.Td>
                      <Group gap={4} wrap='nowrap'>
                        {canUpdate && (
                          <Button
                            size='compact-xs'
                            color='orange'
                            variant={component.outdated ? 'filled' : 'light'}
                            disabled={!component.canUpdate || component.activeRun || !component.latest}
                            leftSection={<FontAwesomeIcon icon={faCloudArrowDown} />}
                            onClick={() => setUpdateFor(component)}
                          >
                            {tExt('overview.update', {})}
                          </Button>
                        )}
                        {canSettings && (
                          <Button
                            size='compact-xs'
                            variant='default'
                            leftSection={<FontAwesomeIcon icon={faGear} />}
                            onClick={() => setConfigureFor(component)}
                          >
                            {tExt('overview.configure', {})}
                          </Button>
                        )}
                      </Group>
                    </Table.Td>
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>
          </Table.ScrollContainer>
        )}
      </Card>

      <UpdateModal component={updateFor} overview={overview} onClose={() => setUpdateFor(null)} onQueued={() => load(true)} />
      <TargetModal
        component={configureFor}
        opened={configureFor !== null}
        onClose={() => setConfigureFor(null)}
        onSaved={() => load(true)}
      />
      <RunLogModal runId={logRun} opened={logRun !== null} onClose={() => setLogRun(null)} canUpdate={canUpdate} />

      <Modal opened={bulkOpen} onClose={() => setBulkOpen(false)} title={tExt('overview.bulkTitle', {})}>
        <Stack gap='md'>
          <Text size='sm'>{tExt('overview.bulkBody', {})}</Text>
          {anyNeedsForce && (
            <Alert color='orange' title={tExt('overview.aheadOfPanel', {})}>
              <Stack gap='xs'>
                <Text size='sm'>{tExt('overview.forceHelp', {})}</Text>
                <Switch label={tExt('overview.forceLabel', {})} checked={bulkForce} onChange={(ev) => setBulkForce(ev.currentTarget.checked)} />
              </Stack>
            </Alert>
          )}
          <Group justify='flex-end'>
            <Button variant='default' onClick={() => setBulkOpen(false)}>
              {tExt('common.cancel', {})}
            </Button>
            <Button color='orange' loading={bulkBusy} onClick={bulk}>
              {tExt('overview.updateAll', {})}
            </Button>
          </Group>
        </Stack>
      </Modal>
    </Stack>
  );
}

export { OverviewTab };
