# ADR-006: Volledige scan, met groepen en projecten als opt-in

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Vervangt de groepsplicht uit [ADR-001](ADR-001-aannames.md), vraag 2.

## Context

Een verplichte lijst `gitlab.groups` blokkeert een eerste scan zolang de paden niet vaststaan. Dennis wil standaard alles zien wat het token mag zien, en alleen vernauwen als hij een groep, een subgroep, of een los project noemt.

## Besluit

1. `gitlab.groups` en `gitlab.projects` zijn allebei optioneel.
2. Zijn ze allebei leeg, dan is de scan `GET /projects?membership=false&archived=false&simple=true` (geen `archived=false` als `exclude_archived` uit staat). Paginatie is keyset (`pagination=keyset&order_by=id&sort=asc`) wanneer GitLab een `Link: rel="next"` stuurt, anders `x-next-page`. `per_page`, `max_concurrency` en `Retry-After` op 429 blijven gelden.
3. Een pad in `gitlab.groups` is een groep of een subgroep, bijvoorbeeld `platform` of `org/infra/platform`. Eén pad haalt elk project daaronder op via `GET /groups/:id/projects?include_subgroups=true`. `:id` is het pad, URL-gecodeerd.
4. `gitlab.projects` is een lijst projectpaden, bijvoorbeeld `org/other-team/ingress-gateway`, elk via `GET /projects/:url-encoded-path`.
5. Groepen en projecten mogen samen. Het resultaat is de vereniging, ontdubbeld op project-id.
6. Filters gelden na de groeps- of volledige lijst, en zijn allemaal optioneel: `include_paths` en `exclude_paths` (glob op `path_with_namespace`), `topics` (any-of), `exclude_forks` (default true), `exclude_archived` (default true), `min_last_activity` (`180d`, `24h`, `2w`), `visibility` (`public`, `internal`, `private`).
7. Expliciete projecten slaan include/exclude, topics, visibility, activiteit en forks over. Een gearchiveerd expliciet project wordt overgeslagen, met een warning op stderr.
8. De voorbeeldconfig doet een volledige scan en laat forks en gearchiveerde projecten weg.

## Gevolgen

- De paden van Dennis' team zijn geen blokkade meer. Zet ze in `groups` of `projects` als de scan te breed is.
- `max_concurrency` begrenst het uitlezen per project. De pagina's zelf blijven achter elkaar, omdat keyset dat vraagt.
- Het token blijft uit logs en uit de CSV.
