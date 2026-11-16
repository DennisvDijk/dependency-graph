# ADR-009: SBOM-digestkoppeling wordt overgeslagen

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Volgt [ADR-001](ADR-001-aannames.md) vraag 8.

## Context

F2-4 vroeg om `sbom_links` of, als dat duur is, een ADR die de koppeling overslaat. Er zijn geen interne libraries. `libraries.enabled` staat uit. Collectors mogen geen `Library`-nodes en geen `uses_library`-edges schrijven.

## Besluit

`sbom_links` blijft een stub. Digest-, sha- en tag-koppeling uit een CycloneDX-document wordt niet gebouwd. Er is geen library om aan een image-digest te hangen, en een koppeling die toch een node zou schrijven, botst met het uitgezette library-pad.

## Gevolgen

- `depgraph library-impact` antwoordt `libraries disabled`.
- De types `Library` en `uses_library` blijven in het model.
- Een latere fase kan dit heropenen als er wel interne packages zijn.
