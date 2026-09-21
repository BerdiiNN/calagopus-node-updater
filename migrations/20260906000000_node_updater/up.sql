-- How each Wings node / DB Agent host gets updated. One row per (kind, ref_uuid); a node
-- without a row is "check only" (its version is still compared, it just cannot be updated).
-- SSH secrets are stored encrypted with the panel's APP_ENCRYPTION_KEY (base64 blob).
CREATE TABLE IF NOT EXISTS com_pineriver_nodeupdater_targets (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  -- wings | db_agent
  kind TEXT NOT NULL,
  -- nodes.uuid or database_agent_hosts.uuid (not a foreign key: the referenced table
  -- differs per kind; orphans are hidden by the inventory join and pruned by the checker)
  ref_uuid UUID NOT NULL,
  -- auto | api | ssh_docker | ssh_package | ssh_binary
  strategy TEXT NOT NULL DEFAULT 'auto',
  ssh_host TEXT NOT NULL DEFAULT '',
  ssh_port INTEGER NOT NULL DEFAULT 22,
  ssh_user TEXT NOT NULL DEFAULT 'root',
  -- none | password | key
  ssh_auth TEXT NOT NULL DEFAULT 'none',
  -- encrypted JSON: {"password": ".."} or {"pem": "..", "passphrase": ".."}
  ssh_secret TEXT,
  -- SHA256:<base64> host-key fingerprint confirmed by the operator (trust on first use)
  ssh_host_key TEXT,
  use_sudo BOOLEAN NOT NULL DEFAULT FALSE,
  compose_dir TEXT NOT NULL DEFAULT '',
  compose_service TEXT NOT NULL DEFAULT '',
  -- systemd unit / OpenRC service name (wings, db-agent)
  service_name TEXT NOT NULL DEFAULT '',
  binary_path TEXT NOT NULL DEFAULT '',
  -- for the api strategy: what the daemon runs after swapping its binary
  restart_command TEXT NOT NULL DEFAULT '',
  auto_update BOOLEAN NOT NULL DEFAULT FALSE,
  -- last detection result (JSON) for the UI
  detected JSONB,
  detected_at TIMESTAMP WITH TIME ZONE,
  created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
  updated TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
  UNIQUE (kind, ref_uuid)
);

-- One row per update attempt, with the streamed log.
CREATE TABLE IF NOT EXISTS com_pineriver_nodeupdater_runs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  kind TEXT NOT NULL,
  ref_uuid UUID NOT NULL,
  ref_name TEXT NOT NULL DEFAULT '',
  strategy TEXT NOT NULL,
  from_version TEXT,
  to_version TEXT NOT NULL,
  -- queued | running | succeeded | failed | cancelled
  status TEXT NOT NULL DEFAULT 'queued',
  log TEXT NOT NULL DEFAULT '',
  error TEXT,
  -- manual:<username> | auto | bulk:<username>
  triggered_by TEXT NOT NULL DEFAULT '',
  force BOOLEAN NOT NULL DEFAULT FALSE,
  started_at TIMESTAMP WITH TIME ZONE,
  finished_at TIMESTAMP WITH TIME ZONE,
  created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
  updated TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS com_pineriver_nodeupdater_runs_created
  ON com_pineriver_nodeupdater_runs (created DESC);

CREATE INDEX IF NOT EXISTS com_pineriver_nodeupdater_runs_active
  ON com_pineriver_nodeupdater_runs (kind, ref_uuid)
  WHERE status IN ('queued', 'running');

-- Small key/value store: latest-version cache, last inventory snapshot, notification markers.
CREATE TABLE IF NOT EXISTS com_pineriver_nodeupdater_state (
  key TEXT PRIMARY KEY,
  value JSONB NOT NULL,
  updated TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- SHA-256 of release assets the panel has already downloaded once (the Wings/DB Agent
-- upgrade API requires the checksum and GitHub publishes none).
CREATE TABLE IF NOT EXISTS com_pineriver_nodeupdater_checksums (
  kind TEXT NOT NULL,
  version TEXT NOT NULL,
  architecture TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  size BIGINT NOT NULL DEFAULT 0,
  created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
  PRIMARY KEY (kind, version, architecture)
);
