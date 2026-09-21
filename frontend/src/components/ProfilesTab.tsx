import { faKey, faPen, faPlus, faRotate, faTrash } from '@fortawesome/free-solid-svg-icons';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { Group, Loader, Stack, Table } from '@mantine/core';
import { useCallback, useEffect, useState } from 'react';
import { httpErrorToHuman } from '@/api/axios.ts';
import Alert from '@/elements/Alert.tsx';
import Badge from '@/elements/Badge.tsx';
import Button from '@/elements/Button.tsx';
import Card from '@/elements/Card.tsx';
import PasswordInput from '@/elements/input/PasswordInput.tsx';
import Select from '@/elements/input/Select.tsx';
import Switch from '@/elements/input/Switch.tsx';
import TextArea from '@/elements/input/TextArea.tsx';
import TextInput from '@/elements/input/TextInput.tsx';
import { Modal } from '@/elements/modals/Modal.tsx';
import Text from '@/elements/Text.tsx';
import { useToast } from '@/providers/ToastProvider.tsx';
import { type ProfileInput, createProfile, deleteProfile, getProfiles, updateProfile } from '../lib/api/index.ts';
import { relativeTime } from '../lib/format.ts';
import type { Profile } from '../lib/schemas.ts';
import { useExtTranslations } from '../translations';

function emptyForm(profile?: Profile | null): ProfileInput {
  return {
    name: profile?.name ?? '',
    sshUser: profile?.sshUser ?? 'root',
    sshAuth: profile?.sshAuth ?? 'key',
    password: '',
    privateKey: '',
    passphrase: '',
    useSudo: profile?.useSudo ?? false,
  };
}

