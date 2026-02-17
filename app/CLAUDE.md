# CLAUDE.md — Robrix App

> **Note**: All build and cargo commands in this file should be run from the `app/` directory.

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Robrix is a Matrix chat client written in Rust using the Makepad UI framework and Project Robius app development framework. It targets desktop (Windows, macOS, Linux) and mobile (Android, iOS) platforms.

**Important**: Robrix only works with Matrix homeservers that support native Sliding Sync (like Element X).

## Build Commands

### Desktop (Windows/macOS/Linux)
```sh
cargo run --release
```

Linux/WSL dependencies:
```sh
sudo apt-get install libssl-dev libsqlite3-dev pkg-config binfmt-support libxcursor-dev libx11-dev libasound2-dev libpulse-dev libwayland-dev libxkbcommon-dev
```

### Mobile

First install cargo-makepad:
```sh
cargo install --force --git https://github.com/makepad/makepad.git --branch dev cargo-makepad
```

**Android:**
```sh
cargo makepad android install-toolchain
cargo makepad android run -p robrix --release
```

**iOS Simulator:**
```sh
rustup toolchain install nightly
cargo makepad apple ios install-toolchain
cargo makepad apple ios --org=rs.robius --app=robrix run-sim -p robrix --release
```

### Linting/Formatting
```sh
cargo clippy --workspace --all-features
cargo fmt
```

Formatting rules: max line width 100 chars (see rustfmt.toml).

### Packaging for Distribution
```sh
cargo +stable install --force --locked cargo-packager
cargo install --locked --git https://github.com/project-robius/robius-packaging-commands.git
cargo packager --release
```

## Architecture

### Core Modules

- **src/sliding_sync.rs** (~4000 lines): The nerve center - handles Matrix client initialization, sliding sync service, timeline management, message sending/editing, device verification, and media fetching. Any Matrix-related changes will likely touch this file.

- **src/home/room_screen.rs** (~2200 lines): Timeline rendering and message display. Performance-critical for smooth scrolling.

- **src/home/rooms_list.rs** (~2000 lines): Room list management and real-time updates.

- **src/space_service_sync.rs**: Space/hierarchy management.

- **src/persistence/**: Data serialization - `matrix_state.rs` (session), `app_state.rs` (settings), `tsp_state.rs` (wallet).

### UI Framework Pattern

Robrix uses Makepad's `live_design!` macro for declarative UI. Key concepts:

1. UI components are defined using the `live_design!` DSL
2. All home/* modules register their designs in `src/home/mod.rs`
3. Platform-specific layouts: `main_desktop_ui.rs` vs `main_mobile_ui.rs`
4. Widget events flow through Action enums to state mutations

### Threading Model

- Tokio async runtime for concurrent tasks
- `crossbeam-channel` for cross-thread communication
- Long-running tasks spawn to tokio runtime with async request queuing (in sliding_sync.rs)
- Eyeball/eyeball_im for reactive observables

### Caching Architecture

- **Avatar cache** (`avatar_cache.rs`): AvatarUpdate messages + disk persistence
- **Media cache** (`media_cache.rs`): Per-room media caching with size limits
- **User profile cache** (`profile/user_profile_cache.rs`): Lazy-loaded with updates
- **Link preview cache** (`home/link_preview.rs`): Rate-limited preview fetching

## Cargo Features

- `tsp`: Enables TSP (Trust Spanning Protocol) wallet support
- `hide_windows_console`: Hides Windows console window
- `log_room_list_diffs`, `log_timeline_diffs`, `log_space_service_diffs`: Debug logging

## Build Profiles

- `debug-opt`: Optimized debug builds (fast iteration with debugging)
- `release-lto`: Release with thin LTO
- `distribution`: Full optimization with fat LTO (for packaging)

## Adding New UI Components

1. Create module in appropriate directory (e.g., `src/home/`, `src/shared/`)
2. Add `live_design!` macro defining the widget
3. Export from parent `mod.rs`
4. Register design in `src/home/mod.rs` if it's a home component
5. Add event handling in `src/app.rs` if needed

## TSP Integration

The `tsp` feature is optional. When disabled, `src/tsp_dummy/mod.rs` provides stub implementations. When making changes, ensure both TSP-enabled and TSP-disabled builds work.

## Key Dependencies

- **makepad-widgets**: UI toolkit (from git, dev branch)
- **matrix-sdk**, **matrix-sdk-ui**: Matrix protocol (from git, main branch)
- **ruma**: Matrix types (patched version with TSP signature field support)

Note: Several dependencies use patched versions (see `[patch.crates-io]` in Cargo.toml) for compatibility.
