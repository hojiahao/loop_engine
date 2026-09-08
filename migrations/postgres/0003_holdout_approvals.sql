-- Immutable human attestations. Grant attachment will be a separate relation;
-- neither attaching nor consuming a grant may rewrite the signed record.
CREATE TABLE holdout_approvals (
    approval_id TEXT PRIMARY KEY CHECK (length(approval_id) BETWEEN 1 AND 128),
    period_id TEXT NOT NULL REFERENCES holdout_periods(period_id),
    actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 128),
    authenticated_subject TEXT NOT NULL CHECK (octet_length(authenticated_subject) BETWEEN 1 AND 4096),
    freeze_sha256 BYTEA NOT NULL CHECK (octet_length(freeze_sha256) = 32),
    plan_id TEXT NOT NULL CHECK (plan_id ~ '^sha256:[0-9a-f]{64}$'),
    plan_sha256 BYTEA NOT NULL CHECK (octet_length(plan_sha256) = 32),
    plan_entry_count INTEGER NOT NULL CHECK (plan_entry_count BETWEEN 1 AND 4096),
    approved_at_ms BIGINT NOT NULL CHECK (approved_at_ms BETWEEN 0 AND 253402300799999),
    expires_at_ms BIGINT NOT NULL CHECK (
        expires_at_ms > approved_at_ms AND expires_at_ms <= 253402300799999 AND
        expires_at_ms::NUMERIC - approved_at_ms::NUMERIC <= 604800000
    ),
    canonical_blob BYTEA NOT NULL CHECK (octet_length(canonical_blob) BETWEEN 1 AND 262144),
    canonical_sha256 BYTEA NOT NULL UNIQUE CHECK (octet_length(canonical_sha256) = 32),
    record_blob BYTEA NOT NULL CHECK (octet_length(record_blob) BETWEEN 1 AND 4194304),
    record_sha256 BYTEA NOT NULL CHECK (octet_length(record_sha256) = 32)
);

CREATE INDEX holdout_approvals_period_actor ON holdout_approvals (period_id, actor_id);

CREATE TRIGGER holdout_approvals_no_update
BEFORE UPDATE OR DELETE ON holdout_approvals
FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();
