-- Historical successes require explicit evidence migration; never synthesize metrics.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM jobs WHERE kind IN (3, 7) AND state = 4) THEN
        RAISE EXCEPTION 'existing successful backtests require evidence migration'
            USING ERRCODE = '23514';
    END IF;
END;
$$;

CREATE TABLE backtest_results (
    job_id TEXT PRIMARY KEY REFERENCES jobs(job_id),
    job_revision BIGINT NOT NULL CHECK (job_revision > 1),
    backtest_id TEXT NOT NULL CHECK (length(backtest_id) BETWEEN 1 AND 128),
    engine INTEGER NOT NULL CHECK (engine BETWEEN 1 AND 3),
    manifest_sha256 BYTEA NOT NULL CHECK (octet_length(manifest_sha256) = 32),
    result_blob BYTEA NOT NULL CHECK (octet_length(result_blob) BETWEEN 1 AND 4194304),
    result_sha256 BYTEA NOT NULL CHECK (octet_length(result_sha256) = 32),
    committed_at_ms BIGINT NOT NULL CHECK (committed_at_ms BETWEEN 0 AND 253402300799999),
    UNIQUE (backtest_id, engine)
);
CREATE TRIGGER backtest_results_no_update BEFORE UPDATE OR DELETE ON backtest_results
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();

CREATE FUNCTION verify_backtest_result() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE valid BOOLEAN;
BEGIN
    EXECUTE format(
        'SELECT CASE WHEN j.kind IN (3, 7) AND j.state = 4 THEN
            r.job_id IS NOT NULL AND r.job_revision = j.revision
            AND r.committed_at_ms = j.updated_at_ms
          ELSE r.job_id IS NULL END
         FROM %I.jobs j LEFT JOIN %I.backtest_results r ON r.job_id = j.job_id
         WHERE j.job_id = $1', TG_TABLE_SCHEMA, TG_TABLE_SCHEMA)
         INTO valid USING NEW.job_id;
    IF valid IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'successful backtest requires one atomic result' USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER completed_backtest_result AFTER INSERT OR UPDATE ON jobs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_backtest_result();
CREATE CONSTRAINT TRIGGER result_job_binding AFTER INSERT ON backtest_results
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_backtest_result();
