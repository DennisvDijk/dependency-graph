---
description: What breaks if a component changes, and who to tell
argument-hint: <component> [dev|acc|prod]
---

Use the depgraph skill. Run `depgraph impact $1 --env ${2:-prod}` and, if a snapshot is missing, say how to build one instead of guessing. Report: what breaks (closest first), the owners to tell, and which ties are only `candidate` (inferred, not confirmed).
