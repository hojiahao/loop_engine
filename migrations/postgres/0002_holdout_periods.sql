CREATE TABLE holdout_periods (
    period_id TEXT PRIMARY KEY,
    canonical_sha256 BYTEA NOT NULL UNIQUE CHECK (octet_length(canonical_sha256) = 32),
    canonical_blob BYTEA NOT NULL CHECK (octet_length(canonical_blob) BETWEEN 1 AND 65536),
    state INTEGER NOT NULL CHECK (state BETWEEN 1 AND 4),
    revision BIGINT NOT NULL CHECK (revision BETWEEN 1 AND 3),
    issued_grant_id TEXT UNIQUE,
    grant_issued_at_ms BIGINT,
    terminal_at_ms BIGINT,
    record_blob BYTEA NOT NULL CHECK (octet_length(record_blob) BETWEEN 1 AND 4194304),
    record_sha256 BYTEA NOT NULL CHECK (octet_length(record_sha256) = 32),
    CHECK (period_id = 'sha256:' || encode(canonical_sha256, 'hex')),
    CHECK (
        (state = 1 AND revision = 1 AND issued_grant_id IS NULL
          AND grant_issued_at_ms IS NULL AND terminal_at_ms IS NULL)
        OR (state = 2 AND revision = 2 AND issued_grant_id IS NOT NULL
          AND length(issued_grant_id) BETWEEN 1 AND 128
          AND grant_issued_at_ms IS NOT NULL AND grant_issued_at_ms >= 0
          AND terminal_at_ms IS NULL)
        OR (state IN (3, 4) AND revision = 3 AND issued_grant_id IS NOT NULL
          AND length(issued_grant_id) BETWEEN 1 AND 128
          AND grant_issued_at_ms IS NOT NULL AND grant_issued_at_ms >= 0
          AND terminal_at_ms IS NOT NULL AND terminal_at_ms >= grant_issued_at_ms)
    )
);

CREATE FUNCTION enforce_period_lifecycle() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'holdout periods cannot be deleted' USING ERRCODE = '23514';
    ELSIF TG_OP = 'INSERT' THEN
        IF NEW.state != 1 THEN
            RAISE EXCEPTION 'new holdout periods must be sealed' USING ERRCODE = '23514';
        END IF;
    ELSIF NEW.period_id IS DISTINCT FROM OLD.period_id
      OR NEW.canonical_sha256 IS DISTINCT FROM OLD.canonical_sha256
      OR NEW.canonical_blob IS DISTINCT FROM OLD.canonical_blob
      OR NEW.revision != OLD.revision + 1
      OR NOT (
        (OLD.state = 1 AND NEW.state = 2)
        OR (OLD.state = 2 AND NEW.state IN (3, 4)
          AND NEW.issued_grant_id IS NOT DISTINCT FROM OLD.issued_grant_id
          AND NEW.grant_issued_at_ms IS NOT DISTINCT FROM OLD.grant_issued_at_ms)
      ) THEN
        RAISE EXCEPTION 'holdout periods are immutable and monotonic' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER holdout_periods_monotonic BEFORE INSERT OR UPDATE OR DELETE ON holdout_periods
    FOR EACH ROW EXECUTE FUNCTION enforce_period_lifecycle();

CREATE TABLE holdout_command_receipts (
    actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 128),
    operation TEXT NOT NULL CHECK (length(operation) BETWEEN 1 AND 128),
    idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 128),
    request_id TEXT NOT NULL CHECK (length(request_id) BETWEEN 1 AND 128),
    period_id TEXT NOT NULL REFERENCES holdout_periods(period_id),
    request_blob BYTEA NOT NULL CHECK (octet_length(request_blob) BETWEEN 1 AND 4194304),
    request_sha256 BYTEA NOT NULL CHECK (octet_length(request_sha256) = 32),
    response_blob BYTEA NOT NULL CHECK (octet_length(response_blob) BETWEEN 1 AND 4194304),
    response_sha256 BYTEA NOT NULL CHECK (octet_length(response_sha256) = 32),
    committed_at_ms BIGINT NOT NULL CHECK (committed_at_ms >= 0),
    PRIMARY KEY (actor_id, operation, idempotency_key)
);
CREATE TRIGGER holdout_receipts_no_update BEFORE UPDATE OR DELETE ON holdout_command_receipts
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();
