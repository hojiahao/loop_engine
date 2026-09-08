FROM docker.m.daocloud.io/library/postgres:17.11-bookworm@sha256:051f7b7b3abdd564d5d1bd1e8c4b9c1b6e77087d1dd22020ede611c096a272e0
COPY infra/postgres/test-entrypoint.sh /usr/local/bin/loop-test-postgres
ENTRYPOINT ["bash", "/usr/local/bin/loop-test-postgres"]
