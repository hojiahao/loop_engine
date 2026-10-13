-- Numerical workflows are opt-in. Existing discovery-only rows remain v1.
-- Stop older writers before deployment; rollback disables v2 creation without
-- deleting reservations, transitions, jobs, receipts or audit evidence.
ALTER TABLE research_runs ADD COLUMN workflow_version INTEGER NOT NULL DEFAULT 1
    CHECK (workflow_version IN (1, 2));

CREATE TABLE run_transitions (
    run_id TEXT NOT NULL REFERENCES research_runs(run_id),
    revision BIGINT NOT NULL CHECK (revision BETWEEN 1 AND 193),
    current_job_id TEXT NOT NULL REFERENCES jobs(job_id) DEFERRABLE INITIALLY DEFERRED,
    round INTEGER NOT NULL CHECK (round BETWEEN 1 AND 64),
    stage INTEGER NOT NULL CHECK (stage BETWEEN 1 AND 3),
    created_child BOOLEAN NOT NULL,
    binding_blob BYTEA NOT NULL CHECK (octet_length(binding_blob) BETWEEN 1 AND 4194304),
    binding_sha256 BYTEA NOT NULL CHECK (octet_length(binding_sha256) = 32),
    PRIMARY KEY (run_id, revision)
);
CREATE UNIQUE INDEX run_created_child ON run_transitions(current_job_id)
    WHERE created_child;
CREATE UNIQUE INDEX run_created_stage ON run_transitions(run_id, round, stage)
    WHERE created_child;

CREATE FUNCTION protect_run_transition() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP <> 'INSERT'
       OR current_setting('loop.run_writer', true) IS DISTINCT FROM 'v2'
       OR NOT EXISTS (
           SELECT 1 FROM research_runs r WHERE r.run_id = NEW.run_id
           AND r.workflow_version = 2 AND r.revision = NEW.revision
           AND r.current_job_id = NEW.current_job_id
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
CREATE TRIGGER run_transitions_protected BEFORE INSERT OR UPDATE OR DELETE ON run_transitions
    FOR EACH ROW EXECUTE FUNCTION protect_run_transition();

CREATE FUNCTION require_run_transition() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.workflow_version = 2 AND NOT EXISTS (
        SELECT 1 FROM run_transitions WHERE run_id = NEW.run_id
        AND revision = NEW.revision AND current_job_id = NEW.current_job_id
    ) THEN
        RAISE EXCEPTION 'numerical parent requires immutable transition' USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER research_runs_evidence AFTER INSERT OR UPDATE ON research_runs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION require_run_transition();

CREATE OR REPLACE FUNCTION protect_research_run() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    writer TEXT := current_setting('loop.run_writer', true);
BEGIN
    IF TG_OP = 'DELETE' OR writer IS DISTINCT FROM ('v' || NEW.workflow_version::TEXT) THEN
        RAISE EXCEPTION 'research run requires current writer' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.revision <> 1 OR NEW.status <> 1 OR NEW.completed_rounds <> 0
           OR NEW.first_job_id <> NEW.current_job_id OR NEW.submitted_at_ms <> NEW.updated_at_ms THEN
            RAISE EXCEPTION 'research run must start with first reservation' USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.run_id, NEW.owner_id, NEW.first_job_id, NEW.specification_blob,
        NEW.specification_sha256, NEW.submitted_at_ms, NEW.deadline_ms, NEW.workflow_version)
       IS DISTINCT FROM
       (OLD.run_id, OLD.owner_id, OLD.first_job_id, OLD.specification_blob,
        OLD.specification_sha256, OLD.submitted_at_ms, OLD.deadline_ms, OLD.workflow_version)
       OR OLD.status <> 1 OR NEW.revision <> OLD.revision + 1
       OR NEW.updated_at_ms < OLD.updated_at_ms
       OR NEW.completed_rounds NOT BETWEEN OLD.completed_rounds AND OLD.completed_rounds + 1
       OR NEW.reserved_steps < OLD.reserved_steps OR NEW.reserved_input < OLD.reserved_input
       OR NEW.reserved_output < OLD.reserved_output OR NEW.reserved_nano_usd < OLD.reserved_nano_usd
       OR (NEW.current_job_id <> OLD.current_job_id AND
           (NEW.status <> 1 OR NEW.reserved_steps <= OLD.reserved_steps))
       OR (NEW.current_job_id = OLD.current_job_id AND
           (NEW.status = 1 OR
            (NEW.reserved_steps, NEW.reserved_input, NEW.reserved_output, NEW.reserved_nano_usd)
            IS DISTINCT FROM
            (OLD.reserved_steps, OLD.reserved_input, OLD.reserved_output, OLD.reserved_nano_usd)))
       OR (NEW.workflow_version = 1 AND NEW.current_job_id <> OLD.current_job_id AND
           (NEW.completed_rounds <> OLD.completed_rounds + 1
            OR NEW.reserved_input <= OLD.reserved_input OR NEW.reserved_output <= OLD.reserved_output
            OR NEW.reserved_nano_usd <= OLD.reserved_nano_usd)) THEN
        RAISE EXCEPTION 'invalid research run transition' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

-- A v1 writer cannot add a child to a v2 aggregate, even if it learns its ID.
CREATE OR REPLACE FUNCTION fence_run_child() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
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
