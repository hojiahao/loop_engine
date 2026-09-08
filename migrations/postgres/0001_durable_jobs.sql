CREATE TABLE store_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    audit_ledger_id TEXT NOT NULL CHECK (length(audit_ledger_id) BETWEEN 1 AND 128),
    last_observed_at_ms BIGINT NOT NULL CHECK (last_observed_at_ms >= 0)
);

CREATE TABLE jobs (
    job_id TEXT PRIMARY KEY CHECK (length(job_id) BETWEEN 1 AND 128),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 128),
    kind INTEGER NOT NULL CHECK (kind BETWEEN 1 AND 7),
    state INTEGER NOT NULL CHECK (state BETWEEN 1 AND 8),
    revision BIGINT NOT NULL CHECK (revision > 0),
    attempt BIGINT NOT NULL CHECK (attempt BETWEEN 0 AND 4294967295),
    submitted_at_ms BIGINT NOT NULL CHECK (submitted_at_ms >= 0),
    updated_at_ms BIGINT NOT NULL CHECK (updated_at_ms >= submitted_at_ms),
    deadline_ms BIGINT NOT NULL CHECK (deadline_ms > submitted_at_ms),
    lease_id TEXT,
    lease_owner_id TEXT,
    lease_expires_at_ms BIGINT,
    record_blob BYTEA NOT NULL CHECK (octet_length(record_blob) BETWEEN 1 AND 4194304),
    record_sha256 BYTEA NOT NULL CHECK (octet_length(record_sha256) = 32),
    CHECK (
        (state IN (2, 3) AND lease_id IS NOT NULL AND lease_owner_id IS NOT NULL
          AND lease_expires_at_ms IS NOT NULL
          AND lease_expires_at_ms > updated_at_ms AND lease_expires_at_ms <= deadline_ms)
        OR (state NOT IN (2, 3) AND lease_id IS NULL AND lease_owner_id IS NULL
          AND lease_expires_at_ms IS NULL)
    ),
    CHECK ((state = 1 AND attempt = 0) OR state IN (7, 8)
      OR (state IN (2, 3, 4, 5, 6) AND attempt > 0))
);

CREATE INDEX jobs_run ON jobs (run_id, job_id);
CREATE INDEX jobs_queue ON jobs (state, submitted_at_ms, job_id);
CREATE INDEX jobs_deadline ON jobs (deadline_ms, state);
CREATE INDEX jobs_expiring_lease ON jobs (lease_expires_at_ms) WHERE state IN (2, 3);

CREATE TABLE command_receipts (
    actor_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_id TEXT NOT NULL,
    job_id TEXT NOT NULL REFERENCES jobs(job_id),
    request_blob BYTEA NOT NULL CHECK (octet_length(request_blob) BETWEEN 1 AND 4194304),
    request_sha256 BYTEA NOT NULL CHECK (octet_length(request_sha256) = 32),
    response_blob BYTEA NOT NULL CHECK (octet_length(response_blob) BETWEEN 1 AND 4194304),
    response_sha256 BYTEA NOT NULL CHECK (octet_length(response_sha256) = 32),
    committed_at_ms BIGINT NOT NULL CHECK (committed_at_ms >= 0),
    PRIMARY KEY (actor_id, operation, idempotency_key)
);

CREATE TABLE audit_events (
    sequence BIGINT PRIMARY KEY CHECK (sequence > 0),
    event_id TEXT NOT NULL UNIQUE,
    previous_sha256 BYTEA NOT NULL CHECK (octet_length(previous_sha256) = 32),
    occurred_at_ms BIGINT NOT NULL CHECK (occurred_at_ms >= 0),
    correlation_id TEXT NOT NULL,
    causation_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('human', 'service', 'agent', 'scheduler')),
    actor_display_name TEXT NOT NULL,
    actor_subject TEXT NOT NULL,
    action TEXT NOT NULL,
    job_id TEXT REFERENCES jobs(job_id),
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    payload_schema TEXT NOT NULL,
    payload_blob BYTEA NOT NULL CHECK (octet_length(payload_blob) BETWEEN 1 AND 262144),
    payload_sha256 BYTEA NOT NULL CHECK (octet_length(payload_sha256) = 32),
    event_sha256 BYTEA NOT NULL CHECK (octet_length(event_sha256) = 32)
);

CREATE FUNCTION reject_immutable_change() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    RAISE EXCEPTION 'immutable evidence cannot be changed' USING ERRCODE = '23514';
END;
$$;
CREATE TRIGGER audit_events_no_update BEFORE UPDATE OR DELETE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();
CREATE TRIGGER command_receipts_no_update BEFORE UPDATE OR DELETE ON command_receipts
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();
