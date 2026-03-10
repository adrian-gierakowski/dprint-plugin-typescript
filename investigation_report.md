# Bug Investigation Report: Unstable Formatting and State Leak in Member Expressions

## Executive Summary
The investigation revealed a fundamental state leak in the `dprint-core` printer combined with non-deterministic behavior in `dprint-plugin-typescript` when handling member expressions. Specifically, resolved layout information (like line numbers and condition results) persists across printer restorations (save points), leading to incorrect formatting of subsequent expressions. This "wrong" formatting then becomes "locked in" during subsequent passes because the plugin makes decisions based on node positions in the source text.

## Root Cause Analysis

### 1. State Leak in `dprint-core` Printer
The core of the issue lies in how `dprint-core` handles "look-ahead" conditions. When the printer encounters a condition that depends on information not yet known (e.g., a line number of a future node), it:
1.  Creates a `SavePoint` of its current state.
2.  Continues printing speculatively.
3.  Once the required information is resolved, it restores to the `SavePoint` and re-evaluates the condition.

**The Bug:** The `Printer` maintains several maps of resolved information (`resolved_line_numbers`, `resolved_conditions`, `resolved_column_numbers`, etc.). When `update_state_to_save_point` is called, these "resolved" maps are **not** restored to their state at the time of the save point.

**The Consequence:** Information resolved during a speculative pass remains in the map even after the printer "goes back in time" to the save point. If the subsequent re-evaluation changes the layout (e.g., by introducing a newline), all previously resolved information that followed that point is now **wrong** (e.g., it has the wrong line numbers), but it remains in the map and is never updated.

### 2. Mechanism of "Unrelated Expression" Interference
In member expressions like `aaa.b.c`, each dot uses an `isMultipleLines` condition that looks ahead to the end of the expression.
-   The printer speculatively prints past the dots to find the end of the expression.
-   If it encounters subsequent statements (like `Set.add(...)`) during this speculative pass, it may resolve their conditions/infos based on the *current* (possibly incorrect) speculative layout.
-   When it finally resolves the look-ahead for `aaa.b.c` and restores to the first dot, the "wrong" resolutions for `Set.add(...)` are still in the map.
-   This causes `Set.add(...)` to be printed using stale, incorrect layout information.

### 3. Instability and Multiple Passes
The instability (taking multiple passes to converge) is caused by a feedback loop between the printer and the plugin:
1.  **Pass 1:** `dprint-core` produces a slightly incorrect layout due to the state leak (e.g., it breaks a member expression that should have stayed on one line).
2.  **Pass 2:** `dprint-plugin-typescript` parses the output of Pass 1. Its IR generation logic often looks at the source text to decide whether to force newlines (e.g., `get_use_new_lines_for_nodes`).
3.  Because Pass 1 produced a newline, the plugin now sees a newline in the source and generates IR that **forces** a newline (`force_use_new_line: true`), regardless of the `isMultipleLines` condition.
4.  This "locks in" the layout, making it stable from Pass 2 onwards, even if it's not what was originally intended.

## Demonstrated Failure Cases

### Failing Test 1 (`unstableFormattingFailingTests1.txt`)
A one-character difference in a parameter name causes `func(...)` to just hit the line width limit. This triggers a complex sequence of look-aheads and restorations. The state leak causes the subsequent `a.b;` statement to resolve its `isMultipleLines` condition to `true` (circularly, because it was already pushed to a new line in a discarded speculative pass), leading to bad formatting.

### Failing Test 2 (`unstableFormattingFailingTests2.txt`)
Similarly, `aaa.b.c` causes speculative printing that reaches the first `Set.add` statement. The first `Set.add` gets "polluted" state and breaks. The second `Set.add` is far enough away that it might not be reached in the same speculative pass, so it stays on one line in Pass 1. In Pass 2, the first `Set.add` is now multi-line in the source, so the plugin forces it to stay multi-line, and the second one now gets polluted and breaks too.

## Proposed Approach for Fixing

### Option A: Restore Resolved Maps in `SavePoint` (Recommended)
Modify `dprint-core`'s `SavePoint` to include copies of the `resolved_*` maps and restore them in `update_state_to_save_point`.
-   **Pros:** Guaranteed correctness; prevents all forms of resolution state leak.
-   **Cons:** Potential performance impact due to frequent cloning of maps. (Note: `VecU32U32Map` is just a `Vec<u32>`, so cloning is relatively fast).

### Option B: Resolution Versioning / Undo Log
Track the number of resolutions in each map at the time a `SavePoint` is created. When restoring, truncate the maps (or undo resolutions) to that count.
-   **Pros:** High performance; minimal cloning.
-   **Cons:** More complex implementation; requires maps to support truncation or ordered removal.

### Option C: Explicit Clearing in Plugin (Workaround)
Add `clearWhenPositionChanges` to member expressions in `dprint-plugin-typescript`.
-   **Pros:** No changes needed to `dprint-core`.
-   **Cons:** Doesn't fix the underlying bug in the core; might not cover all cases; adds complexity to the plugin.

## Conclusion
The bug is a fundamental state management issue in `dprint-core`. Fixing it there (Option A or B) is the most robust solution and will likely fix other hidden formatting bugs and instabilities across all dprint plugins.
