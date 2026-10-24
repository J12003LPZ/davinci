# The runner resolves the built image ID and checks tools before spending usage.
# Pinned by digest; apt package contents below are still time-dependent.
FROM python:3.12-slim-bookworm@sha256:6a0e06f5ba0b373d051aabd3fffb9b873f27f9a2a3a14bcbe37dee6f0e3bbc70
RUN apt-get update \
    && apt-get install -y --no-install-recommends bash git ca-certificates ripgrep \
    && rm -rf /var/lib/apt/lists/* \
    && python -m pip install --no-cache-dir pytest==8.3.5
WORKDIR /workspace
