-- Stop old writers before applying this additive lifecycle migration. Preserve
-- immutable requests, responses, reservations, receipts and audit history.
ALTER TABLE jobs DROP CONSTRAINT jobs_state_check;
ALTER TABLE jobs ADD CONSTRAINT jobs_state_check
    CHECK (state BETWEEN 1 AND 8 OR (state = 9 AND kind = 1));
-- 0001's cross-column clock checks are jobs_check/jobs_check1; the
-- state/attempt invariant is jobs_check3. Preserve both clock constraints.
ALTER TABLE jobs DROP CONSTRAINT jobs_check3;
ALTER TABLE jobs ADD CONSTRAINT jobs_check3
    CHECK ((state = 1 AND attempt = 0) OR state IN (7, 8)
      OR (state = 9 AND kind = 1)
      OR (state IN (2, 3, 4, 5, 6) AND attempt > 0));

ALTER TABLE model_steps
    ADD COLUMN lookup_attempts INTEGER NOT NULL DEFAULT 0 CHECK (lookup_attempts BETWEEN 0 AND 3),
    ADD COLUMN tool_attempts INTEGER NOT NULL DEFAULT 0 CHECK (tool_attempts BETWEEN 0 AND 3),
    ADD COLUMN retry_after_ms BIGINT NOT NULL DEFAULT 0 CHECK (retry_after_ms >= 0),
    ADD COLUMN response_revision BIGINT,
    ADD COLUMN response_at_ms BIGINT;
-- Completion chronology must survive later safe-attempt metadata updates.
ALTER TABLE model_steps DISABLE TRIGGER model_steps_protected;
UPDATE model_steps SET response_revision = updated_revision,
    response_at_ms = updated_at_ms WHERE state = 'completed';
ALTER TABLE model_steps ENABLE TRIGGER model_steps_protected;
ALTER TABLE model_steps ADD CONSTRAINT model_response_clock CHECK (
    (state = 'completed' AND response_revision IS NOT NULL AND response_at_ms IS NOT NULL
      AND response_revision > created_revision AND response_revision <= updated_revision
      AND response_at_ms >= created_at_ms AND response_at_ms <= updated_at_ms)
    OR (state <> 'completed' AND response_revision IS NULL AND response_at_ms IS NULL));
ALTER TABLE model_steps ADD CONSTRAINT model_retry_clock CHECK (
    (lookup_attempts = 0 AND tool_attempts = 0 AND retry_after_ms = 0)
    OR ((lookup_attempts > 0 OR tool_attempts > 0) AND retry_after_ms >= created_at_ms));

CREATE OR REPLACE FUNCTION protect_model_step() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('loop.model_step_writer', true) IS DISTINCT FROM 'v3' THEN
        RAISE EXCEPTION 'current model-step writer required' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'model-step history cannot be deleted' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF NEW.state <> 'reserved' OR NEW.lookup_attempts <> 0
           OR NEW.tool_attempts <> 0 OR NEW.retry_after_ms <> 0 THEN
            RAISE EXCEPTION 'model step must start reserved' USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.job_id, NEW.ordinal, NEW.actor_id, NEW.request_id, NEW.idempotency_key,
        NEW.invocation_sha256, NEW.request_blob, NEW.request_sha256,
        NEW.reserved_input, NEW.reserved_output, NEW.reserved_nano_usd,
        NEW.created_revision, NEW.created_at_ms)
        IS DISTINCT FROM
       (OLD.job_id, OLD.ordinal, OLD.actor_id, OLD.request_id, OLD.idempotency_key,
        OLD.invocation_sha256, OLD.request_blob, OLD.request_sha256,
        OLD.reserved_input, OLD.reserved_output, OLD.reserved_nano_usd,
        OLD.created_revision, OLD.created_at_ms)
       OR NEW.updated_revision <= OLD.updated_revision
       OR NEW.updated_at_ms < OLD.updated_at_ms THEN
        RAISE EXCEPTION 'immutable model identity or clock changed' USING ERRCODE = '23514';
    END IF;
    IF NEW.state = OLD.state THEN
        IF (NEW.response_blob, NEW.response_sha256, NEW.response_revision, NEW.response_at_ms)
            IS DISTINCT FROM
           (OLD.response_blob, OLD.response_sha256, OLD.response_revision, OLD.response_at_ms)
           OR NEW.retry_after_ms <> NEW.updated_at_ms + 250
           OR NEW.updated_at_ms < OLD.retry_after_ms
           OR NOT (
               (OLD.state IN ('dispatched', 'ambiguous')
                AND NEW.lookup_attempts = OLD.lookup_attempts + 1
                AND NEW.tool_attempts = OLD.tool_attempts)
               OR (OLD.state = 'completed' AND OLD.ordinal = 0
                AND NEW.tool_attempts = OLD.tool_attempts + 1
                AND NEW.lookup_attempts = OLD.lookup_attempts
                AND NOT EXISTS (SELECT 1 FROM tool_results
                    WHERE job_id = OLD.job_id AND ordinal = OLD.ordinal))) THEN
            RAISE EXCEPTION 'invalid safe model retry' USING ERRCODE = '23514';
        END IF;
    ELSE
        IF (NEW.lookup_attempts, NEW.tool_attempts, NEW.retry_after_ms)
            IS DISTINCT FROM (OLD.lookup_attempts, OLD.tool_attempts, OLD.retry_after_ms)
           OR NOT ((OLD.state = 'reserved' AND NEW.state = 'dispatched')
                OR (OLD.state = 'dispatched' AND NEW.state IN ('ambiguous', 'completed'))
                OR (OLD.state = 'ambiguous' AND NEW.state = 'completed')) THEN
            RAISE EXCEPTION 'invalid model-step transition' USING ERRCODE = '23514';
        END IF;
        IF NEW.state = 'completed' AND
           (NEW.response_revision IS DISTINCT FROM NEW.updated_revision
            OR NEW.response_at_ms IS DISTINCT FROM NEW.updated_at_ms) THEN
            RAISE EXCEPTION 'model completion clock mismatch' USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION fence_model_job() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF (OLD.kind = 1 OR EXISTS (SELECT 1 FROM model_steps WHERE job_id = OLD.job_id))
       AND current_setting('loop.model_step_writer', true) IS DISTINCT FROM 'v3' THEN
        RAISE EXCEPTION 'discovery job requires current Harness writer' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION protect_tool_result() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('loop.model_step_writer', true) IS DISTINCT FROM 'v3'
       OR TG_OP <> 'INSERT' THEN
        RAISE EXCEPTION 'immutable tool evidence requires current Harness writer' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
