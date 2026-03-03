# bop quickstart

This example shows the minimal setup to run bop in your project.

## 1. Initialize

```zsh
cd your-project
bop init          # creates .cards/ scaffold
```

Or copy this directory:
```zsh
cp -r examples/quickstart/.cards your-project/.cards
```

## 2. Create a card

```zsh
bop new implement add-auth
# → .cards/pending/add-auth.card/
```

Edit `.cards/pending/add-auth.card/spec.md` with what you want built.

## 3. Dispatch

```zsh
bop dispatcher --once
# Picks a pending card, runs the adapter, moves to done/ or failed/
```

## 4. Review and merge

```zsh
bop status                    # see board
bop inspect add-auth          # see logs + output
bop merge-gate --once         # run acceptance criteria, merge if green
```

## Directory layout

```
.cards/
├── pending/          ← cards waiting to run
├── running/          ← currently executing (auto-managed)
├── done/             ← finished, awaiting merge
├── merged/           ← landed on main
├── failed/           ← errored or rejected
├── templates/        ← card templates for `bop new`
│   └── implement.card/
├── providers.json    ← adapter config + cooldowns
└── .locks/           ← dispatcher mutex
```
