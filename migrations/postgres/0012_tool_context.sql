-- Preserve existing invocation evidence while allowing the closed two-call
-- discovery profile. Stop old writers before applying this migration.
ALTER TABLE model_steps ADD COLUMN ordinal INTEGER NOT NULL DEFAULT 0
    CHECK (ordinal BETWEEN 0 AND 1);
ALTER TABLE model_steps DROP CONSTRAINT model_steps_pkey;
ALTER TABLE model_steps ADD PRIMARY KEY (job_id, ordinal);

CREATE OR REPLACE FUNCTION protect_model_step() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('loop.model_step_writer', true) IS DISTINCT FROM 'v2' THEN
        RAISE EXCEPTION 'current model-step writer required' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'model-step history cannot be deleted' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.job_id, NEW.ordinal, NEW.actor_id, NEW.request_id, NEW.idempotency_key,
            NEW.invocation_sha256, NEW.request_blob, NEW.request_sha256,
            NEW.reserved_input, NEW.reserved_output, NEW.reserved_nano_usd,
            NEW.created_revision, NEW.created_at_ms)
            IS DISTINCT FROM
           (OLD.job_id, OLD.ordinal, OLD.actor_id, OLD.request_id, OLD.idempotency_key,
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

CREATE OR REPLACE FUNCTION fence_model_job() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM model_steps WHERE job_id = OLD.job_id)
       AND current_setting('loop.model_step_writer', true) IS DISTINCT FROM 'v2' THEN
        RAISE EXCEPTION 'tracked model job requires current Harness writer' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TABLE tool_results (
    job_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal = 0),
    tool_call_id TEXT NOT NULL CHECK (length(tool_call_id) BETWEEN 1 AND 128),
    result_blob BYTEA NOT NULL CHECK (octet_length(result_blob) BETWEEN 1 AND 262144),
    result_sha256 BYTEA NOT NULL CHECK (octet_length(result_sha256) = 32),
    created_revision BIGINT NOT NULL CHECK (created_revision > 1),
    created_at_ms BIGINT NOT NULL CHECK (created_at_ms >= 0),
    PRIMARY KEY (job_id, ordinal),
    FOREIGN KEY (job_id, ordinal) REFERENCES model_steps(job_id, ordinal)
);

CREATE FUNCTION protect_tool_result() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('loop.model_step_writer', true) IS DISTINCT FROM 'v2'
       OR TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'immutable tool evidence requires current Harness writer' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER tool_results_protected BEFORE INSERT OR UPDATE OR DELETE ON tool_results
    FOR EACH ROW EXECUTE FUNCTION protect_tool_result();
