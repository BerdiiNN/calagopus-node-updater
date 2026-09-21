import { axiosInstance } from '@/api/axios.ts';
import { parseFromApi } from '@/lib/api-transform.ts';
import {
  type BulkResult,
  type DetectResult,
  type Overview,
  type Profile,
  type Run,
  type RunDetail,
  type Settings,
  type Target,
  bulkResultSchema,
  detectResultSchema,
  overviewSchema,
  profileSchema,
  profilesSchema,
  runDetailSchema,
  runSchema,
  runsSchema,
  settingsSchema,
  targetSchema,
  targetsSchema,
} from '../schemas.ts';

const ADMIN = '/api/admin/extensions/com.pineriver.nodeupdater';

export async function getOverview(): Promise<Overview> {
  const { data } = await axiosInstance.get(`${ADMIN}/overview`);
  return parseFromApi(overviewSchema, data);
}

export async function checkNow(): Promise<Overview> {
  const { data } = await axiosInstance.post(`${ADMIN}/check`);
  return parseFromApi(overviewSchema, data);
}

export async function getTargets(): Promise<Target[]> {
  const { data } = await axiosInstance.get(`${ADMIN}/targets`);
  return parseFromApi(targetsSchema, data).targets;
}

export interface TargetInput {
  strategy: string;
  sshHost: string;
  sshPort: number;
  sshUser: string;
  sshAuth: string;
  /** Empty = keep the stored one. */
  password: string;
  /** Empty = keep the stored one. */
  privateKey: string;
  passphrase: string;
  clearSecret: boolean;
  /** `null` keeps the confirmed key, `''` clears it. */
  hostKeyFingerprint: string | null;
  useSudo: boolean;
  composeDir: string;
  composeService: string;
  serviceName: string;
  binaryPath: string;
  restartCommand: string;
  autoUpdate: boolean;
  /** Credential profile to log in with; `null` = the target's own credentials. */
  profileId: string | null;
}

export async function saveTarget(kind: string, ref: string, input: TargetInput): Promise<Target> {
  const { data } = await axiosInstance.put(`${ADMIN}/targets/${kind}/${ref}`, {
    profile_id: input.profileId,
    strategy: input.strategy,
    ssh_host: input.sshHost.trim(),
    ssh_port: input.sshPort,
    ssh_user: input.sshUser.trim(),
    ssh_auth: input.sshAuth,
    password: input.password,
    private_key: input.privateKey,
    passphrase: input.passphrase,
    clear_secret: input.clearSecret,
    host_key_fingerprint: input.hostKeyFingerprint,
    use_sudo: input.useSudo,
    compose_dir: input.composeDir.trim(),
    compose_service: input.composeService.trim(),
    service_name: input.serviceName.trim(),
    binary_path: input.binaryPath.trim(),
    restart_command: input.restartCommand.trim(),
    auto_update: input.autoUpdate,
  });
  return parseFromApi(targetSchema, data);
}

export async function deleteTarget(kind: string, ref: string): Promise<void> {
  await axiosInstance.delete(`${ADMIN}/targets/${kind}/${ref}`);
}

export interface HostKey {
  fingerprint: string;
  algorithm: string;
}

export async function probeHostKey(kind: string, ref: string, host: string, port: number): Promise<HostKey> {
  const { data } = await axiosInstance.post(`${ADMIN}/targets/${kind}/${ref}/probe`, { host: host.trim(), port });
  return { fingerprint: String(data?.fingerprint ?? ''), algorithm: String(data?.algorithm ?? '') };
}

export async function testTarget(kind: string, ref: string): Promise<{ ok: boolean; output: string }> {
  const { data } = await axiosInstance.post(`${ADMIN}/targets/${kind}/${ref}/test`);
  return { ok: Boolean(data?.ok), output: String(data?.output ?? '') };
}

export async function detectTarget(kind: string, ref: string): Promise<DetectResult> {
  const { data } = await axiosInstance.post(`${ADMIN}/targets/${kind}/${ref}/detect`);
  return parseFromApi(detectResultSchema, data);
}

export async function queueUpdate(kind: string, refUuid: string, version?: string, force = false): Promise<Run> {
  const { data } = await axiosInstance.post(`${ADMIN}/updates`, {
    kind,
    ref_uuid: refUuid,
    version: version ?? null,
    force,
  });
  return parseFromApi(runSchema, data);
}

export async function queueBulk(kind?: string, force = false): Promise<BulkResult> {
  const { data } = await axiosInstance.post(`${ADMIN}/updates/bulk`, { kind: kind ?? null, force });
  return parseFromApi(bulkResultSchema, data);
}

