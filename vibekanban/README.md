# vibekanban JobCard Provider

Integrates JobCard's `.cards/` filesystem with [vibekanban-cli](https://github.com/chasebuild/vibekanban-cli).

## Usage

```zsh
npx vibe-kanban --config vibekanban/config.json
```

## Card → Column Mapping

| `.cards/` state | vibekanban column |
|-----------------|-------------------|
| `pending/`      | Backlog           |
| `running/`      | In Progress       |
| `done/`         | Review            |
| `merged/`       | Done              |
| `failed/`       | Blocked           |

## Provider Script

`bop-provider.nu` polls the `.cards/` directory and outputs a JSON array of tasks.
Override the cards directory with `CARDS_DIR=path nu ./vibekanban/bop-provider.nu`.

## Actions

Actions map vibekanban UI buttons to `jc` CLI commands:
- Retry: `jc retry <id>`
- Kill: `jc kill <id>`
- Logs: `jc logs <id> --follow`
