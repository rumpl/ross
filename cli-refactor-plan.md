# CLI Refactoring Plan

## Current State
- `cli/src/main.rs` is ~1500 lines containing all commands, handlers, and utilities
- All command definitions and implementations are in a single file

## Target Structure
```
cli/src/
├── main.rs              # Entry point, CLI struct, command dispatch
├── commands/
│   ├── mod.rs           # Module exports
│   ├── health.rs        # Health command
│   ├── image.rs         # Image commands and handlers
│   └── container.rs     # Container commands and handlers
└── utils.rs             # Shared utilities (format_size, format_timestamp, etc.)
```

## File Contents

### `cli/src/main.rs` (~50-80 lines)
- `Cli` struct with global args (host, port)
- Top-level `Commands` enum (Health, Image, Container)
- `main()` function with command dispatch
- Imports from commands module

### `cli/src/commands/mod.rs`
- Public exports for submodules
- Re-export command enums and handlers

### `cli/src/commands/health.rs` (~30 lines)
- `health_check()` function

### `cli/src/commands/image.rs` (~350 lines)
- `ImageCommands` enum with all 8 subcommands
- `handle_image_command()` dispatcher
- All image handler functions:
  - `image_list()`
  - `image_inspect()`
  - `image_pull()`
  - `image_push()`
  - `image_build()`
  - `image_remove()`
  - `image_tag()`
  - `image_search()`

### `cli/src/commands/container.rs` (~700 lines)
- `ContainerCommands` enum with all 16 subcommands
- `handle_container_command()` dispatcher
- All container handler functions:
  - `container_create()`
  - `container_start()`
  - `container_stop()`
  - `container_restart()`
  - `container_list()`
  - `container_inspect()`
  - `container_remove()`
  - `container_pause()`
  - `container_unpause()`
  - `container_logs()`
  - `container_exec()`
  - `container_attach()`
  - `container_wait()`
  - `container_kill()`
  - `container_rename()`
  - `container_stats()`
- Helper functions: `calculate_cpu_percent()`, `calculate_memory()`

### `cli/src/utils.rs` (~50 lines)
- `format_size()` - bytes to human readable
- `format_timestamp()` - prost Timestamp to string
- Any other shared utilities

## Implementation Notes
- Keep imports organized at the top of each file
- Use `pub` appropriately for cross-module access
- Command enums need `#[derive(Subcommand)]` from clap
- Handler functions should be `pub(crate)` or `pub` as needed
