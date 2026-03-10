this is the source repo for dprint-plugin-typescript

your task is to investigate the root cause of a bug which leads to both:
1. formatting of unrelated preceding expression to affect formatting of following one
2. formatting being "unstable" and taking multiple passes to converge at stable output

I wrote 3 tests to help you with this task:
1. @tests/specs/unstable_formatting/unstableFormattingRegressionTests.txt - adds some tests cases which were missing from the existing test suite. These all succeede and should remain so.
2. @tests/specs/unstable_formatting/unstableFormattingFailingTests1.txt - contains 2 test cases with input code which differs by 1 character. Formatting in the first OK test case works as expected. The additional character in the second one leads to bad formatting of the `func` call as well as unrelated statement which follows: `a.b`;
3. @tests/specs/unstable_formatting/unstableFormattingFailingTests2.txt - similarly to the above, addition of one character (`aa` => `aaa`) leads to bad formatting. Moreover, formatting only of the `Set.add(Entity.make(x));` expressions converges on stable output after 2 passes:

first pass outputs:

```ts
  Set
    .add(Entity
      .make(x));
  Set.add(Entity.make(x));
```


second pass outputs:

```ts
  Set
    .add(Entity
      .make(x));
  Set
    .add(Entity
      .make(x));
```

You can run the all of the above tests with: `cargo test specs::unstable_formatting`
Or individual ones, for example: `specs::unstable_formatting::unstableFormattingFailingTests2`


Your main job is to explain and demonstrate the mechanism which leads to the observed behavior. If necessary add logging which could be observed during tests, and highlight the differences between processing of the OK vs BROKEN test cases. Add comments in source code at key points to highlight control flow etc. I've also added source code for dprint-core crate (under dprint-crates/core), which is used to actually print the final formatted code according to the formatting hints produced by dprint-plugin-typescript. You can add comments to dprint-core's source code as well, if appropriate.

Produce an .md report with your finding and proposed approach(es) to how this could be fixed. I will then review the plan and give it to a coding agent to attempt to fix the bug.