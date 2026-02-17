# CLAUDE.md — Robrix Monorepo

## Repo Structure

| Directory | Purpose |
|-----------|---------|
| `app/` | Robrix Matrix chat client (Rust / Makepad). See `app/CLAUDE.md` for details. |
| `aspire/` | .NET Aspire orchestration for local Matrix infrastructure (Synapse + Postgres + Element Web). |
| `.cargo/` | Shared Cargo config (Cargo walks up from `app/` and finds it here). |

## Quick Start

### 1. Start local Matrix infrastructure
```sh
cd aspire/Robrix.AppHost && dotnet run
```
Opens the Aspire dashboard at `http://localhost:15178`. Wait for all resources to show green.

### 2. Bootstrap via Element Web
Open `http://localhost:8088` and register a test user (e.g. `testuser` / `testpassword`). Create rooms as needed.

### 3. Run Robrix
```sh
cd app && cargo run --release
```
Connect to `http://localhost:8008` and log in with the test user.

## Port Map

| Service | Host Port | Notes |
|---------|-----------|-------|
| Synapse (client API) | 8008 | Matrix homeserver with native Sliding Sync |
| Element Web | 8088 | Browser-based Matrix client |
| PostgreSQL | 15432 | Synapse database backend |
| Aspire Dashboard | 15178 | Resource monitoring UI |

## Isolation

All containers and volumes are prefixed `robrix-` to avoid conflicts with other projects on the same machine.
