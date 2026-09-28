-- A projection must not silently omit already committed development rejections.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM jobs WHERE kind = 3 AND state = 5) THEN
        RAISE EXCEPTION 'existing development rejections require evidence migration'
            USING ERRCODE = '23514';
    END IF;
END;
$$;

CREATE TABLE backtest_rejections (
    job_id TEXT PRIMARY KEY REFERENCES jobs(job_id),
    job_revision BIGINT NOT NULL CHECK (job_revision > 1),
    context_sha256 BYTEA NOT NULL CHECK (octet_length(context_sha256) = 32),
    rejection_code INTEGER NOT NULL CHECK (rejection_code BETWEEN 2 AND 9),
    committed_at_ms BIGINT NOT NULL CHECK (committed_at_ms BETWEEN 0 AND 253402300799999)
);
CREATE INDEX backtest_rejections_context ON backtest_rejections (context_sha256, job_id)
    WHERE rejection_code IN (4, 5, 6);
CREATE TRIGGER backtest_rejections_no_update BEFORE UPDATE OR DELETE ON backtest_rejections
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();

CREATE FUNCTION verify_backtest_rejection() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE valid BOOLEAN;
BEGIN
    EXECUTE format(
        'SELECT CASE WHEN j.kind = 3 AND j.state = 5 THEN
            r.job_id IS NOT NULL AND r.job_revision = j.revision
            AND r.committed_at_ms = j.updated_at_ms
          ELSE r.job_id IS NULL END
         FROM %I.jobs j LEFT JOIN %I.backtest_rejections r ON r.job_id = j.job_id
         WHERE j.job_id = $1', TG_TABLE_SCHEMA, TG_TABLE_SCHEMA)
         INTO valid USING NEW.job_id;
    IF valid IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'development rejection requires one atomic record'
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER rejected_backtest_record AFTER INSERT OR UPDATE ON jobs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_backtest_rejection();
CREATE CONSTRAINT TRIGGER rejection_job_binding AFTER INSERT ON backtest_rejections
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_backtest_rejection();
