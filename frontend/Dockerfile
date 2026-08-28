# syntax=docker/dockerfile:1

# ── build the SPA ──────────────────────────────────────────────────────────
# The two `node:22-slim` tags below are the only copies of the console's Node
# version that cannot read `.nvmrc` (issue #858) — a `FROM` line takes no build
# arg by default and CI resolves the file instead. So these and `frontend/.nvmrc`
# are the pair to bump together; bumping only one means CI stops being evidence
# for the thing that deploys.
FROM node:22-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npm run build && npm run build:pages-sdk

# ── local development ─────────────────────────────────────────────────────
# Source is bind-mounted over /app; node_modules lives in a named volume.
FROM node:22-slim AS development
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci

# ── serve + proxy the API ──────────────────────────────────────────────────
# nginx serves the static bundle and proxies the API to the host. The upstream
# is configurable so the same image works under Compose (service name),
# single-task ECS (localhost), or any host:port.
FROM nginx:1.27-alpine AS runtime
ENV OC_UPSTREAM=opencompany:8080
COPY nginx/default.conf.template /etc/nginx/templates/default.conf.template
COPY --from=build /app/dist /usr/share/nginx/html
EXPOSE 80
