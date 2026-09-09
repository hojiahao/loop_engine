CREATE TABLE holdout_grants (
    grant_id TEXT PRIMARY KEY CHECK (length(grant_id) BETWEEN 1 AND 128),
    period_id TEXT NOT NULL UNIQUE REFERENCES holdout_periods(period_id),
    freeze_sha256 BYTEA NOT NULL CHECK (octet_length(freeze_sha256) = 32),
    freeze_blob BYTEA NOT NULL CHECK (octet_length(freeze_blob) BETWEEN 1 AND 4194304),
    freeze_blob_sha256 BYTEA NOT NULL CHECK (octet_length(freeze_blob_sha256) = 32),
    approval_count INTEGER NOT NULL CHECK (approval_count BETWEEN 1 AND 8),
    state INTEGER NOT NULL CHECK (state BETWEEN 1 AND 4),
    revision BIGINT NOT NULL CHECK (revision BETWEEN 1 AND 2),
    issued_at_ms BIGINT NOT NULL CHECK (issued_at_ms BETWEEN 0 AND 253402300799999),
    expires_at_ms BIGINT NOT NULL CHECK (expires_at_ms > issued_at_ms
        AND expires_at_ms <= 253402300799999
        AND expires_at_ms::NUMERIC - issued_at_ms::NUMERIC <= 604800000),
    terminal_at_ms BIGINT,
    record_blob BYTEA NOT NULL CHECK (octet_length(record_blob) BETWEEN 1 AND 4194304),
    record_sha256 BYTEA NOT NULL CHECK (octet_length(record_sha256) = 32),
    UNIQUE (grant_id, period_id),
    CHECK ((state = 1 AND revision = 1 AND terminal_at_ms IS NULL)
        OR (state IN (2, 4) AND revision = 2 AND terminal_at_ms IS NOT NULL AND terminal_at_ms >= issued_at_ms
            AND terminal_at_ms < expires_at_ms)
        OR (state = 3 AND revision = 2 AND terminal_at_ms IS NOT NULL AND terminal_at_ms >= expires_at_ms
            AND terminal_at_ms <= 253402300799999))
);

ALTER TABLE holdout_periods ADD CONSTRAINT period_grant_identity
    FOREIGN KEY (issued_grant_id, period_id) REFERENCES holdout_grants(grant_id, period_id)
    DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE holdout_approvals ADD CONSTRAINT approval_subject_identity
    UNIQUE (approval_id, period_id, actor_id, authenticated_subject);

CREATE TABLE holdout_grant_approvals (
    grant_id TEXT NOT NULL,
    period_id TEXT NOT NULL,
    approval_id TEXT NOT NULL UNIQUE,
    actor_id TEXT NOT NULL,
    authenticated_subject TEXT NOT NULL,
    PRIMARY KEY (grant_id, approval_id),
    UNIQUE (grant_id, actor_id),
    UNIQUE (grant_id, authenticated_subject),
    FOREIGN KEY (grant_id, period_id) REFERENCES holdout_grants(grant_id, period_id),
    FOREIGN KEY (approval_id, period_id, actor_id, authenticated_subject)
        REFERENCES holdout_approvals(approval_id, period_id, actor_id, authenticated_subject)
);
CREATE TRIGGER grant_approvals_no_update BEFORE UPDATE OR DELETE ON holdout_grant_approvals
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();

CREATE FUNCTION enforce_grant_lifecycle() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'grants cannot be deleted' USING ERRCODE = '23514';
    ELSIF TG_OP = 'INSERT' THEN
        IF NEW.state != 1 THEN
            RAISE EXCEPTION 'new grants must be issued' USING ERRCODE = '23514';
        END IF;
    ELSIF OLD.state != 1 OR NEW.state NOT IN (2, 3, 4)
        OR NEW.revision != OLD.revision + 1
        OR (NEW.grant_id, NEW.period_id, NEW.freeze_sha256, NEW.freeze_blob,
            NEW.freeze_blob_sha256, NEW.approval_count, NEW.issued_at_ms, NEW.expires_at_ms)
           IS DISTINCT FROM
           (OLD.grant_id, OLD.period_id, OLD.freeze_sha256, OLD.freeze_blob,
            OLD.freeze_blob_sha256, OLD.approval_count, OLD.issued_at_ms, OLD.expires_at_ms) THEN
        RAISE EXCEPTION 'grant lifecycle is immutable and single-use' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER grants_monotonic BEFORE INSERT OR UPDATE OR DELETE ON holdout_grants
    FOR EACH ROW EXECUTE FUNCTION enforce_grant_lifecycle();

-- Deferred checks observe the complete transaction, never a partially built grant.
CREATE FUNCTION verify_grant_aggregate() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE valid BOOLEAN;
BEGIN
    EXECUTE format(
        'SELECT (p.state = 1 AND g.grant_id IS NULL) OR
           (p.issued_grant_id = g.grant_id AND p.grant_issued_at_ms = g.issued_at_ms
            AND ((p.state = 2 AND g.state = 1 AND p.terminal_at_ms IS NULL)
              OR (p.state = 3 AND g.state = 2 AND p.terminal_at_ms = g.terminal_at_ms)
              OR (p.state = 4 AND g.state IN (3,4) AND p.terminal_at_ms = g.terminal_at_ms))
            AND g.approval_count = (SELECT count(*) FROM
              (SELECT 1 FROM %I.holdout_grant_approvals a WHERE a.grant_id = g.grant_id LIMIT 9) bounded))
         FROM %I.holdout_periods p LEFT JOIN %I.holdout_grants g ON g.period_id = p.period_id
         WHERE p.period_id = $1', TG_TABLE_SCHEMA, TG_TABLE_SCHEMA, TG_TABLE_SCHEMA)
        INTO valid USING NEW.period_id;
    IF valid IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'period, grant and approval attachment disagree' USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER period_grant_consistency AFTER INSERT OR UPDATE ON holdout_periods
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_grant_aggregate();
CREATE CONSTRAINT TRIGGER grant_period_consistency AFTER INSERT OR UPDATE ON holdout_grants
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_grant_aggregate();
CREATE CONSTRAINT TRIGGER approval_attachment_consistency AFTER INSERT ON holdout_grant_approvals
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_grant_aggregate();
