-- Additive managed-run reservations. Stop older writers before deployment;
-- preserve this schema and immutable receipts on application rollback.
CREATE TABLE research_runs (
    run_id TEXT PRIMARY KEY CHECK (length(run_id) BETWEEN 1 AND 128),
    owner_id TEXT NOT NULL CHECK (length(owner_id) BETWEEN 1 AND 128),
    first_job_id TEXT NOT NULL REFERENCES jobs(job_id) DEFERRABLE INITIALLY DEFERRED,
    current_job_id TEXT NOT NULL REFERENCES jobs(job_id) DEFERRABLE INITIALLY DEFERRED,
    status INTEGER NOT NULL CHECK (status BETWEEN 1 AND 5),
    revision BIGINT NOT NULL CHECK (revision > 0),
    completed_rounds INTEGER NOT NULL CHECK (completed_rounds BETWEEN 0 AND 64),
    reserved_steps BIGINT NOT NULL CHECK (reserved_steps > 0),
    reserved_input BIGINT NOT NULL CHECK (reserved_input > 0),
    reserved_output BIGINT NOT NULL CHECK (reserved_output > 0),
    reserved_nano_usd BIGINT NOT NULL CHECK (reserved_nano_usd > 0),
    submitted_at_ms BIGINT NOT NULL CHECK (submitted_at_ms >= 0),
    updated_at_ms BIGINT NOT NULL CHECK (updated_at_ms >= submitted_at_ms),
    deadline_ms BIGINT NOT NULL CHECK (deadline_ms > submitted_at_ms),
    specification_blob BYTEA NOT NULL CHECK (octet_length(specification_blob) BETWEEN 1 AND 4194304),
    specification_sha256 BYTEA NOT NULL CHECK (octet_length(specification_sha256) = 32),
    view_blob BYTEA NOT NULL CHECK (octet_length(view_blob) BETWEEN 1 AND 4194304),
    view_sha256 BYTEA NOT NULL CHECK (octet_length(view_sha256) = 32)
);

CREATE FUNCTION protect_research_run() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('loop.run_writer', true) IS DISTINCT FROM 'v1' OR TG_OP = 'DELETE' THEN
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
        NEW.specification_sha256, NEW.submitted_at_ms, NEW.deadline_ms)
       IS DISTINCT FROM
       (OLD.run_id, OLD.owner_id, OLD.first_job_id, OLD.specification_blob,
        OLD.specification_sha256, OLD.submitted_at_ms, OLD.deadline_ms)
       OR OLD.status <> 1 OR NEW.revision <> OLD.revision + 1
       OR NEW.updated_at_ms < OLD.updated_at_ms
       OR NEW.completed_rounds NOT BETWEEN OLD.completed_rounds AND OLD.completed_rounds + 1
       OR NEW.reserved_steps < OLD.reserved_steps OR NEW.reserved_input < OLD.reserved_input
       OR NEW.reserved_output < OLD.reserved_output OR NEW.reserved_nano_usd < OLD.reserved_nano_usd
       OR (NEW.current_job_id <> OLD.current_job_id AND
           (NEW.status <> 1 OR NEW.completed_rounds <> OLD.completed_rounds + 1
            OR NEW.reserved_steps <= OLD.reserved_steps OR NEW.reserved_input <= OLD.reserved_input
            OR NEW.reserved_output <= OLD.reserved_output OR NEW.reserved_nano_usd <= OLD.reserved_nano_usd))
       OR (NEW.current_job_id = OLD.current_job_id AND
           (NEW.reserved_steps, NEW.reserved_input, NEW.reserved_output, NEW.reserved_nano_usd)
           IS DISTINCT FROM
           (OLD.reserved_steps, OLD.reserved_input, OLD.reserved_output, OLD.reserved_nano_usd)) THEN
        RAISE EXCEPTION 'invalid research run transition' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER research_runs_protected BEFORE INSERT OR UPDATE OR DELETE ON research_runs
    FOR EACH ROW EXECUTE FUNCTION protect_research_run();

CREATE FUNCTION fence_run_child() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM research_runs WHERE run_id = NEW.run_id) AND
       (current_setting('loop.run_child', true) IS DISTINCT FROM NEW.job_id
        OR NOT EXISTS (SELECT 1 FROM research_runs WHERE run_id = NEW.run_id
            AND current_job_id = NEW.job_id AND status = 1)) THEN
        RAISE EXCEPTION 'managed run requires atomic child reservation' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER jobs_run_fenced BEFORE INSERT ON jobs FOR EACH ROW EXECUTE FUNCTION fence_run_child();
