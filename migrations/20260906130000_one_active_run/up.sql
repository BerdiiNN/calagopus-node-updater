-- At most one queued/running update per node, enforced by the database so two concurrent
-- requests (or the checker and an admin) cannot both queue a run.
CREATE UNIQUE INDEX IF NOT EXISTS com_pineriver_nodeupdater_runs_one_active
  ON com_pineriver_nodeupdater_runs (kind, ref_uuid)
  WHERE status IN ('queued', 'running');
