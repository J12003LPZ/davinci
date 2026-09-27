# The runner resolves the built image ID and checks tools before spending usage.
FROM python:3.12-slim-bookworm
RUN apt-get update \
    && apt-get install -y --no-install-recommends bash git ca-certificates ripgrep \
    && rm -rf /var/lib/apt/lists/* \
    && python -m pip install --no-cache-dir pytest==8.3.5
WORKDIR /workspace
