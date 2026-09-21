-- Reusable SSH credentials. A target that references a profile takes user, authentication,
-- secret and sudo from it; host, port and the confirmed host key stay per target.
CREATE TABLE IF NOT EXISTS com_pineriver_nodeupdater_profiles (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name TEXT NOT NULL UNIQUE,
  ssh_user TEXT NOT NULL DEFAULT 'root',
  -- password | key
  ssh_auth TEXT NOT NULL DEFAULT 'key',
  -- encrypted JSON: {"password": ".."} or {"pem": "..", "passphrase": ".."}
  ssh_secret TEXT,
  use_sudo BOOLEAN NOT NULL DEFAULT FALSE,
  created TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
  updated TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

ALTER TABLE com_pineriver_nodeupdater_targets
  ADD COLUMN IF NOT EXISTS profile_id UUID REFERENCES com_pineriver_nodeupdater_profiles(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS com_pineriver_nodeupdater_targets_profile
  ON com_pineriver_nodeupdater_targets (profile_id);
