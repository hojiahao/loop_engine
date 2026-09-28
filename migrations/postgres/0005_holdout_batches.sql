CREATE TABLE holdout_batches (
    batch_id TEXT PRIMARY KEY CHECK (length(batch_id) BETWEEN 1 AND 128),
    grant_id TEXT NOT NULL UNIQUE REFERENCES holdout_grants(grant_id),
    period_id TEXT NOT NULL REFERENCES holdout_periods(period_id),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 128),
    actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 128),
    plan_id TEXT NOT NULL CHECK (plan_id ~ '^sha256:[0-9a-f]{64}$'),
    plan_sha256 BYTEA NOT NULL CHECK (octet_length(plan_sha256) = 32),
    job_count INTEGER NOT NULL CHECK (job_count BETWEEN 1 AND 4096),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms BETWEEN 0 AND 253402300799999),
    handle_blob BYTEA NOT NULL CHECK (octet_length(handle_blob) BETWEEN 1 AND 4194304),
    handle_sha256 BYTEA NOT NULL CHECK (octet_length(handle_sha256) = 32),
    FOREIGN KEY (grant_id, period_id) REFERENCES holdout_grants(grant_id, period_id)
);
CREATE TRIGGER batches_no_update BEFORE UPDATE OR DELETE ON holdout_batches
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();

CREATE TABLE holdout_batch_jobs (
    batch_id TEXT NOT NULL REFERENCES holdout_batches(batch_id),
    entry_index INTEGER NOT NULL CHECK (entry_index BETWEEN 1 AND 4096),
    job_id TEXT NOT NULL UNIQUE REFERENCES jobs(job_id),
    specification_sha256 BYTEA NOT NULL CHECK (octet_length(specification_sha256) = 32),
    PRIMARY KEY (batch_id, entry_index)
);
CREATE TRIGGER batch_jobs_no_update BEFORE UPDATE OR DELETE ON holdout_batch_jobs
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();

CREATE FUNCTION verify_holdout_batch() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE valid BOOLEAN;
BEGIN
    EXECUTE format(
        'SELECT (g.state != 2 AND b.batch_id IS NULL) OR
          (g.state = 2 AND b.grant_id = g.grant_id AND b.period_id = g.period_id
           AND b.created_at_ms = g.terminal_at_ms
           AND b.job_count = (SELECT count(*) FROM
             (SELECT 1 FROM %I.holdout_batch_jobs l WHERE l.batch_id = b.batch_id LIMIT 4097) bounded))
         FROM %I.holdout_grants g LEFT JOIN %I.holdout_batches b ON b.grant_id = g.grant_id
         WHERE g.grant_id = $1', TG_TABLE_SCHEMA, TG_TABLE_SCHEMA, TG_TABLE_SCHEMA)
        INTO valid USING NEW.grant_id;
    IF valid IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'consumed grant requires one complete batch' USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER consumed_grant_batch AFTER INSERT OR UPDATE ON holdout_grants
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_holdout_batch();
CREATE CONSTRAINT TRIGGER batch_complete AFTER INSERT ON holdout_batches
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_holdout_batch();

CREATE FUNCTION verify_holdout_job_link() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE valid BOOLEAN;
BEGIN
    EXECUTE format(
        'SELECT (j.kind != 7 AND l.job_id IS NULL) OR
          (j.kind = 7 AND l.job_id = j.job_id AND l.entry_index <= b.job_count
           AND j.run_id = b.run_id AND j.submitted_at_ms = b.created_at_ms)
         FROM %I.jobs j LEFT JOIN %I.holdout_batch_jobs l ON l.job_id = j.job_id
         LEFT JOIN %I.holdout_batches b ON b.batch_id = l.batch_id WHERE j.job_id = $1',
         TG_TABLE_SCHEMA, TG_TABLE_SCHEMA, TG_TABLE_SCHEMA) INTO valid USING NEW.job_id;
    IF valid IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'holdout jobs require exact batch membership' USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER batch_job_binding AFTER INSERT ON holdout_batch_jobs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_holdout_job_link();
CREATE CONSTRAINT TRIGGER holdout_job_membership AFTER INSERT OR UPDATE ON jobs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_holdout_job_link();
