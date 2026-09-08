\set ON_ERROR_STOP on

-- Run once as a deployment administrator. Existing names cause an error;
-- this script never adopts, resets, or drops an existing database or role.
BEGIN;
CREATE ROLE loop_engine_owner NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE
    NOREPLICATION NOBYPASSRLS;
CREATE ROLE loop_engine_app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE
    NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 20;
COMMENT ON ROLE loop_engine_owner IS 'Loop Engine schema owner; deployment administration only';
COMMENT ON ROLE loop_engine_app IS 'Loop Engine runtime; no schema ownership or administrative membership';
COMMIT;

CREATE DATABASE loop_engine OWNER loop_engine_owner ENCODING 'UTF8' TEMPLATE template0;
REVOKE ALL ON DATABASE loop_engine FROM PUBLIC;
GRANT CONNECT ON DATABASE loop_engine TO loop_engine_app;
ALTER DATABASE loop_engine SET timezone = 'UTC';
ALTER DATABASE loop_engine SET statement_timeout = '30s';
ALTER DATABASE loop_engine SET lock_timeout = '5s';
ALTER DATABASE loop_engine SET idle_in_transaction_session_timeout = '15s';

\connect loop_engine
BEGIN;
REVOKE ALL ON SCHEMA public FROM PUBLIC;
ALTER SCHEMA public OWNER TO loop_engine_owner;
GRANT USAGE ON SCHEMA public TO loop_engine_app;
ALTER DEFAULT PRIVILEGES FOR ROLE loop_engine_owner IN SCHEMA public
    GRANT SELECT, INSERT ON TABLES TO loop_engine_app;
ALTER DEFAULT PRIVILEGES FOR ROLE loop_engine_owner IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO loop_engine_app;
COMMIT;

-- Set the application password separately with psql's interactive \password.
-- No password or bearer credential belongs in this file or psql command arguments.
