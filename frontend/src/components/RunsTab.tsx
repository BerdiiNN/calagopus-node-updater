import { faRotate, faTerminal, faTrash, faXmark } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Group, Loader, Stack, Table, Tooltip } from '@mantine/core';
import { useCallback, useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Alert from '@/elements/Alert.tsx';
import Badge from '@/elements/Badge.tsx';
import Button from '@/elements/Button.tsx';
import Card from '@/elements/Card.tsx';
import Code from '@/elements/Code.tsx';
import Text from '@/elements/Text.tsx';
import { useAdminCan } from '@/plugins/usePermissions.ts';
import { useToast } from '@/providers/ToastProvider.tsx';
import { cancelRun, deleteRun, getRuns } from '../lib/api/index.ts';
import { duration, kindLabel, relativeTime, runStatusColor, runStatusLabel, strategyLabel } from '../lib/format.ts';
import { RUN_ACTIVE, type Run } from '../lib/schemas.ts';
import { useExtTranslations } from '../translations';
import RunLogModal from './RunLogModal.tsx';

const POLL_MS = 5000;

export default function RunsTab() {
  const { t: tExt } = useExtTranslations();
  const { addToast } = useToast();
  const canUpdate = useAdminCan('node-updater.update');
  const [runs, setRuns] = useState<Run[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [logRun, setLogRun] = useState<string | null>(null);

  const load = useCallback((quiet = false) => {
    if (!quiet) setLoading(true);
    getRuns(100, 0)
      .then((rows) => {
        setRuns(rows);
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

  const anyActive = (runs ?? []).some((r) => (RUN_ACTIVE as readonly string[]).includes(r.status));

  useEffect(() => {
    if (!anyActive && logRun === null) return;
    const timer = setInterval(() => load(true), POLL_MS);
    return () => clearInterval(timer);
  }, [anyActive, logRun, load]);

  const cancel = async (run: Run) => {
    try {
      await cancelRun(run.id);
      addToast(tExt('runs.cancelled', {}), 'success');
      load(true);
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    }
  };

  const remove = async (run: Run) => {
    try {
      await deleteRun(run.id);
      addToast(tExt('runs.deleted', {}), 'success');
      load(true);
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    }
  };

  if (runs === null && !error) {
    return (
      <Group justify='center' className='py-10'>
        <Loader size='sm' />
      </Group>
    );
  }

  if (error && runs === null) {
    return (
      <Alert color='red' title={tExt('runs.couldNotLoad', {})}>
        <Text size='sm'>{error}</Text>
      </Alert>
    );
  }

  return (
    <Stack gap='md'>
      <Card>
        <Group justify='space-between' wrap='wrap' gap='sm' mb='sm'>
          <Text size='xs' c='dimmed'>
            {anyActive ? tExt('runs.autoRefresh', {}) : ''}
          </Text>
          <Button
            size='compact-sm'
            variant='default'
            loading={loading}
            onClick={() => load()}
            leftSection={<FontAwesomeIcon icon={faRotate} />}
          >
            {tExt('common.refresh', {})}
          </Button>
        </Group>
        {(runs ?? []).length === 0 ? (
          <Text size='sm' c='dimmed'>
            {tExt('runs.none', {})}
          </Text>
        ) : (
          <Table.ScrollContainer minWidth={900}>
            <Table striped highlightOnHover verticalSpacing={6} fz='sm'>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th>{tExt('runs.node', {})}</Table.Th>
                  <Table.Th>{tExt('runs.change', {})}</Table.Th>
                  <Table.Th>{tExt('runs.strategy', {})}</Table.Th>
                  <Table.Th>{tExt('runs.status', {})}</Table.Th>
                  <Table.Th>{tExt('runs.started', {})}</Table.Th>
                  <Table.Th>{tExt('runs.duration', {})}</Table.Th>
                  <Table.Th>{tExt('runs.triggeredBy', {})}</Table.Th>
                  <Table.Th />
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {(runs ?? []).map((run) => {
                  const active = (RUN_ACTIVE as readonly string[]).includes(run.status);
                  return (
                    <Table.Tr key={run.id}>
                      <Table.Td>
                        <Group gap={6} wrap='nowrap'>
                          <Badge size='xs' variant='light' color={run.kind === 'db_agent' ? 'violet' : 'blue'}>
                            {kindLabel(run.kind)}
                          </Badge>
                          <Text size='sm' fw={500}>
                            {run.refName}
                          </Text>
                        </Group>
                      </Table.Td>
                      <Table.Td>
                        <Code>
                          {run.fromVersion ?? '?'} → {run.toVersion}
                        </Code>
                        {run.force && (
                          <Badge size='xs' color='orange' variant='light' ml={6}>
                            force
                          </Badge>
                        )}
                      </Table.Td>
                      <Table.Td>{strategyLabel(run.strategy)}</Table.Td>
                      <Table.Td>
                        <Tooltip label={run.error ?? ''} disabled={!run.error} multiline maw={420} withArrow>
                          <span>
                            <Badge size='sm' color={runStatusColor(run.status)} variant='light'>
                              {runStatusLabel(run.status)}
                            </Badge>
                          </span>
                        </Tooltip>
                      </Table.Td>
                      <Table.Td>{relativeTime(run.startedAt ?? run.created)}</Table.Td>
                      <Table.Td>{duration(run.startedAt, run.finishedAt)}</Table.Td>
                      <Table.Td>
                        <Text size='xs' c='dimmed'>
                          {run.triggeredBy}
                        </Text>
                      </Table.Td>
                      <Table.Td>
                        <Group gap={4} wrap='nowrap' justify='flex-end'>
                          <Button
                            size='compact-xs'
                            variant='default'
                            leftSection={<FontAwesomeIcon icon={faTerminal} />}
                            onClick={() => setLogRun(run.id)}
                          >
                            {tExt('runs.log', {})}
                          </Button>
                          {active && canUpdate && (
                            <Button
                              size='compact-xs'
                              color='red'
                              variant='light'
                              leftSection={<FontAwesomeIcon icon={faXmark} />}
                              onClick={() => cancel(run)}
                            >
                              {tExt('runs.cancel', {})}
                            </Button>
                          )}
                          {!active && canUpdate && (
                            <Button size='compact-xs' color='gray' variant='subtle' onClick={() => remove(run)}>
                              <FontAwesomeIcon icon={faTrash} />
                            </Button>
                          )}
                        </Group>
                      </Table.Td>
                    </Table.Tr>
                  );
                })}
              </Table.Tbody>
            </Table>
          </Table.ScrollContainer>
        )}
      </Card>

      <RunLogModal runId={logRun} opened={logRun !== null} onClose={() => setLogRun(null)} canUpdate={canUpdate} />
    </Stack>
  );
}

export { RunsTab };
