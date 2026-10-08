-- Additive observation support. Keep immutable audit rows and all existing
-- writer fences; rollback may retain this index without rewriting history.
CREATE INDEX audit_events_job_sequence ON audit_events (job_id, sequence)
    WHERE job_id IS NOT NULL;
