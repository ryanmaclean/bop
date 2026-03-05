# Roadmap Hot Folder (Sample)

Drop request files into [`drop/`](./drop) and the launchd watcher can auto-create
`🂠-*.bop` roadmap cards in `.cards/pending/`.

## Supported file types

- `.roadmap`
- `.md`
- `.txt`
- `.json`
- `.yaml` / `.yml`

## Local one-shot ingest (manual test)

```nu
nu scripts/ingest_roadmap_hotfolder.nu \
  --inbox (pwd | path join examples/roadmap-inbox/drop) \
  --cards-dir (pwd | path join .cards)
```

## launchd install (hot-folder trigger)

```nu
nu scripts/install_roadmap_hotfolder_launchd.nu \
  --inbox (pwd | path join examples/roadmap-inbox/drop) \
  --cards-dir (pwd | path join .cards)
```

Then simply copy a request file into `drop/`.
