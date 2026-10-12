-- A single frozen Discovery model step. Old writers must be stopped before
-- deployment; the trigger also fences already-connected pre-Harness binaries.
CREATE TABLE model_steps (
    job_id TEXT PRIMARY KEY REFERENCES jobs(job_id),
    actor_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    invocation_sha256 BYTEA NOT NULL CHECK (octet_length(invocation_sha256) = 32),
    request_blob BYTEA NOT NULL CHECK (octet_length(request_blob) BETWEEN 1 AND 1048576),
    request_sha256 BYTEA NOT NULL CHECK (octet_length(request_sha256) = 32),
    reserved_input BIGINT NOT NULL CHECK (reserved_input > 0),
    reserved_output BIGINT NOT NULL CHECK (reserved_output > 0),
    reserved_nano_usd BIGINT NOT NULL CHECK (reserved_nano_usd > 0),
    state TEXT NOT NULL CHECK (state IN ('reserved', 'dispatched', 'ambiguous', 'completed')),
    created_revision BIGINT NOT NULL CHECK (created_revision > 1),
    updated_revision BIGINT NOT NULL CHECK (updated_revision >= created_revision),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms BIGINT NOT NULL CHECK (updated_at_ms >= created_at_ms),
    response_blob BYTEA,
    response_sha256 BYTEA,
    UNIQUE(actor_id, idempotency_key),
    CHECK ((state = 'completed' AND response_blob IS NOT NULL
            AND octet_length(response_blob) BETWEEN 1 AND 1048576
            AND response_sha256 IS NOT NULL AND octet_length(response_sha256) = 32)
        OR (state <> 'completed' AND response_blob IS NULL AND response_sha256 IS NULL))
);

CREATE FUNCTION protect_model_step() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('loop.model_step_writer', true) IS DISTINCT FROM 'v1' THEN
        RAISE EXCEPTION 'model-step writer required' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'model-step history cannot be deleted' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.job_id, NEW.actor_id, NEW.request_id, NEW.idempotency_key,
            NEW.invocation_sha256, NEW.request_blob, NEW.request_sha256,
            NEW.reserved_input, NEW.reserved_output, NEW.reserved_nano_usd,
            NEW.created_revision, NEW.created_at_ms)
            IS DISTINCT FROM
           (OLD.job_id, OLD.actor_id, OLD.request_id, OLD.idempotency_key,
            OLD.invocation_sha256, OLD.request_blob, OLD.request_sha256,
            OLD.reserved_input, OLD.reserved_output, OLD.reserved_nano_usd,
            OLD.created_revision, OLD.created_at_ms)
           OR NOT ((OLD.state = 'reserved' AND NEW.state = 'dispatched')
                OR (OLD.state = 'dispatched' AND NEW.state IN ('ambiguous', 'completed'))
                OR (OLD.state = 'ambiguous' AND NEW.state = 'completed'))
           OR NEW.updated_revision <= OLD.updated_revision
           OR NEW.updated_at_ms < OLD.updated_at_ms THEN
            RAISE EXCEPTION 'invalid model-step transition' USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER model_steps_protected BEFORE INSERT OR UPDATE OR DELETE ON model_steps
    FOR EACH ROW EXECUTE FUNCTION protect_model_step();

CREATE FUNCTION fence_model_job() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM model_steps WHERE job_id = OLD.job_id)
       AND current_setting('loop.model_step_writer', true) IS DISTINCT FROM 'v1' THEN
        RAISE EXCEPTION 'tracked model job requires current Harness writer' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER model_jobs_fenced BEFORE UPDATE OR DELETE ON jobs
    FOR EACH ROW EXECUTE FUNCTION fence_model_job();