function ProfileModal({
  profile,
  opened,
  onClose,
  onSaved,
}: {
  /** `null` = create a new profile. */
  profile: Profile | null;
  opened: boolean;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t: tExt } = useExtTranslations();
  const { addToast } = useToast();
  const [form, setForm] = useState<ProfileInput>(emptyForm(null));
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (opened) setForm(emptyForm(profile));
  }, [opened, profile]);

  const set = (patch: Partial<ProfileInput>) => setForm((f) => ({ ...f, ...patch }));
  const editing = profile !== null;
  const authChanged = editing && profile.sshAuth !== form.sshAuth;
  const secretTyped = form.sshAuth === 'password' ? form.password.length > 0 : form.privateKey.trim().length > 0;
  const needsSecret = !editing || authChanged;

  const save = async () => {
    setSaving(true);
    try {
      if (editing) {
        await updateProfile(profile.id, form);
      } else {
        await createProfile(form);
      }
      addToast(tExt('profiles.saved', {}), 'success');
      onSaved();
      onClose();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title={editing ? tExt('profiles.editTitle', { name: profile.name }) : tExt('profiles.newTitle', {})}
      size='lg'
    >
      <Stack gap='md'>
        <TextInput
          label={tExt('profiles.name', {})}
          description={tExt('profiles.nameHelp', {})}
          value={form.name}
          onChange={(ev) => set({ name: ev.currentTarget.value })}
          maxLength={64}
        />
        <Group grow align='flex-start'>
          <TextInput label={tExt('target.user', {})} value={form.sshUser} onChange={(ev) => set({ sshUser: ev.currentTarget.value })} />
          <Select
            label={tExt('target.auth', {})}
            data={[
              { value: 'key', label: tExt('target.authKey', {}) },
              { value: 'password', label: tExt('target.authPassword', {}) },
            ]}
            value={form.sshAuth}
            onChange={(v) => v && set({ sshAuth: v })}
            allowDeselect={false}
          />
        </Group>
        {form.sshAuth === 'password' ? (
          <PasswordInput
            label={tExt('target.password', {})}
            description={editing && !needsSecret ? tExt('target.passwordKeep', {}) : undefined}
            value={form.password}
            onChange={(ev) => set({ password: ev.currentTarget.value })}
          />
        ) : (
          <>
            <TextArea
              label={tExt('target.privateKey', {})}
              description={editing && !needsSecret ? tExt('target.privateKeyKeep', {}) : undefined}
              placeholder='-----BEGIN OPENSSH PRIVATE KEY-----'
              autosize
              minRows={3}
              maxRows={10}
              value={form.privateKey}
              onChange={(ev) => set({ privateKey: ev.currentTarget.value })}
              className='font-mono'
            />
            <PasswordInput
              label={tExt('target.passphrase', {})}
              value={form.passphrase}
              onChange={(ev) => set({ passphrase: ev.currentTarget.value })}
            />
          </>
        )}
        {authChanged && (
          <Alert color='yellow'>
            <Text size='sm'>{tExt('profiles.authChanged', {})}</Text>
          </Alert>
        )}
        <Switch label={tExt('target.useSudo', {})} checked={form.useSudo} onChange={(ev) => set({ useSudo: ev.currentTarget.checked })} />
        <Group justify='flex-end'>
          <Button variant='default' onClick={onClose}>
            {tExt('common.cancel', {})}
          </Button>
          <Button loading={saving} disabled={!form.name.trim() || (needsSecret && !secretTyped)} onClick={save}>
            {tExt('common.save', {})}
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

export default function ProfilesTab() {
  const { t: tExt } = useExtTranslations();
  const { addToast } = useToast();
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState<Profile | null | undefined>(undefined);
  const [deleting, setDeleting] = useState<Profile | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);

  const load = useCallback(() => {
    setLoading(true);
    getProfiles()
      .then((rows) => {
        setProfiles(rows);
        setError(null);
      })
      .catch((err) => setError(httpErrorToHuman(err)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const remove = async () => {
    if (!deleting) return;
    setDeleteBusy(true);
    try {
      await deleteProfile(deleting.id);
      addToast(tExt('profiles.deleted', {}), 'success');
      setDeleting(null);
      load();
    } catch (err) {
      addToast(httpErrorToHuman(err), 'error');
    } finally {
      setDeleteBusy(false);
    }
  };

  if (profiles === null && !error) {
    return (
      <Group justify='center' className='py-10'>
        <Loader size='sm' />
      </Group>
    );
  }
  if (profiles === null) {
    return (
      <Alert color='red' title={tExt('profiles.couldNotLoad', {})}>
        <Text size='sm'>{error}</Text>
      </Alert>
    );
  }

  return (
    <Stack gap='md'>
      <Card>
        <Group justify='space-between' wrap='wrap' gap='sm' mb='sm'>
          <Text size='sm' c='dimmed' className='max-w-3xl'>
            {tExt('profiles.help', {})}
          </Text>
          <Group gap='xs'>
            <Button size='compact-sm' variant='default' loading={loading} onClick={load} leftSection={<FontAwesomeIcon icon={faRotate} />}>
              {tExt('common.refresh', {})}
            </Button>
            <Button size='compact-sm' onClick={() => setEditing(null)} leftSection={<FontAwesomeIcon icon={faPlus} />}>
              {tExt('profiles.new', {})}
            </Button>
          </Group>
        </Group>
        {profiles.length === 0 ? (
          <Text size='sm' c='dimmed'>
            {tExt('profiles.none', {})}
          </Text>
        ) : (
          <Table.ScrollContainer minWidth={700}>
            <Table striped highlightOnHover verticalSpacing={6} fz='sm'>
              <Table.Thead>
                <Table.Tr>
                  <Table.Th>{tExt('profiles.name', {})}</Table.Th>
                  <Table.Th>{tExt('target.user', {})}</Table.Th>
                  <Table.Th>{tExt('target.auth', {})}</Table.Th>
                  <Table.Th>{tExt('profiles.sudo', {})}</Table.Th>
                  <Table.Th>{tExt('profiles.usedBy', {})}</Table.Th>
                  <Table.Th>{tExt('profiles.updated', {})}</Table.Th>
                  <Table.Th />
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {profiles.map((profile) => (
                  <Table.Tr key={profile.id}>
                    <Table.Td>
                      <Group gap={6} wrap='nowrap'>
                        <FontAwesomeIcon icon={faKey} className='text-gray-400' />
                        <Text size='sm' fw={500}>
                          {profile.name}
                        </Text>
                      </Group>
                    </Table.Td>
                    <Table.Td className='font-mono text-xs'>{profile.sshUser}</Table.Td>
                    <Table.Td>
                      <Badge size='xs' variant='light' color={profile.sshAuth === 'key' ? 'blue' : 'gray'}>
                        {profile.sshAuth === 'key' ? tExt('target.authKey', {}) : tExt('target.authPassword', {})}
                      </Badge>
                      {!profile.hasSecret && (
                        <Badge size='xs' variant='light' color='red' ml={6}>
                          {tExt('profiles.noSecret', {})}
                        </Badge>
                      )}
                    </Table.Td>
                    <Table.Td>{profile.useSudo ? 'sudo' : '—'}</Table.Td>
                    <Table.Td>{tExt('profiles.nodes', { n: profile.targets })}</Table.Td>
                    <Table.Td>{relativeTime(profile.updated)}</Table.Td>
                    <Table.Td>
                      <Group gap={4} wrap='nowrap' justify='flex-end'>
                        <Button size='compact-xs' variant='default' leftSection={<FontAwesomeIcon icon={faPen} />} onClick={() => setEditing(profile)}>
                          {tExt('profiles.edit', {})}
                        </Button>
                        <Button size='compact-xs' color='red' variant='subtle' onClick={() => setDeleting(profile)}>
                          <FontAwesomeIcon icon={faTrash} />
                        </Button>
                      </Group>
                    </Table.Td>
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>
          </Table.ScrollContainer>
        )}
      </Card>

      <ProfileModal
        profile={editing ?? null}
        opened={editing !== undefined}
        onClose={() => setEditing(undefined)}
        onSaved={load}
      />

      <Modal opened={deleting !== null} onClose={() => setDeleting(null)} title={tExt('profiles.deleteTitle', { name: deleting?.name ?? '' })}>
        <Stack gap='md'>
          <Text size='sm'>
            {deleting && deleting.targets > 0
              ? tExt('profiles.deleteUsed', { n: deleting.targets })
              : tExt('profiles.deleteBody', {})}
          </Text>
          <Group justify='flex-end'>
            <Button variant='default' onClick={() => setDeleting(null)}>
              {tExt('common.cancel', {})}
            </Button>
            <Button color='red' loading={deleteBusy} onClick={remove}>
              {tExt('common.delete', {})}
            </Button>
          </Group>
        </Stack>
      </Modal>
    </Stack>
  );
}

export { ProfilesTab };
