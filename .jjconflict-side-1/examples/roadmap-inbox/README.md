# Roadmap Hot Folder (Sample)

Drop request files into [`drop/`](./drop) and the launchd watcher can auto-create
`🂠-*.jobcard` roadmap cards in `.cards/pending/`.

## Supported file types

- `.roadmap`
- `.md`
- `.txt`
- `.json`
- `.yaml` / `.yml`

## Local one-shot ingest (manual test)

```zsh
scripts/ingest_roadmap_hotfolder.zsh \
  --inbox "$(pwd)/examples/roadmap-inbox/drop" \
  --cards-dir "$(pwd)/.cards"
```

## launchd install (hot-folder trigger)

```zsh
scripts/install_roadmap_hotfolder_launchd.zsh \
  --inbox "$(pwd)/examples/roadmap-inbox/drop" \
  --cards-dir "$(pwd)/.cards"
```

Then simply copy a request file into `drop/`.
