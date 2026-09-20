# 0005 — the writer never emits `<g>`

- **Status:** accepted
- **Date:** 2026-09-20
- **Phase:** 6

## Context

A run of consecutive shapes that share a fill can be wrapped in one
`<g fill="#f00">`, with each shape then dropping its own `fill`. For a run of
`n` shapes whose fill spells as six hex digits that trades `n * 13` bytes for
about 27, so it breaks even near `n = 2` and pays from `n = 3`.

The run has to be consecutive **in paint order**, and it has to stay that way.
Paint order is the whole semantics of a stacked trace: shape `k` covers
everything below it. Reordering to make runs longer would change the picture.

Measured on traced documents, runs depend entirely on the input:

| Input | Shapes | Run lengths |
|---|---|---|
| Five dots alternating between two colors | 6 | 1, 1, 1, 1, 1, 1 |
| Five dots in one color | 6 | 1, 5 |

So the win is real but conditional. On the second document a group would save
roughly 40 bytes out of a few hundred; on the first it would save nothing and
`collapseGroups` in the optimizer would have to undo any group that was written
anyway.

Against that: a `<g>` changes the document's shape for everything downstream. A
reader and a diff both stop being able to see a shape's fill without looking up
the tree, and the writer needs lookahead and state it otherwise does not have.

## Decision

`write_svg` emits every shape as a sibling of the root, each carrying its own
`fill`. No `<g>` is ever written, whatever the fills are.

## Consequences

**Gained.** The writer is a flat loop with no lookahead and no state between
shapes. Paint order is exactly the order of elements, which is what a reader, a
diff, and a snapshot test all expect. Every shape's fill is visible where the
shape is.

**Lost.** A document with long same-fill runs is larger than it could be, by
about `13 * (n - 2)` bytes per run of `n`. The measurement above shows such runs
exist whenever the source repeats one color across separate regions, which is
common in icon sets.

**Accepted.** This is the one place where the plan's "fewest bytes" goal is
knowingly not met. Grouping can be added later as a post-pass over the emitted
element list that merges only runs already adjacent in paint order, so it can
never reorder anything, and it should ship with a measurement of the byte saving
on real inputs rather than on the two synthetic documents above.
