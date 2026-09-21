ALTER TABLE com_pineriver_nodeupdater_targets DROP COLUMN IF EXISTS profile_id;
DROP TABLE IF EXISTS com_pineriver_nodeupdater_profiles CASCADE;
