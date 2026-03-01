# jc poker — Planning Poker

Add a `poker` subcommand to `jc` for async, multi-participant estimation using
playing card glyphs as estimate tokens.

## Why

The `glyph` field in `meta.json` encodes team + priority as a playing card.
Planning poker makes glyph assignment a ceremony rather than a unilateral label:
participants submit secretly, reveal simultaneously, outliers surface disagreement.

## Encoding

```
suit  = estimating perspective   ♠=complexity  ♥=effort  ♦=risk  ♣=value
rank  = magnitude                Ace=1pt  2-10=face  J=13  Q=21  K=40
🃏   = needs breakdown (blocks consensus)
```

## Commands

```
jc poker open    <card-id>           Open a round (sets poker_round=open)
jc poker submit  <card-id> [glyph]   Submit estimate; interactive picker if no glyph
jc poker reveal  <card-id>           Flip all cards, print spread, detect outliers
jc poker status  <card-id>           Who has submitted (names only, not glyphs)
jc poker consensus <card-id> <glyph> Commit agreed glyph to meta.json, close round
```

## Data model additions to Meta

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub poker_round: Option<String>,           // "open" | "revealed" | None

#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
pub estimates: BTreeMap<String, String>,   // participant → glyph
```

## Interactivity

`jc poker submit <card-id>` with no glyph arg: display a suit×rank grid,
let user navigate with arrow keys and confirm with Enter. Show face-down
card (🂠) until reveal.

`jc poker reveal` output format:
```
  alice   🂻  Jack of Hearts  — effort: 13pt
  bob     🃁  Ace of Diamonds — risk:    1pt
  claude  🂻  Jack of Hearts  — effort: 13pt

  Spread: 12pt  ← outlier: bob (♦1 vs ♥J median)
```

## Joker rule

If any participant submits 🃏, consensus is blocked. Output:
```
  ⊘ bob played 🃏 — card needs breakdown before estimation
```

## Acceptance criteria

- `cargo test`
- `cargo clippy -- -D warnings`
- `jc poker open jc-poker && jc poker submit jc-poker && jc poker reveal jc-poker`
