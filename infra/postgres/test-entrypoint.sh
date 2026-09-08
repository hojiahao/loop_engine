#!/usr/bin/env bash
set -euo pipefail

test "${POSTGRES_DB:-}" = loop_engine_test
install -d -m 0700 -o postgres -g postgres /run/loop-test-tls
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj /CN=loop-engine-test \
  -keyout /run/loop-test-tls/server.key -out /run/loop-test-tls/server.crt 2>/dev/null
chown postgres:postgres /run/loop-test-tls/server.key /run/loop-test-tls/server.crt
chmod 0600 /run/loop-test-tls/server.key
exec docker-entrypoint.sh postgres \
  -c ssl=on \
  -c ssl_cert_file=/run/loop-test-tls/server.crt \
  -c ssl_key_file=/run/loop-test-tls/server.key \
  -c shared_buffers=32MB -c work_mem=1MB -c max_connections=100
