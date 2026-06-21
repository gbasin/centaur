# centaur-node-sync image — the privileged per-node sync daemon (Track C4).
# Multi-stage: build the static-ish binary, ship a minimal privileged runtime.
# Built from the api-rs workspace root context:
#   docker build -f crates/centaur-node-sync/Dockerfile -t centaur-node-sync:latest .
FROM rust:1-bookworm AS build
WORKDIR /work
COPY . .
RUN cargo build --release -p centaur-node-sync --bins

FROM debian:bookworm-slim
# The daemon needs coreutils + libc (dynamically linked) + CA certs for egress TLS.
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*
COPY --from=build /work/target/release/centaur-node-syncd /usr/local/bin/centaur-node-syncd
COPY --from=build /work/target/release/scan-demo /usr/local/bin/scan-demo
# Runs privileged (overlay xattr reads + write-through-merged); the DaemonSet sets
# securityContext.privileged=true. No USER drop here — node-root is required.
ENTRYPOINT ["/usr/local/bin/centaur-node-syncd"]
