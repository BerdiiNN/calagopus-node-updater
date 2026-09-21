import { faBell, faClock, faMagnifyingGlass, faPaperPlane } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Group, Loader, SimpleGrid, Stack } from '@mantine/core';
import { useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Alert from '@/elements/Alert.tsx';
import Button from '@/elements/Button.tsx';
import NumberInput from '@/elements/input/NumberInput.tsx';
import PasswordInput from '@/elements/input/PasswordInput.tsx';
import Switch from '@/elements/input/Switch.tsx';
import TextInput from '@/elements/input/TextInput.tsx';
import Text from '@/elements/Text.tsx';
import TitleCard from '@/elements/TitleCard.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { getSettings, saveSettings, testNotification } from '../lib/api/index.ts';
import type { Settings } from '../lib/schemas.ts';
import { useExtTranslations } from '../translations';

type Channel = 'email' | 'discord' | 'webhook';

function num(value: string | number, fallback: number): number {
  const n = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(n) ? Math.floor(n) : fallback;
}

export default function SettingsTab() {
  const { t: tExt } = useExtTranslations();
  const { addToast } = useToast();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState<Channel | null>(null);
  const [secret, setSecret] = useState('');
  const [clearSecret, setClearSecret] = useState(false);

  const load = () => {
    getSettings()
      .then((s) => {
        setSettings(s);
        setError(null);
        setSecret('');
        setClearSecret(false);
      })
      .catch((err) => setError(httpErrorToHuman(err)));
  };

  useEffect(load, []);

  const update = (patch: Partial<Settings>) => setSettings((s) => (s ? { ...s, ...patch } : s));

  const save = async () => {
    if (!settings) return;
    setSaving(true);
    try {
      await saveSettings({
        checkIntervalMinutes: settings.checkIntervalMinutes,
        includePrereleases: settings.includePrereleases,
        allowAheadOfPanel: settings.allowAheadOfPanel,
        notifyEmail: settings.notifyEmail,
        notifyOnSuccess: settings.notifyOnSuccess,
        notifyDiscordUrl: settings.notifyDiscordUrl,
        notifyWebhookUrl: settings.notifyWebhookUrl,
        notifyWebhookSecret: secret,
        clearWebhookSecret: clearSecret,
        remindAfterHours: settings.remindAfterHours,
        sshConnectTimeoutSecs: settings.sshConnectTimeoutSecs,
        updateTimeoutSecs: settings.updateTimeoutSecs,
        verifyTimeoutSecs: settings.verifyTimeoutSecs,
      });
      addToast(tExt('settings.saved', {}), 'success');
      load();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSaving(false);
    }
  };

  const test = async (channel: Channel) => {
    setTesting(channel);
    try {
      await testNotification(channel);
      addToast(tExt('settings.testOk', {}), 'success');
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setTesting(null);
    }
  };

  if (!settings && !error) {
    return (
      <Group justify='center' className='py-10'>
        <Loader size='sm' />
      </Group>
    );
  }
  if (!settings) {
    return (
      <Alert color='red' title={tExt('settings.couldNotLoad', {})}>
        <Text size='sm'>{error}</Text>
      </Alert>
    );
  }

  return (
    <Stack gap='md'>
      <TitleCard title={tExt('settings.checks', {})} icon={<FontAwesomeIcon icon={faMagnifyingGlass} />}>
        <Stack gap='md'>
          <Text size='xs' c='dimmed'>
            {tExt('settings.panelVersion', { version: settings.panelVersion })}
          </Text>
          <NumberInput
            label={tExt('settings.interval', {})}
            description={tExt('settings.intervalHelp', {})}
            min={15}
            max={1440}
            value={settings.checkIntervalMinutes}
            onChange={(v) => update({ checkIntervalMinutes: num(v, 60) })}
            className='max-w-72'
          />
          <Switch
            label={tExt('settings.prereleases', {})}
            checked={settings.includePrereleases}
            onChange={(ev) => update({ includePrereleases: ev.currentTarget.checked })}
          />
          <Switch
            label={tExt('settings.allowAhead', {})}
            description={tExt('settings.allowAheadHelp', {})}
            checked={settings.allowAheadOfPanel}
            onChange={(ev) => update({ allowAheadOfPanel: ev.currentTarget.checked })}
          />
        </Stack>
      </TitleCard>

      <TitleCard title={tExt('settings.notifications', {})} icon={<FontAwesomeIcon icon={faBell} />}>
        <Stack gap='md'>
          <Group align='flex-end' wrap='wrap'>
            <Switch
              label={tExt('settings.email', {})}
              description={tExt('settings.emailHelp', {})}
              checked={settings.notifyEmail}
              onChange={(ev) => update({ notifyEmail: ev.currentTarget.checked })}
            />
            <Button
              size='compact-sm'
              variant='default'
              loading={testing === 'email'}
              leftSection={<FontAwesomeIcon icon={faPaperPlane} />}
              onClick={() => test('email')}
            >
              {tExt('settings.test', {})}
            </Button>
          </Group>
          <Switch
            label={tExt('settings.onSuccess', {})}
            description={tExt('settings.onSuccessHelp', {})}
            checked={settings.notifyOnSuccess}
            onChange={(ev) => update({ notifyOnSuccess: ev.currentTarget.checked })}
          />
          <Group align='flex-end' wrap='nowrap'>
            <TextInput
              label={tExt('settings.discordUrl', {})}
              placeholder='https://discord.com/api/webhooks/…'
              value={settings.notifyDiscordUrl}
              onChange={(ev) => update({ notifyDiscordUrl: ev.currentTarget.value })}
              className='flex-1'
            />
            <Button
              variant='default'
              loading={testing === 'discord'}
              disabled={!settings.notifyDiscordUrl}
              onClick={() => test('discord')}
            >
              {tExt('settings.test', {})}
            </Button>
          </Group>
          <Group align='flex-end' wrap='nowrap'>
            <TextInput
              label={tExt('settings.webhookUrl', {})}
              placeholder='https://example.com/hooks/node-updater'
              value={settings.notifyWebhookUrl}
              onChange={(ev) => update({ notifyWebhookUrl: ev.currentTarget.value })}
              className='flex-1'
            />
            <Button
              variant='default'
              loading={testing === 'webhook'}
              disabled={!settings.notifyWebhookUrl}
              onClick={() => test('webhook')}
            >
              {tExt('settings.test', {})}
            </Button>
          </Group>
          <SimpleGrid cols={{ base: 1, md: 2 }}>
            <PasswordInput
              label={tExt('settings.webhookSecret', {})}
              description={settings.webhookSecretSet ? tExt('settings.webhookSecretSet', {}) : tExt('settings.webhookSecretHelp', {})}
              value={secret}
              onChange={(ev) => setSecret(ev.currentTarget.value)}
              disabled={clearSecret}
            />
            {settings.webhookSecretSet && (
              <Switch
                label={tExt('settings.clearSecret', {})}
                checked={clearSecret}
                onChange={(ev) => setClearSecret(ev.currentTarget.checked)}
                className='self-end mb-1'
              />
            )}
          </SimpleGrid>
          <NumberInput
            label={tExt('settings.remind', {})}
            description={tExt('settings.remindHelp', {})}
            min={0}
            max={2160}
            value={settings.remindAfterHours}
            onChange={(v) => update({ remindAfterHours: num(v, 0) })}
            className='max-w-72'
          />
        </Stack>
      </TitleCard>

      <TitleCard title={tExt('settings.timeouts', {})} icon={<FontAwesomeIcon icon={faClock} />}>
        <SimpleGrid cols={{ base: 1, md: 3 }}>
          <NumberInput
            label={tExt('settings.sshTimeout', {})}
            min={5}
            max={300}
            value={settings.sshConnectTimeoutSecs}
            onChange={(v) => update({ sshConnectTimeoutSecs: num(v, 30) })}
          />
          <NumberInput
            label={tExt('settings.updateTimeout', {})}
            min={60}
            max={7200}
            value={settings.updateTimeoutSecs}
            onChange={(v) => update({ updateTimeoutSecs: num(v, 900) })}
          />
          <NumberInput
            label={tExt('settings.verifyTimeout', {})}
            description={tExt('settings.verifyTimeoutHelp', {})}
            min={30}
            max={1800}
            value={settings.verifyTimeoutSecs}
            onChange={(v) => update({ verifyTimeoutSecs: num(v, 180) })}
          />
        </SimpleGrid>
      </TitleCard>

      <Group>
        <Button loading={saving} onClick={save}>
          {tExt('common.save', {})}
        </Button>
        <Button variant='default' disabled={saving} onClick={load}>
          {tExt('common.refresh', {})}
        </Button>
      </Group>
    </Stack>
  );
}

export { SettingsTab };
