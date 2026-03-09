# Bug Investigation: Formatting Interference and Instability

## Root Cause Summary
The bug is caused by "cache pollution" in `dprint-core`'s `Printer`. When the printer performs a **look-ahead** (to resolve a condition) or a **state restoration** (to handle a line exceeding `lineWidth`), it rolls back its writer state but **fails to roll back its resolution caches** (`resolved_conditions`, `resolved_line_numbers`, etc.).

This allows resolutions made during a "discarded" tentative formatting run to persist and incorrectly influence subsequent formatting of unrelated expressions.

## Mechanism of Failure

### 1. The Cache Problem
The `Printer` maintains several maps to cache the results of IR instructions:
- `resolved_conditions`: Stores whether a `Condition` resolved to `true` or `false`.
- `resolved_line_numbers`: Stores the line number where a `LineNumber` IR was placed.
- (And others for columns, indent levels, etc.)

These caches are global to the `print` run.

### 2. State Restoration (SavePoints)
The `Printer` uses `SavePoint`s to revert state in two main scenarios:
- **Look-ahead**: When a `Condition` (like `isMultipleLines`) depends on a `LineNumber` that hasn't been reached yet, the printer saves a `SavePoint` and tentatively formats ahead until the `LineNumber` is hit.
- **Line Wrap**: When a written string exceeds `lineWidth`, the printer may revert to a previous `PossibleNewLine` save point to break the line.

### 3. The Leak
When `Printer::update_state_to_save_point` is called, it restores the writer's position and the stack of items to format. **It does NOT restore the resolution caches.**

If, during a tentative run that is later discarded, the printer resolves a condition or records a line number for a *future* expression, that resolution stays in the cache.

### 4. Test 1 Analysis (OK vs BROKEN)
In the BROKEN case of Test 1:
```ts
const g = (_12345678901) => function() { func(_123456789012345, () => _1234567); a.b; };
```
1. The printer starts formatting the arrow function. The block `{ ... }` triggers a look-ahead to see if it's multi-line.
2. During this look-ahead, it tentatively formats the `func(...)` call and `a.b;`.
3. Because `func(...)` is long, it eventually causes a line-wrap restoration *within* the look-ahead or the look-ahead itself completes after seeing the wrap.
4. If `a.b`'s `isMultipleLines` condition is resolved during this tentative phase (e.g., while `func` was still on one line), that `false` result is cached.
5. When the printer rolls back to format the block "for real" (now knowing it's multi-line), it reaches `a.b`. Instead of resolving `isMultipleLines` based on the new (correct) multi-line layout, it hits the cache and gets the stale `false` value from the discarded tentative run.
6. This causes `a.b` to be formatted as if it were on a single line, even though the preceding code now takes multiple lines.

### 5. Test 2 Analysis (Instability)
Instability (multiple passes to converge) occurs because:
- Pass 1: Look-ahead from statement A pollutes the cache for statement B with incorrect values. Statement B formats "badly".
- Pass 2: The input text now contains the "bad" formatting from Pass 1. This changes the timing/positioning of look-aheads. The printer may no longer look as far ahead, or the tentative run resolves conditions differently. Eventually, it converges on a stable (but potentially still incorrect) state.

## Proposed Fixes in `dprint-core`

### Option A: Restore Caches in SavePoint (Recommended for correctness)
Modify `SavePoint` to include a snapshot of the `resolved_*` maps.
- **Pros**: Perfectly solves the issue.
- **Cons**: Might be slow if maps are large. However, `resolved_conditions` is already a `BumpHashMap` and `resolved_line_numbers` is a `Vec`. Cloning these might be acceptable given how often save points are used.

### Option B: Transactional Caches / Rollback Log
Record every insertion into the caches. Each `SavePoint` stores the number of resolutions made so far. When restoring a `SavePoint`, pop and remove all resolutions made after that count.
- **Pros**: High performance (no cloning of maps).
- **Cons**: Requires changing how `VecU32Map` works (needs to support removing latest entries efficiently).

### Option C: Clear caches on restore
Coarsely clear the caches when restoring a `SavePoint`.
- **Pros**: Simple.
- **Cons**: Will cause re-resolution of many things, potentially hurting performance significantly, and might even lead to infinite loops if not careful.

## Recommended Action
I recommend **Option B**. Implementing a rollback log for the caches ensures that resolutions made during a discarded tentative run never leak into the final run, preserving both correctness and performance.
