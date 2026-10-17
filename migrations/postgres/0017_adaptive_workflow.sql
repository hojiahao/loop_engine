-- Adaptive workflows add a real leased proposal job, not a factor trial.
-- Preserve v1/v2 rows and immutable receipts. Rollback disables v3 writers;
-- retain this schema and use a compatible reader rather than down-migrating.
ALTER TABLE jobs DROP CONSTRAINT jobs_kind_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_kind_check CHECK (kind BETWEEN 1 AND 8);

ALTER TABLE research_runs DROP CONSTRAINT research_runs_workflow_version_check;
ALTER TABLE research_runs ADD CONSTRAINT research_runs_workflow_version_check
    CHECK (workflow_version IN (1, 2, 3));
ALTER TABLE research_runs DROP CONSTRAINT research_runs_status_check;
ALTER TABLE research_runs ADD CONSTRAINT research_runs_status_check
    CHECK (status BETWEEN 1 AND 6);
ALTER TABLE research_runs ADD CONSTRAINT research_runs_adaptive_status
    CHECK (status <> 6 OR workflow_version = 3);

ALTER TABLE run_transitions DROP CONSTRAINT run_transitions_stage_check;
ALTER TABLE run_transitions ADD CONSTRAINT run_transitions_stage_check
    CHECK (stage BETWEEN 1 AND 4);

-- A perturbation replaces Discovery, so three children per round and the
-- existing 193-transition ceiling remain sufficient.
CREATE OR REPLACE FUNCTION protect_run_transition() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP <> 'INSERT'
       OR NOT EXISTS (
           SELECT 1 FROM research_runs r WHERE r.run_id = NEW.run_id
           AND r.workflow_version IN (2, 3) AND r.revision = NEW.revision
           AND r.current_job_id = NEW.current_job_id
           AND current_setting('loop.run_writer', true) = ('v' || r.workflow_version::TEXT)
           AND (NEW.stage <> 4 OR r.workflow_version = 3)
       )
       OR (NEW.revision > 1 AND NOT EXISTS (
           SELECT 1 FROM run_transitions t WHERE t.run_id = NEW.run_id
           AND t.revision = NEW.revision - 1
       )) THEN
        RAISE EXCEPTION 'run transition requires current atomic writer' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION require_run_transition() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.workflow_version IN (2, 3) AND NOT EXISTS (
        SELECT 1 FROM run_transitions WHERE run_id = NEW.run_id
        AND revision = NEW.revision AND current_job_id = NEW.current_job_id
    ) THEN
        RAISE EXCEPTION 'numerical parent requires immutable transition' USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION fence_run_child() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.kind = 8 AND NOT EXISTS (
        SELECT 1 FROM research_runs WHERE run_id = NEW.run_id AND workflow_version = 3
    ) THEN
        RAISE EXCEPTION 'perturbation requires an adaptive parent' USING ERRCODE = '23514';
    END IF;
    IF EXISTS (SELECT 1 FROM research_runs WHERE run_id = NEW.run_id) AND
       (current_setting('loop.run_child', true) IS DISTINCT FROM NEW.job_id
        OR NOT EXISTS (SELECT 1 FROM research_runs WHERE run_id = NEW.run_id
            AND current_job_id = NEW.job_id AND status = 1
            AND current_setting('loop.run_writer', true) = ('v' || workflow_version::TEXT))) THEN
        RAISE EXCEPTION 'managed run requires atomic child reservation' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
