# Zellij Layout for JobCard

## Launch

```zsh
# Build the WASM status bar plugin first:
cargo build --manifest-path crates/jc-zellij-plugin/Cargo.toml --target wasm32-wasip1 --release
cp crates/jc-zellij-plugin/target/wasm32-wasip1/release/jc_zellij_plugin.wasm target/bop-status.wasm

# Then launch:
zellij --layout zellij/bop.kdl
```

## Layout

- **Dispatchers tab**: three dispatcher panes (team-cli, team-arch, team-quality)
- **vibekanban tab**: kanban board showing card states
- **Status bar**: shows per-team card counts (updates every 2s via WASM plugin)

## Adaptive Tier Logic

The dispatcher auto-manages panes based on active card count:
- 1–5 running cards: one pane per card (shows live stdout.log)
- 6–20 running cards: one pane per team
- 21+: status bar only (no per-card panes)
