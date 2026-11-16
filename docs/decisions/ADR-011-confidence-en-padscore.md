# ADR-011: confidence en padscore blijven naast elkaar

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar).

## Context

De adoptie-ronde zette `confidence` op een edge: een getal uit status en of er calls zijn. Het plan voor F4-2 vraagt een padscore uit call-aandeel maal hardheid, voor rangschikking, niet als kans.

## Besluit

`confidence` blijft. Het zegt hoe stevig de edge zelf is (beide bronnen, alleen gezien, alleen gedeclareerd, kandidaat). De padscore is iets anders: het aandeel van de calls van de client, vermenigvuldigd langs het pad met de hardheidsfactor uit `scoring` in `config.yaml`. Die factoren zijn aannames (hard 1.0, unknown 0.5, soft 0.2). Geen van beide is een kans. Impact, critical en de MCP-tools tonen de padscore of `weight_score`. `confidence` blijft in `show --format json`.

## Gevolgen

- Een heuristische `soft` verlaagt de padscore via de factor 0.2, maar de bron blijft `heuristic`. Dat is bewijs, geen oorzaak.
- Dennis kan de factoren wijzigen zonder de code te veranderen.
