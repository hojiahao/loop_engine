-- One immutable projection per numerical success. Historical successes retain
-- an explicit unverified baseline; opaque protobuf blobs are not reinterpreted.
CREATE TABLE factor_evaluations (
    job_id TEXT PRIMARY KEY REFERENCES factor_trials(job_id),
    job_revision BIGINT NOT NULL CHECK (job_revision > 1),
    record_sha256 BYTEA NOT NULL CHECK (octet_length(record_sha256) = 32),
    context_sha256 BYTEA CHECK (octet_length(context_sha256) = 32),
    evidence_blob BYTEA CHECK (octet_length(evidence_blob) BETWEEN 1 AND 4194304),
    evidence_sha256 BYTEA CHECK (octet_length(evidence_sha256) = 32),
    CHECK ((context_sha256 IS NULL AND evidence_blob IS NULL AND evidence_sha256 IS NULL)
        OR (context_sha256 IS NOT NULL AND evidence_blob IS NOT NULL AND evidence_sha256 IS NOT NULL))
);
INSERT INTO factor_evaluations (job_id, job_revision, record_sha256)
    SELECT job_id, revision, record_sha256 FROM jobs WHERE kind = 2 AND state = 4;
CREATE INDEX factor_evaluations_context ON factor_evaluations(context_sha256, job_id)
    WHERE context_sha256 IS NOT NULL;
CREATE TRIGGER factor_evaluations_immutable BEFORE UPDATE OR DELETE ON factor_evaluations
    FOR EACH ROW EXECUTE FUNCTION reject_immutable_change();

CREATE FUNCTION verify_factor_evaluation() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE valid BOOLEAN;
BEGIN
    EXECUTE format(
        'SELECT CASE WHEN j.kind = 2 AND j.state = 4 THEN
            e.job_id IS NOT NULL AND e.job_revision = j.revision
            AND e.record_sha256 = j.record_sha256 AND e.evidence_blob IS NOT NULL
          ELSE e.job_id IS NULL END
         FROM %I.jobs j LEFT JOIN %I.factor_evaluations e ON e.job_id = j.job_id
         WHERE j.job_id = $1', TG_TABLE_SCHEMA, TG_TABLE_SCHEMA)
         INTO valid USING NEW.job_id;
    IF valid IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'numerical success requires one atomic evaluation'
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END;
$$;
CREATE CONSTRAINT TRIGGER completed_factor_evaluation AFTER INSERT OR UPDATE ON jobs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_factor_evaluation();
CREATE CONSTRAINT TRIGGER factor_evaluation_binding AFTER INSERT ON factor_evaluations
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION verify_factor_evaluation();
