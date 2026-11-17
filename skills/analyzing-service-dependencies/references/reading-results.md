# Reading results

## Contents
- Edge status
- Edge types
- How impact is ordered
- Node attributes

## Edge status

| Status | Meaning | How to word it |
|---|---|---|
| `both` | declared and observed | "confirmed" |
| `observed_only` | traffic seen, nobody declared it | "seen in traffic, not documented" |
| `declared_only` | a file says so, no traffic seen | "documented, not seen in traffic" (may be stale, or a quiet path) |
| `candidate` | source code or config suggests it | "likely, not confirmed" |
| `n/a` | structural (deployment, placement) | not a dependency claim |

A missing edge is a gap, not proof of independence. A wrong edge looks like knowledge, so the tool shows candidates as candidates.

## Edge types

| Type | Meaning | Counts for impact |
|---|---|---|
| `declared_depends_on` | a catalog or manifest says so | yes |
| `observed_calls` | telemetry saw it, per environment | yes |
| `candidate_calls` | static evidence | yes, as a candidate |
| `routes_to` | gateway to backend | yes; a backend change reaches the gateway in front |
| `publishes_to`, `consumes_from` | messaging | yes; a change to a publisher reaches its topic, and so the topic's consumers |
| `network_traffic` | packets between addresses | no, direction is not call direction |
| `deployed_as`, `runs_in_cluster`, `has_workload` | placement | no |
| `uses_library` | library use (libraries are off by default) | yes, when libraries are enabled |

## How impact is ordered

Impact walks from the changed thing to what depends on it. Results are ranked by a path score (call share times a hardness factor: hard 1.0, unknown 0.5, soft 0.2). It ranks; it is not a probability. Depth is the number of hops. Say "closest and most critical first", not "most likely to break".

## Node attributes

`owner`, `department`, `lifecycle` come from catalog files. `system`, `languages`, `container` come from sources. An empty `owner` means no catalog file names one.
