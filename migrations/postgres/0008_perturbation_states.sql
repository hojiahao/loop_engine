CREATE TABLE perturbation_states (
    context_id TEXT PRIMARY KEY CHECK (length(context_id) BETWEEN 1 AND 128),
    revision BIGINT NOT NULL CHECK (revision > 0),
    space_blob BYTEA NOT NULL CHECK (octet_length(space_blob) BETWEEN 1 AND 1048576),
    space_sha256 BYTEA NOT NULL CHECK (octet_length(space_sha256) = 32),
    state_blob BYTEA NOT NULL CHECK (octet_length(state_blob) BETWEEN 1 AND 1048576),
    state_sha256 BYTEA NOT NULL CHECK (octet_length(state_sha256) = 32),
    updated_at_ms BIGINT NOT NULL CHECK (updated_at_ms BETWEEN 0 AND 253402300799999),
    actor_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK (operation = 'loop.perturbation.advance'),
    idempotency_key TEXT NOT NULL,
    FOREIGN KEY (actor_id, operation, idempotency_key)
        REFERENCES command_receipts (actor_id, operation, idempotency_key)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE FUNCTION fence_perturbation_state() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'perturbation history cannot be deleted' USING ERRCODE = '23514';
    END IF;
    IF NEW.context_id <> OLD.context_id OR NEW.space_blob <> OLD.space_blob
       OR NEW.space_sha256 <> OLD.space_sha256 OR NEW.revision <> OLD.revision + 1
       OR NEW.updated_at_ms < OLD.updated_at_ms THEN
        RAISE EXCEPTION 'perturbation state update is not revision fenced'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER perturbation_states_fenced BEFORE UPDATE OR DELETE ON perturbation_states
    FOR EACH ROW EXECUTE FUNCTION fence_perturbation_state();
