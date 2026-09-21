import { Group, Loader, ScrollArea, Stack } from '@mantine/core';
import { useEffect, useRef, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Badge from '@/elements/Badge.tsx';
import Button from '@/elements/Button.tsx';
import Code from '@/elements/Code.tsx';
import Text from '@/elements/Text.tsx';
import { Modal } from '@/elements/modals/Modal.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { cancelRun, getRun } from '../lib/api/index.ts';
import { duration, runStatusColor, runStatusLabel, strategyLabel } from '../lib/format.ts';
import { RUN_ACTIVE, type RunDetail } from '../lib/schemas.ts';
import { useExtTranslations } from '../translations';

const POLL_MS = 2000;

export default function RunLogModal({
  runId,
  opened,
  onClose,
  canUpdate,
}: {
  runId: string | null;
  opened: boolean;
  onClose: () => void;
  canUpdate: boolean;
}) {
  const { t: tExt } = useExtTranslations();
  const { addToast } = useToast();
  const [run, setRun] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const viewport = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!opened || !runId) {
      setRun(null);
      setError(null);
      return;
    }
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = () => {
      getRun(runId)
        .then((detail) => {
          if (!active) return;
          setRun(detail);
          setError(null);
          if ((RUN_ACTIVE as readonly string[]).includes(detail.status)) {
            timer = setTimeout(load, POLL_MS);
          }
        })
        .catch((err) => {
          if (!active) return;
          setError(httpErrorToHuman(err));
        });
    };
    load();
    return () => {
      active = false;
      if (timer) clearTimeout(timer);
    };
  }, [opened, runId]);

  useEffect(() => {
    // Follow the log while it grows.
    const el = viewport.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [run?.log]);

  const active = run ? (RUN_ACTIVE as readonly string[]).includes(run.status) : false;

  const cancel = async () => {
    if (!run) return;
    setCancelling(true);
    try {
      await cancelRun(run.id);
      addToast(tExt('runs.cancelled', {}), 'success');
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setCancelling(false);
    }
  };

  return (
    <Modal opened={opened} onClose={onClose} title={tExt('runs.logTitle', { name: run?.refName ?? '' })} size='xl'>
      {!run && !error && (
        <Group justify='center' className='py-6'>
          <Loader size='sm' />
        </Group>
      )}
      {error && (
        <Text size='sm' c='red'>
          {error}
        </Text>
      )}
      {run && (
        <Stack gap='sm'>
          <Group gap='xs' wrap='wrap'>
            <Badge size='sm' color={runStatusColor(run.status)} variant='light'>
              {runStatusLabel(run.status)}
            </Badge>
            <Code>
              {run.fromVersion ?? '?'} → {run.toVersion}
            </Code>
            <Text size='xs' c='dimmed'>
              {strategyLabel(run.strategy)} · {duration(run.startedAt, run.finishedAt)} · {run.triggeredBy}
            </Text>
          </Group>
          {run.error && (
            <Text size='sm' c='red' className='break-words'>
              {run.error}
            </Text>
          )}
          <ScrollArea.Autosize mah={460} viewportRef={viewport} type='auto'>
            <pre className='text-xs font-mono whitespace-pre-wrap break-words bg-black/30 rounded p-3 m-0'>
              {run.log || tExt('runs.logEmpty', {})}
            </pre>
          </ScrollArea.Autosize>
          <Group justify='space-between'>
            <Text size='xs' c='dimmed'>
              {active ? tExt('runs.autoRefresh', {}) : ''}
            </Text>
            <Group gap='xs'>
              {active && canUpdate && (
                <Button size='compact-sm' color='red' variant='light' loading={cancelling} onClick={cancel}>
                  {tExt('runs.cancel', {})}
                </Button>
              )}
              <Button size='compact-sm' variant='default' onClick={onClose}>
                {tExt('common.close', {})}
              </Button>
            </Group>
          </Group>
        </Stack>
      )}
    </Modal>
  );
}

export { RunLogModal };
