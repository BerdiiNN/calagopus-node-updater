import { z } from 'zod';

const optional = <T extends z.ZodTypeAny>(schema: T) =>
  schema.nullish().transform((value: z.infer<T> | null | undefined) => value ?? undefined);

export const KINDS = ['wings', 'db_agent'] as const;
export type Kind = (typeof KINDS)[number];

export const STRATEGIES = ['auto', 'api', 'ssh_docker', 'ssh_package', 'ssh_binary'] as const;
export type Strategy = (typeof STRATEGIES)[number];

export const AUTHS = ['none', 'password', 'key'] as const;
export type Auth = (typeof AUTHS)[number];

export const RUN_ACTIVE = ['queued', 'running'] as const;

export const releaseSchema = z.object({
  version: z.string(),
  tag: z.string().default(''),
  prerelease: z.boolean().default(false),
  htmlUrl: z.string().default(''),
  publishedAt: optional(z.string()),
  notes: z.string().default(''),
  assets: z.array(z.string()).default([]),
});
export type Release = z.infer<typeof releaseSchema>;

export const componentLatestSchema = z.object({
  latest: optional(releaseSchema),
  recommended: optional(z.string()),
  recent: z.array(releaseSchema).default([]),
  error: optional(z.string()),
});
export type ComponentLatest = z.infer<typeof componentLatestSchema>;

export const latestInfoSchema = z.object({
  panel: componentLatestSchema,
  wings: componentLatestSchema,
  dbAgent: componentLatestSchema,
  checkedAt: z.string().default(''),
  feedError: optional(z.string()),
});
export type LatestInfo = z.infer<typeof latestInfoSchema>;

export const detectedSchema = z.object({
  os: z.string().default(''),
  architecture: z.string().default(''),
  kernel: z.string().default(''),
  init: z.string().default(''),
  packageManager: z.string().default(''),
  packageInstalled: z.string().default(''),
  unit: z.string().default(''),
  unitActive: z.string().default(''),
  execStart: z.string().default(''),
  binary: z.string().default(''),
  binaryVersion: z.string().default(''),
  dockerContainer: z.string().default(''),
  dockerImage: z.string().default(''),
  composeDir: z.string().default(''),
  composeService: z.string().default(''),
  composeTool: z.string().default(''),
  whoami: z.string().default(''),
  sudoOk: z.boolean().default(false),
  suggestedStrategy: z.string().default(''),
  suggestionReason: z.string().default(''),
});
export type Detected = z.infer<typeof detectedSchema>;

export const profileSchema = z.object({
  id: z.string(),
  name: z.string().default(''),
  sshUser: z.string().default('root'),
  sshAuth: z.string().default('key'),
  hasSecret: z.boolean().default(false),
  useSudo: z.boolean().default(false),
  targets: z.number().default(0),
  created: z.string().default(''),
  updated: z.string().default(''),
});
export type Profile = z.infer<typeof profileSchema>;
export const profilesSchema = z.object({ profiles: z.array(profileSchema).default([]) });

export const targetSchema = z.object({
  id: z.string(),
  kind: z.string(),
  refUuid: z.string(),
  strategy: z.string().default('auto'),
  sshHost: z.string().default(''),
  sshPort: z.number().default(22),
  sshUser: z.string().default('root'),
  sshAuth: z.string().default('none'),
  hasSecret: z.boolean().default(false),
  sshHostKey: optional(z.string()),
  useSudo: z.boolean().default(false),
  composeDir: z.string().default(''),
  composeService: z.string().default(''),
  serviceName: z.string().default(''),
  binaryPath: z.string().default(''),
  restartCommand: z.string().default(''),
  autoUpdate: z.boolean().default(false),
  profileId: optional(z.string()),
  profileName: optional(z.string()),
  detected: optional(detectedSchema),
  detectedAt: optional(z.string()),
  created: z.string().default(''),
  updated: z.string().default(''),
});
export type Target = z.infer<typeof targetSchema>;

export const runSchema = z.object({
  id: z.string(),
  kind: z.string(),
  refUuid: z.string(),
  refName: z.string().default(''),
  strategy: z.string().default(''),
  fromVersion: optional(z.string()),
  toVersion: z.string().default(''),
  status: z.string().default('queued'),
  error: optional(z.string()),
  triggeredBy: z.string().default(''),
  force: z.boolean().default(false),
  startedAt: optional(z.string()),
  finishedAt: optional(z.string()),
  created: z.string().default(''),
  updated: z.string().default(''),
});
export type Run = z.infer<typeof runSchema>;

export const runDetailSchema = runSchema.extend({
  log: z.string().default(''),
});
export type RunDetail = z.infer<typeof runDetailSchema>;

export const componentSchema = z.object({
  kind: z.string(),
  refUuid: z.string(),
  name: z.string().default(''),
  url: z.string().default(''),
  location: optional(z.string()),
  host: optional(z.string()),
  installed: optional(z.string()),
  installedRaw: z.string().default(''),
  architecture: z.string().default(''),
  containerType: z.string().default(''),
  error: optional(z.string()),
  latest: optional(z.string()),
  outdated: z.boolean().default(false),
  effectiveStrategy: z.string().default(''),
  canUpdate: z.boolean().default(false),
  blockedReason: optional(z.string()),
  needsForce: z.boolean().default(false),
  target: optional(targetSchema),
  lastRun: optional(runSchema),
  activeRun: z.boolean().default(false),
});
export type ComponentStatus = z.infer<typeof componentSchema>;

export const panelStatusSchema = z.object({
  installed: z.string().default(''),
  installedRaw: z.string().default(''),
  latest: optional(z.string()),
  outdated: z.boolean().default(false),
});
export type PanelStatus = z.infer<typeof panelStatusSchema>;

export const overviewSchema = z.object({
  panel: panelStatusSchema,
  latest: latestInfoSchema,
  components: z.array(componentSchema).default([]),
  checkedAt: z.string().default(''),
  runsActive: z.number().default(0),
  allowAheadOfPanel: z.boolean().default(false),
  checkIntervalMinutes: z.number().default(60),
  panelPrimary: z.boolean().default(true),
  strategies: z.array(z.string()).default([...STRATEGIES]),
});
export type Overview = z.infer<typeof overviewSchema>;

export const settingsSchema = z.object({
  checkIntervalMinutes: z.number().default(60),
  includePrereleases: z.boolean().default(false),
  allowAheadOfPanel: z.boolean().default(false),
  notifyEmail: z.boolean().default(true),
  notifyOnSuccess: z.boolean().default(true),
  notifyDiscordUrl: z.string().default(''),
  notifyWebhookUrl: z.string().default(''),
  remindAfterHours: z.number().default(168),
  sshConnectTimeoutSecs: z.number().default(30),
  updateTimeoutSecs: z.number().default(900),
  verifyTimeoutSecs: z.number().default(180),
  webhookSecretSet: z.boolean().default(false),
  panelVersion: z.string().default(''),
});
export type Settings = z.infer<typeof settingsSchema>;

export const runsSchema = z.object({ runs: z.array(runSchema).default([]) });
export const targetsSchema = z.object({ targets: z.array(targetSchema).default([]) });

export const bulkResultSchema = z.object({
  queued: z.number().default(0),
  skipped: z.array(z.object({ name: z.string().default(''), reason: z.string().default('') })).default([]),
});
export type BulkResult = z.infer<typeof bulkResultSchema>;

export const detectResultSchema = z.object({
  detected: detectedSchema,
  raw: z.string().default(''),
});
export type DetectResult = z.infer<typeof detectResultSchema>;
