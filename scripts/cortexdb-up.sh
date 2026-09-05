#!/bin/sh
# Starts (or reuses) a local CortexDB instance for the `cortexdb` memory
# driver (`src/store/memory/cortexdb.rs`) — a standalone service, NOT the
# removed in-pod tinycortex engine.
#
# Binds 127.0.0.1:3141, persists to a named Docker volume, and points
# embeddings/LLM at the local ladder router. Enrichment, layers and graph
# extraction are OFF by default: this driver only ever writes
# `Content::Json` envelopes under the `tool_result` modality and reads them
# back with `view=raw`, so the extra pipelines would cost a model call per
# write for a feature this driver never uses.
#
# Usage:
#   ./scripts/cortexdb-up.sh
#   CORTEX_API_KEY=... LADDER_API_KEY=... ./scripts/cortexdb-up.sh
#
# On success, prints the OPENCOMPANY_MEMORY_* exports to run `serve` against
# this instance.
set -eu

CONTAINER_NAME=${CORTEXDB_CONTAINER_NAME:-opencompany-cortexdb}
VOLUME_NAME=${CORTEXDB_VOLUME_NAME:-opencompany-cortexdb-data}
HOST_PORT=${CORTEXDB_PORT:-3141}
IMAGE=${CORTEXDB_IMAGE:-cortexdb/cortexdb:latest}
CONFIG_DIR="${HOME}/.config/opencompany"
ENV_FILE="${CONFIG_DIR}/cortexdb.env"
LADDER_URL=${LADDER_URL:-http://host.docker.internal:6969/v1}

mkdir -p "$CONFIG_DIR"

# Persist a generated API key across runs, exactly like a real deployment's
# credential — regenerating it on every invocation would make the container's
# data unreadable by the next run.
if [ -z "${CORTEX_API_KEY:-}" ]; then
    if [ -f "$ENV_FILE" ]; then
        # shellcheck disable=SC1090
        . "$ENV_FILE"
    fi
fi
if [ -z "${CORTEX_API_KEY:-}" ]; then
    if command -v openssl >/dev/null 2>&1; then
        CORTEX_API_KEY=$(openssl rand -hex 32)
    else
        CORTEX_API_KEY=$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
    fi
    printf 'CORTEX_API_KEY=%s\n' "$CORTEX_API_KEY" > "$ENV_FILE"
    chmod 600 "$ENV_FILE"
    echo "Generated a CortexDB API key and saved it to ${ENV_FILE}." >&2
fi

if [ -z "${LADDER_API_KEY:-}" ]; then
    echo "warning: LADDER_API_KEY is not set; the embedding provider will \
reject requests until it is." >&2
fi

if docker ps --format '{{.Names}}' | grep -qx "$CONTAINER_NAME"; then
    echo "CortexDB container '${CONTAINER_NAME}' is already running." >&2
elif docker ps -a --format '{{.Names}}' | grep -qx "$CONTAINER_NAME"; then
    echo "Starting existing CortexDB container '${CONTAINER_NAME}'." >&2
    docker start "$CONTAINER_NAME" >/dev/null
else
    echo "Creating CortexDB container '${CONTAINER_NAME}' on 127.0.0.1:${HOST_PORT}." >&2
    docker run -d \
        --name "$CONTAINER_NAME" \
        --add-host=host.docker.internal:host-gateway \
        -p "127.0.0.1:${HOST_PORT}:3141" \
        -v "${VOLUME_NAME}:/data" \
        -e "CORTEX_API_KEY=${CORTEX_API_KEY}" \
        -e "CORTEX_V1_MINTER_ENABLE=true" \
        -e "CORTEX_ENRICHMENT_ENABLE=false" \
        -e "CORTEX_LAYERS_ENABLE=false" \
        -e "CORTEX_GRAPH_ENABLE=false" \
        -e "CORTEX_EMBEDDING_URL=${LADDER_URL}" \
        -e "CORTEX_EMBEDDING_MODEL=vectors" \
        -e "CORTEX_EMBEDDING_DIMS=3072" \
        -e "CORTEX_EMBEDDING_API_KEY=${LADDER_API_KEY:-}" \
        -e "CORTEX_LLM_URL=${LADDER_URL}" \
        -e "CORTEX_LLM_MODEL=flash" \
        -e "CORTEX_LLM_API_KEY=${LADDER_API_KEY:-}" \
        "$IMAGE" >/dev/null
fi

echo "Waiting for CortexDB to report ready on http://127.0.0.1:${HOST_PORT}/v1/admin/ready ..." >&2
attempt=0
until curl -fsS -o /dev/null \
    -H "Authorization: Bearer ${CORTEX_API_KEY}" \
    -H "X-Cortex-Actor: opencompany" \
    "http://127.0.0.1:${HOST_PORT}/v1/admin/ready"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 60 ]; then
        echo "CortexDB did not become ready after 60s. 'docker logs ${CONTAINER_NAME}' for detail." >&2
        exit 1
    fi
    sleep 1
done
echo "CortexDB is ready." >&2

cat <<EOF

export OPENCOMPANY_MEMORY=remote
export OPENCOMPANY_MEMORY_DRIVER=cortexdb
export OPENCOMPANY_MEMORY_URL=http://127.0.0.1:${HOST_PORT}
export OPENCOMPANY_MEMORY_API_KEY=${CORTEX_API_KEY}
export OPENCOMPANY_MEMORY_ACTOR=opencompany
EOF
