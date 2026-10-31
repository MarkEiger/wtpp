# Development workflow

wt++ is built in a loop until it matches Windows Terminal and goes beyond it.

```
 ┌────────────────────────────────────────────────────────────┐
 │ 1. Gap check: anything missing vs. WT (docs/PARITY.md)?    │── nothing open ──▶ done
 └──────────────┬─────────────────────────────────────────────┘
                ▼ items open
 ┌────────────────────────────────────────────────────────────┐
 │ 2. Implement the open items (highest value first)          │
 └──────────────┬─────────────────────────────────────────────┘
                ▼
 ┌────────────────────────────────────────────────────────────┐
 │ 3. Run and test every feature                              │◀──┐
 └──────────────┬─────────────────────────────────────────────┘   │
                ▼ bugs found                                       │
 ┌────────────────────────────────────────────────────────────┐   │
 │ 4. Fix every bug found                                     │───┘
 └────────────────────────────────────────────────────────────┘
        no bugs found ──▶ back to 1
```

## 1. Gap check
- Walk `docs/PARITY.md` top to bottom. Any `[ ]` item is open.
- Add anything newly noticed (user reports, WT features not listed yet).
- Nothing open → the app is done.

## 2. Implement
- Pick open items, highest user value first.
- Each feature lands with unit tests for its logic (key encoding, config,
  layout, parsing…) and a scenario in `scripts/windows-test/scenarios.txt`
  when it has visible behavior.
- Mark the item `[x]` only after step 3 passes.

## 3. Test
1. `cargo test --workspace`
2. `cargo clippy --workspace --all-targets` and
   `cargo clippy --workspace --target x86_64-pc-windows-gnu` (no warnings)
3. `scripts/install-windows.sh` (close wt++ first)
4. `scripts/windows-test/run.sh`, then look at every screenshot. A scenario
   passes when the screenshot shows the expected state and nothing else is
   off (layout, colors, sizes, text).
5. Anything wrong is a bug: write it under "Bugs" in `docs/PARITY.md`.

## 4. Fix
- Fix every listed bug, with a regression test or scenario where possible.
- Back to step 3. Only a full clean pass of step 3 returns to step 1.
