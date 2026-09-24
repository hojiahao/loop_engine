CREATE TABLE store_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    audit_ledger_id TEXT NOT NULL CHECK (length(audit_ledger_id) BETWEEN 1 AND 128),
    last_observed_at_ms INTEGER NOT NULL CHECK (last_observed_at_ms >= 0)
) STRICT;

CREATE TABLE jobs (
    job_id TEXT PRIMARY KEY NOT NULL CHECK (length(job_id) BETWEEN 1 AND 128),
    run_id TEXT NOT NULL CHECK (length(run_id) BETWEEN 1 AND 128),
    kind INTEGER NOT NULL CHECK (kind BETWEEN 1 AND 7),
    state INTEGER NOT NULL CHECK (state BETWEEN 1 AND 8),
    revision INTEGER NOT NULL CHECK (revision > 0),
    attempt INTEGER NOT NULL CHECK (attempt BETWEEN 0 AND 4294967295),
    submitted_at_ms INTEGER NOT NULL CHECK (submitted_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= submitted_at_ms),
    deadline_ms INTEGER NOT NULL CHECK (deadline_ms > submitted_at_ms),
    lease_id TEXT,
    lease_owner_id TEXT,
    lease_expires_at_ms INTEGER,
    record_blob BLOB NOT NULL CHECK (length(record_blob) BETWEEN 1 AND 4194304),
    record_sha256 BLOB NOT NULL CHECK (length(record_sha256) = 32),
    CHECK (
        (state IN (2, 3) AND lease_id IS NOT NULL AND lease_owner_id IS NOT NULL
          AND lease_expires_at_ms IS NOT NULL
          AND lease_expires_at_ms > updated_at_ms AND lease_expires_at_ms <= deadline_ms)
        OR (state NOT IN (2, 3) AND lease_id IS NULL AND lease_owner_id IS NULL
          AND lease_expires_at_ms IS NULL)
    ),
    CHECK ((state = 1 AND attempt = 0) OR state IN (7, 8)
      OR (state IN (2, 3, 4, 5, 6) AND attempt > 0))
) STRICT;

CREATE INDEX jobs_run ON jobs (run_id, job_id);
CREATE INDEX jobs_queue ON jobs (state, submitted_at_ms, job_id);
CREATE INDEX jobs_deadline ON jobs (deadline_ms, state);
CREATE INDEX jobs_expiring_lease ON jobs (lease_expires_at_ms)
    WHERE state IN (2, 3);

CREATE TABLE command_receipts (
    actor_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_id TEXT NOT NULL,
    job_id TEXT NOT NULL REFERENCES jobs(job_id),
    request_blob BLOB NOT NULL CHECK (length(request_blob) BETWEEN 1 AND 4194304),
    request_sha256 BLOB NOT NULL CHECK (length(request_sha256) = 32),
    response_blob BLOB NOT NULL CHECK (length(response_blob) BETWEEN 1 AND 4194304),
    response_sha256 BLOB NOT NULL CHECK (length(response_sha256) = 32),
    committed_at_ms INTEGER NOT NULL CHECK (committed_at_ms >= 0),
    PRIMARY KEY (actor_id, operation, idempotency_key)
) STRICT;

CREATE TABLE audit_events (
    sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
    event_id TEXT NOT NULL UNIQUE,
    previous_sha256 BLOB NOT NULL CHECK (length(previous_sha256) = 32),
    occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms >= 0),
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
    payload_blob BLOB NOT NULL CHECK (length(payload_blob) BETWEEN 1 AND 262144),
    payload_sha256 BLOB NOT NULL CHECK (length(payload_sha256) = 32),
    event_sha256 BLOB NOT NULL CHECK (length(event_sha256) = 32)
) STRICT;

CREATE TRIGGER audit_events_no_update BEFORE UPDATE ON audit_events
BEGIN SELECT RAISE(ABORT, 'audit events are immutable'); END;
CREATE TRIGGER audit_events_no_delete BEFORE DELETE ON audit_events
BEGIN SELECT RAISE(ABORT, 'audit events are immutable'); END;
CREATE TRIGGER command_receipts_no_update BEFORE UPDATE ON command_receipts
BEGIN SELECT RAISE(ABORT, 'command receipts are immutable'); END;
CREATE TRIGGER command_receipts_no_delete BEFORE DELETE ON command_receipts
BEGIN SELECT RAISE(ABORT, 'command receipts are immutable'); END;
