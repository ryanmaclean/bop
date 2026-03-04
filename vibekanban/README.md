# vibekanban bop Provider

Integrates bop's `.cards/` filesystem with [vibekanban-cli](https://github.com/chasebuild/vibekanban-cli).

## Usage

```sh
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

Actions map vibekanban UI buttons to `bop` CLI commands:
- Retry: `bop retry <id>`
- Kill: `bop kill <id>`
- Logs: `bop logs <id> --follow`