export async function getRuns(limit = 50, offset = 0): Promise<Run[]> {
  const { data } = await axiosInstance.get(`${ADMIN}/runs`, { params: { limit, offset } });
  return parseFromApi(runsSchema, data).runs;
}

export async function getRun(id: string): Promise<RunDetail> {
  const { data } = await axiosInstance.get(`${ADMIN}/runs/${id}`);
  return parseFromApi(runDetailSchema, data);
}

export async function cancelRun(id: string): Promise<void> {
  await axiosInstance.post(`${ADMIN}/runs/${id}/cancel`);
}

export async function deleteRun(id: string): Promise<void> {
  await axiosInstance.delete(`${ADMIN}/runs/${id}`);
}

export interface ProfileInput {
  name: string;
  sshUser: string;
  sshAuth: string;
  /** Empty = keep the stored one (update only). */
  password: string;
  /** Empty = keep the stored one (update only). */
  privateKey: string;
  passphrase: string;
  useSudo: boolean;
}

function profileBody(input: ProfileInput) {
  return {
    name: input.name.trim(),
    ssh_user: input.sshUser.trim(),
    ssh_auth: input.sshAuth,
    password: input.password,
    private_key: input.privateKey,
    passphrase: input.passphrase,
    use_sudo: input.useSudo,
  };
}

export async function getProfiles(): Promise<Profile[]> {
  const { data } = await axiosInstance.get(`${ADMIN}/profiles`);
  return parseFromApi(profilesSchema, data).profiles;
}

export async function createProfile(input: ProfileInput): Promise<Profile> {
  const { data } = await axiosInstance.post(`${ADMIN}/profiles`, profileBody(input));
  return parseFromApi(profileSchema, data);
}

export async function updateProfile(id: string, input: ProfileInput): Promise<Profile> {
  const { data } = await axiosInstance.put(`${ADMIN}/profiles/${id}`, profileBody(input));
  return parseFromApi(profileSchema, data);
}

export async function deleteProfile(id: string): Promise<void> {
  await axiosInstance.delete(`${ADMIN}/profiles/${id}`);
}

export async function getSettings(): Promise<Settings> {
  const { data } = await axiosInstance.get(`${ADMIN}/settings`);
  return parseFromApi(settingsSchema, data);
}

export interface SettingsInput {
  checkIntervalMinutes: number;
  includePrereleases: boolean;
  allowAheadOfPanel: boolean;
  notifyEmail: boolean;
  notifyOnSuccess: boolean;
  notifyDiscordUrl: string;
  notifyWebhookUrl: string;
  /** Empty = keep the stored secret. */
  notifyWebhookSecret: string;
  clearWebhookSecret: boolean;
  remindAfterHours: number;
  sshConnectTimeoutSecs: number;
  updateTimeoutSecs: number;
  verifyTimeoutSecs: number;
}

export async function saveSettings(input: SettingsInput): Promise<void> {
  await axiosInstance.put(`${ADMIN}/settings`, {
    check_interval_minutes: input.checkIntervalMinutes,
    include_prereleases: input.includePrereleases,
    allow_ahead_of_panel: input.allowAheadOfPanel,
    notify_email: input.notifyEmail,
    notify_on_success: input.notifyOnSuccess,
    notify_discord_url: input.notifyDiscordUrl.trim(),
    notify_webhook_url: input.notifyWebhookUrl.trim(),
    notify_webhook_secret: input.notifyWebhookSecret,
    clear_webhook_secret: input.clearWebhookSecret,
    remind_after_hours: input.remindAfterHours,
    ssh_connect_timeout_secs: input.sshConnectTimeoutSecs,
    update_timeout_secs: input.updateTimeoutSecs,
    verify_timeout_secs: input.verifyTimeoutSecs,
  });
}

export async function testNotification(channel: 'email' | 'discord' | 'webhook'): Promise<void> {
  await axiosInstance.post(`${ADMIN}/settings/test`, { channel });
}

const api = {
  getOverview,
  checkNow,
  getTargets,
  saveTarget,
  deleteTarget,
  probeHostKey,
  testTarget,
  detectTarget,
  queueUpdate,
  queueBulk,
  getRuns,
  getRun,
  cancelRun,
  deleteRun,
  getProfiles,
  createProfile,
  updateProfile,
  deleteProfile,
  getSettings,
  saveSettings,
  testNotification,
};

export default api;
export { api as nodeUpdaterApi };
