-- Opaque historical job blobs need a separately verified trial import.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM jobs WHERE kind IN (2, 3)) THEN
        RAISE EXCEPTION 'existing research jobs require verified trial migration'
            USING ERRCODE = '23514';
    END IF;
END;
$$;

CREATE TABLE factor_trials (
    job_id TEXT PRIMARY KEY REFERENCES jobs(job_id),
    run_id TEXT NOT NULL,
    factor_spec_id TEXT NOT NULL CHECK (factor_spec_id ~ '^sha256:[0-9a-f]{64}$'),
    specification_sha256 BYTEA NOT NULL CHECK (octet_length(specification_sha256) = 32)
);
CREATE INDEX factor_trials_run ON factor_trials(run_id, job_id);
CREATE TRIGGER factor_trials_immutable BEFORE UPDATE OR DELETE ON factor_trials
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();

CREATE FUNCTION verify_factor_trial() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE valid BOOLEAN;
BEGIN
    EXECUTE format(
        'SELECT (j.kind IN (2, 3)) = (t.job_id IS NOT NULL)
          AND (t.job_id IS NULL OR t.run_id = j.run_id)
         FROM %I.jobs j LEFT JOIN %I.factor_trials t ON t.job_id = j.job_id
         WHERE j.job_id = $1', TG_TABLE_SCHEMA, TG_TABLE_SCHEMA)
         INTO valid USING NEW.job_id;
    IF valid IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'research job requires one atomic trial'
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER research_job_trial AFTER INSERT OR UPDATE ON jobs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_factor_trial();
CREATE CONSTRAINT TRIGGER trial_job_binding AFTER INSERT ON factor_trials
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_factor_trial();

CREATE TABLE factor_states (
    context_id TEXT NOT NULL CHECK (length(context_id) BETWEEN 1 AND 128),
    factor_spec_id TEXT NOT NULL CHECK (factor_spec_id ~ '^sha256:[0-9a-f]{64}$'),
    revision BIGINT NOT NULL CHECK (revision > 0),
    status TEXT NOT NULL CHECK (status IN ('admitted', 'rejected', 'retired')),
    admissions BIGINT NOT NULL CHECK (admissions >= 0),
    retirements BIGINT NOT NULL CHECK (retirements >= 0),
    source_job_id TEXT NOT NULL REFERENCES factor_trials(job_id),
    updated_at_ms BIGINT NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 253402300799999),
    actor_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK (operation = 'loop.factors.decide'),
    idempotency_key TEXT NOT NULL,
    PRIMARY KEY(context_id, factor_spec_id),
    CHECK ((status = 'admitted' AND admissions = retirements + 1)
        OR (status <> 'admitted' AND admissions = retirements)),
    FOREIGN KEY (actor_id, operation, idempotency_key)
        REFERENCES command_receipts(actor_id, operation, idempotency_key)
        DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX factor_states_active ON factor_states(context_id, factor_spec_id)
    WHERE status = 'admitted';

CREATE FUNCTION fence_factor_state() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'factor history cannot be deleted' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.revision <> 1 OR NEW.retirements <> 0 OR NEW.status = 'retired' THEN
            RAISE EXCEPTION 'factor state must start at revision one' USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;
    IF NEW.context_id <> OLD.context_id OR NEW.factor_spec_id <> OLD.factor_spec_id
       OR NEW.revision <> OLD.revision + 1 OR NEW.updated_at_ms < OLD.updated_at_ms
       OR NEW.admissions <> OLD.admissions + (CASE WHEN NEW.status = 'admitted' THEN 1 ELSE 0 END)
       OR NEW.retirements <> OLD.retirements + (CASE WHEN NEW.status = 'retired' THEN 1 ELSE 0 END)
       OR (OLD.status = 'admitted' AND NEW.status <> 'retired')
       OR (OLD.status <> 'admitted' AND NEW.status = 'retired') THEN
        RAISE EXCEPTION 'factor transition is not revision fenced' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER factor_states_fenced BEFORE INSERT OR UPDATE OR DELETE ON factor_states
    FOR EACH ROW EXECUTE FUNCTION fence_factor_state();

CREATE FUNCTION verify_factor_source() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE valid BOOLEAN;
BEGIN
    EXECUTE format('SELECT factor_spec_id = $1 FROM %I.factor_trials WHERE job_id = $2', TG_TABLE_SCHEMA)
        INTO valid USING NEW.factor_spec_id, NEW.source_job_id;
    IF valid IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'factor state source identity mismatch' USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER factor_state_source AFTER INSERT OR UPDATE ON factor_states
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_factor_source();
