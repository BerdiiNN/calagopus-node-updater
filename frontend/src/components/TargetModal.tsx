import { faKey, faMagnifyingGlass, faPlug, faTrash } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Group, SimpleGrid, Stack } from '@mantine/core';
import { useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Alert from '@/elements/Alert.tsx';
import Badge from '@/elements/Badge.tsx';
import Button from '@/elements/Button.tsx';
import Code from '@/elements/Code.tsx';
import NumberInput from '@/elements/input/NumberInput.tsx';
import PasswordInput from '@/elements/input/PasswordInput.tsx';
import Select from '@/elements/input/Select.tsx';
import Switch from '@/elements/input/Switch.tsx';
import TextArea from '@/elements/input/TextArea.tsx';
import TextInput from '@/elements/input/TextInput.tsx';
import { Modal } from '@/elements/modals/Modal.tsx';
import Text from '@/elements/Text.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import {
  type HostKey,
  type TargetInput,
  deleteTarget,
  detectTarget,
  getProfiles,
  probeHostKey,
  saveTarget,
  testTarget,
} from '../lib/api/index.ts';
import { strategyLabel } from '../lib/format.ts';
import { AUTHS, type ComponentStatus, type Detected, type Profile, STRATEGIES, type Target } from '../lib/schemas.ts';
import { useExtTranslations } from '../translations';

type Form = Omit<TargetInput, 'hostKeyFingerprint'> & { hostKeyFingerprint: string | null };

function formFrom(component: ComponentStatus | null): Form {
  const t = component?.target;
  return {
    strategy: t?.strategy ?? 'auto',
    sshHost: t?.sshHost ?? component?.host ?? '',
    sshPort: t?.sshPort ?? 22,
    sshUser: t?.sshUser ?? 'root',
    sshAuth: t?.sshAuth ?? 'none',
    password: '',
    privateKey: '',
    passphrase: '',
    clearSecret: false,
    hostKeyFingerprint: null,
    useSudo: t?.useSudo ?? false,
    composeDir: t?.composeDir ?? '',
    composeService: t?.composeService ?? '',
    serviceName: t?.serviceName ?? '',
    binaryPath: t?.binaryPath ?? '',
    restartCommand: t?.restartCommand ?? '',
    autoUpdate: t?.autoUpdate ?? false,
    profileId: t?.profileId ?? null,
  };
}

function DetectedView({ detected }: { detected: Detected }) {
  const { t: tExt } = useExtTranslations();
  const rows: [string, string][] = [
    ['OS', detected.os],
    ['Arch', detected.architecture],
    ['Init', detected.init],
    ['Package', detected.packageInstalled ? `${detected.packageManager}: ${detected.packageInstalled}` : detected.packageManager],
    ['Service', detected.unit ? `${detected.unit} (${detected.unitActive || '?'})` : ''],
    ['Binary', detected.binary ? `${detected.binary} ${detected.binaryVersion}`.trim() : detected.execStart],
    ['Container', detected.dockerContainer ? `${detected.dockerContainer} ${detected.dockerImage}` : ''],
    ['Compose', detected.composeDir ? `${detected.composeDir} ${detected.composeService ? `(${detected.composeService})` : ''} ${detected.composeTool}`.trim() : ''],
    ['Login', `${detected.whoami}${detected.sudoOk ? ' (root/sudo ok)' : ''}`],
  ];
  return (
    <Stack gap={4}>
      <Text size='sm' fw={600}>
        {tExt('target.detected', {})}
      </Text>
      {rows
        .filter(([, value]) => value && value.trim())
        .map(([label, value]) => (
          <Group key={label} gap='xs' wrap='nowrap'>
            <Text size='xs' c='dimmed' className='w-20 shrink-0'>
              {label}
            </Text>
            <Text size='xs' className='font-mono break-all'>
              {value}
            </Text>
          </Group>
        ))}
      <Text size='sm' mt={4}>
        {detected.suggestedStrategy
          ? tExt('target.suggestion', { strategy: strategyLabel(detected.suggestedStrategy) })
          : tExt('target.detectedNone', {})}
      </Text>
      {detected.suggestionReason && (
        <Text size='xs' c='dimmed'>
          {detected.suggestionReason}
        </Text>
      )}
    </Stack>
  );
}

export default function TargetModal({
  component,
  opened,
  onClose,
  onSaved,
}: {
  component: ComponentStatus | null;
  opened: boolean;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t: tExt } = useExtTranslations();
  const { addToast } = useToast();
  const [form, setForm] = useState<Form>(formFrom(null));
  const [saved, setSaved] = useState<Target | null>(null);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [saving, setSaving] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [probing, setProbing] = useState(false);
  const [probed, setProbed] = useState<HostKey | null>(null);
  const [trust, setTrust] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testOutput, setTestOutput] = useState<string | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [detected, setDetected] = useState<Detected | null>(null);

  useEffect(() => {
    if (!opened) return;
    setForm(formFrom(component));
    setSaved(component?.target ?? null);
    setProbed(null);
    setTrust(false);
    setTestOutput(null);
    setDetected(component?.target?.detected ?? null);
    let active = true;
    getProfiles()
      .then((rows) => active && setProfiles(rows))
      .catch(() => active && setProfiles([]));
    return () => {
      active = false;
    };
  }, [opened, component]);

  const set = (patch: Partial<Form>) => setForm((f) => ({ ...f, ...patch }));

  if (!component) return null;

  const usesSsh = form.strategy !== 'api';
  const profile = form.profileId ? profiles.find((p) => p.id === form.profileId) ?? null : null;
  const usesProfile = form.profileId !== null;
  const strategyOptions = STRATEGIES.map((s) => ({ value: s, label: strategyLabel(s) }));
  const authOptions = AUTHS.map((a) => ({
    value: a,
    label: a === 'none' ? tExt('target.authNone', {}) : a === 'password' ? tExt('target.authPassword', {}) : tExt('target.authKey', {}),
  }));
  const profileOptions = [
    { value: '', label: tExt('target.profileOwn', {}) },
    ...profiles.map((p) => ({ value: p.id, label: `${p.name} (${p.sshUser}, ${p.sshAuth === 'key' ? tExt('target.authKey', {}) : tExt('target.authPassword', {})})` })),
  ];
  const helpKey = `target.strategyHelp.${form.strategy}` as
    | 'target.strategyHelp.auto'
    | 'target.strategyHelp.api'
    | 'target.strategyHelp.ssh_docker'
    | 'target.strategyHelp.ssh_package'
    | 'target.strategyHelp.ssh_binary';

  const confirmedKey = saved?.sshHostKey ?? '';
  const pendingKey = trust && probed ? probed.fingerprint : form.hostKeyFingerprint;

  const save = async () => {
    setSaving(true);
    try {
      const result = await saveTarget(component.kind, component.refUuid, {
        ...form,
        hostKeyFingerprint: trust && probed ? probed.fingerprint : form.hostKeyFingerprint,
      });
      setSaved(result);
      setForm((f) => ({ ...f, password: '', privateKey: '', passphrase: '', clearSecret: false, hostKeyFingerprint: null }));
      setTrust(false);
      setProbed(null);
      addToast(tExt('target.saved', {}), 'success');
      onSaved();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    setRemoving(true);
    try {
      await deleteTarget(component.kind, component.refUuid);
      addToast(tExt('target.deleted', {}), 'success');
      onSaved();
      onClose();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setRemoving(false);
    }
  };

  const probe = async () => {
    setProbing(true);
    setProbed(null);
    setTrust(false);
    try {
      const key = await probeHostKey(component.kind, component.refUuid, form.sshHost, form.sshPort);
      setProbed(key);
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setProbing(false);
    }
  };

  const test = async () => {
    setTesting(true);
    setTestOutput(null);
    try {
      const result = await testTarget(component.kind, component.refUuid);
      setTestOutput(result.output);
      if (result.ok) addToast(tExt('target.testOk', {}), 'success');
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setTesting(false);
    }
  };

  const detect = async () => {
    setDetecting(true);
    try {
      const result = await detectTarget(component.kind, component.refUuid);
      setDetected(result.detected);
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setDetecting(false);
    }
  };

  const applySuggestion = () => {
    if (!detected) return;
    set({
      strategy: detected.suggestedStrategy || form.strategy,
      composeDir: detected.composeDir || form.composeDir,
      composeService: detected.composeService || form.composeService,
      serviceName: detected.unit || form.serviceName,
      binaryPath: detected.binary || detected.execStart || form.binaryPath,
    });
    addToast(tExt('target.applied', {}), 'info');
  };

  return (
    <Modal opened={opened} onClose={onClose} title={tExt('target.title', { name: component.name })} size='xl'>
      <Stack gap='md'>
        <Select
          label={tExt('target.strategy', {})}
          description={tExt(helpKey, {})}
          data={strategyOptions}
          value={form.strategy}
          onChange={(v) => v && set({ strategy: v })}
          allowDeselect={false}
        />

        {usesSsh && (
          <Stack gap='sm'>
            <Text fw={600} size='sm'>
              {tExt('target.sshSection', {})}
            </Text>
            <SimpleGrid cols={{ base: 1, md: 2 }}>
              <TextInput
                label={tExt('target.host', {})}
                description={component.host ? tExt('target.hostDefault', { host: component.host }) : undefined}
                value={form.sshHost}
                onChange={(ev) => set({ sshHost: ev.currentTarget.value })}
              />
              <NumberInput
                label={tExt('target.port', {})}
                min={1}
                max={65535}
                value={form.sshPort}
                onChange={(v) => set({ sshPort: typeof v === 'number' ? v : Number(v) || 22 })}
              />
            </SimpleGrid>

            <Select
              label={tExt('target.profile', {})}
              description={profiles.length === 0 ? tExt('target.profileNone', {}) : tExt('target.profileHelp', {})}
              data={profileOptions}
              value={form.profileId ?? ''}
              onChange={(v) => set({ profileId: v ? v : null })}
              allowDeselect={false}
            />

            {usesProfile ? (
              <Alert color='blue' icon={<FontAwesomeIcon icon={faKey} />}>
                <Text size='sm'>
                  {profile
                    ? tExt('target.profileSummary', {
                        name: profile.name,
                        user: profile.sshUser,
                        auth: profile.sshAuth === 'key' ? tExt('target.authKey', {}) : tExt('target.authPassword', {}),
                        sudo: profile.useSudo ? ' · sudo' : '',
                      })
                    : tExt('target.profileLoading', {})}
                </Text>
              </Alert>
            ) : (
              <>
                <SimpleGrid cols={{ base: 1, md: 2 }}>
                  <TextInput label={tExt('target.user', {})} value={form.sshUser} onChange={(ev) => set({ sshUser: ev.currentTarget.value })} />
                  <Select
                    label={tExt('target.auth', {})}
                    data={authOptions}
                    value={form.sshAuth}
                    onChange={(v) => v && set({ sshAuth: v })}
                    allowDeselect={false}
                  />
                </SimpleGrid>
                {form.sshAuth === 'password' && (
                  <PasswordInput
                    label={tExt('target.password', {})}
                    description={saved?.hasSecret ? tExt('target.passwordKeep', {}) : undefined}
                    value={form.password}
                    onChange={(ev) => set({ password: ev.currentTarget.value })}
                    disabled={form.clearSecret}
                  />
                )}
                {form.sshAuth === 'key' && (
                  <>
                    <TextArea
                      label={tExt('target.privateKey', {})}
                      description={saved?.hasSecret ? tExt('target.privateKeyKeep', {}) : undefined}
                      placeholder='-----BEGIN OPENSSH PRIVATE KEY-----'
                      autosize
                      minRows={3}
                      maxRows={10}
                      value={form.privateKey}
                      onChange={(ev) => set({ privateKey: ev.currentTarget.value })}
                      disabled={form.clearSecret}
                      className='font-mono'
                    />
                    <PasswordInput
                      label={tExt('target.passphrase', {})}
                      value={form.passphrase}
                      onChange={(ev) => set({ passphrase: ev.currentTarget.value })}
                      disabled={form.clearSecret}
                    />
                  </>
                )}
                {saved?.hasSecret && (
                  <Switch
                    label={tExt('target.clearSecret', {})}
                    checked={form.clearSecret}
                    onChange={(ev) => set({ clearSecret: ev.currentTarget.checked })}
                  />
                )}
                <Switch label={tExt('target.useSudo', {})} checked={form.useSudo} onChange={(ev) => set({ useSudo: ev.currentTarget.checked })} />
              </>
            )}

            <Stack gap={6}>
              <Text size='sm' fw={500}>
                {tExt('target.hostKey', {})}
              </Text>
              {confirmedKey && pendingKey !== '' ? (
                <Group gap='xs' wrap='wrap'>
                  <Badge size='sm' color='green' variant='light' leftSection={<FontAwesomeIcon icon={faKey} size='xs' />}>
                    {tExt('target.hostKeyConfirmed', { fingerprint: confirmedKey })}
                  </Badge>
                  <Button size='compact-xs' variant='subtle' color='gray' onClick={() => set({ hostKeyFingerprint: '' })}>
                    {tExt('target.hostKeyClear', {})}
                  </Button>
                </Group>
              ) : (
                <Text size='xs' c='dimmed'>
                  {tExt('target.hostKeyNone', {})}
                </Text>
              )}
              <Group gap='xs' align='flex-start' wrap='wrap'>
                <Button
                  size='compact-sm'
                  variant='default'
                  loading={probing}
                  disabled={!form.sshHost.trim()}
                  leftSection={<FontAwesomeIcon icon={faKey} />}
                  onClick={probe}
                >
                  {probing ? tExt('target.probing', {}) : tExt('target.probe', {})}
                </Button>
              </Group>
              {probed && (
                <Alert color='blue'>
                  <Stack gap='xs'>
                    <Text size='sm' className='break-all'>
                      {tExt('target.probed', { algorithm: probed.algorithm, fingerprint: probed.fingerprint })}
                    </Text>
                    <Switch label={tExt('target.trustKey', {})} checked={trust} onChange={(ev) => setTrust(ev.currentTarget.checked)} />
                  </Stack>
                </Alert>
              )}
            </Stack>
          </Stack>
        )}

        {(form.strategy === 'ssh_docker' || form.strategy === 'auto') && (
          <SimpleGrid cols={{ base: 1, md: 2 }}>
            <TextInput
              label={tExt('target.composeDir', {})}
              description={tExt('target.composeDirHelp', {})}
              placeholder='/root/calagopus-wings'
              value={form.composeDir}
              onChange={(ev) => set({ composeDir: ev.currentTarget.value })}
            />
            <TextInput
              label={tExt('target.composeService', {})}
              description={tExt('target.composeServiceHelp', {})}
              placeholder={component.kind === 'db_agent' ? 'db-agent' : 'wings'}
              value={form.composeService}
              onChange={(ev) => set({ composeService: ev.currentTarget.value })}
            />
          </SimpleGrid>
        )}
        {form.strategy !== 'ssh_docker' && (
          <SimpleGrid cols={{ base: 1, md: 2 }}>
            <TextInput
              label={tExt('target.serviceName', {})}
              description={tExt('target.serviceNameHelp', {})}
              placeholder={component.kind === 'db_agent' ? 'db-agent' : 'wings'}
              value={form.serviceName}
              onChange={(ev) => set({ serviceName: ev.currentTarget.value })}
            />
            {(form.strategy === 'ssh_binary' || form.strategy === 'auto') && (
              <TextInput
                label={tExt('target.binaryPath', {})}
                description={tExt('target.binaryPathHelp', {})}
                placeholder={component.kind === 'db_agent' ? '/usr/local/bin/calagopus-db-agent' : '/usr/local/bin/wings'}
                value={form.binaryPath}
                onChange={(ev) => set({ binaryPath: ev.currentTarget.value })}
              />
            )}
            {(form.strategy === 'api' || form.strategy === 'auto') && (
              <TextInput
                label={tExt('target.restartCommand', {})}
                description={tExt('target.restartCommandHelp', {})}
                placeholder={`systemctl restart ${component.kind === 'db_agent' ? 'db-agent' : 'wings'}`}
                value={form.restartCommand}
                onChange={(ev) => set({ restartCommand: ev.currentTarget.value })}
              />
            )}
          </SimpleGrid>
        )}

        <Switch
          label={tExt('target.autoUpdate', {})}
          description={tExt('target.autoUpdateHelp', {})}
          checked={form.autoUpdate}
          onChange={(ev) => set({ autoUpdate: ev.currentTarget.checked })}
        />

        {usesSsh && (
          <Stack gap='xs'>
            <Group gap='xs' wrap='wrap'>
              <Button
                size='compact-sm'
                variant='default'
                loading={testing}
                disabled={!saved}
                leftSection={<FontAwesomeIcon icon={faPlug} />}
                onClick={test}
              >
                {testing ? tExt('target.testing', {}) : tExt('target.test', {})}
              </Button>
              <Button
                size='compact-sm'
                variant='default'
                loading={detecting}
                disabled={!saved}
                leftSection={<FontAwesomeIcon icon={faMagnifyingGlass} />}
                onClick={detect}
              >
                {detecting ? tExt('target.detecting', {}) : tExt('target.detect', {})}
              </Button>
              {!saved && (
                <Text size='xs' c='dimmed'>
                  {tExt('target.saveFirst', {})}
                </Text>
              )}
            </Group>
            {testOutput !== null && (
              <Code block className='text-xs whitespace-pre-wrap'>
                {testOutput || '—'}
              </Code>
            )}
            {detected && (
              <Alert color='slate'>
                <Stack gap='xs'>
                  <DetectedView detected={detected} />
                  {detected.suggestedStrategy && (
                    <Button size='compact-xs' variant='light' onClick={applySuggestion} className='self-start'>
                      {tExt('target.applySuggestion', {})}
                    </Button>
                  )}
                </Stack>
              </Alert>
            )}
          </Stack>
        )}

        <Group justify='space-between' wrap='wrap'>
          <div>
            {saved && (
              <Button color='red' variant='subtle' loading={removing} leftSection={<FontAwesomeIcon icon={faTrash} />} onClick={remove}>
                {tExt('target.remove', {})}
              </Button>
            )}
          </div>
          <Group gap='xs'>
            <Button variant='default' onClick={onClose}>
              {tExt('common.close', {})}
            </Button>
            <Button loading={saving} onClick={save}>
              {tExt('common.save', {})}
            </Button>
          </Group>
        </Group>
      </Stack>
    </Modal>
  );
}

export { TargetModal };
