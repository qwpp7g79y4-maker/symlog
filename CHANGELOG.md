# Changelog

Alle noemenswaardige wijzigingen in dit project staan in dit bestand. Het formaat is
gebaseerd op [Keep a Changelog](https://keepachangelog.com/); dit project gebruikt nog
geen SemVer-garanties zolang het niet gepubliceerd is.

## 0.1.1 — 2026-10-25

### Opgelost

- Een los JWT-achtig token met base64-padding (`=`/`==`) direct na het derde segment
  liet de laatste een of twee tekens van het geheim ongemaskeerd staan; de padding
  wordt nu mee geredigeerd.
- Een `Authorization`-, `password`- of `apikey`-waarde na een CR of LF vlak vóór de
  scheidingstekens (`:`/`=`) werd niet geredigeerd; dat gat is gedicht voor zowel
  Bearer- als wachtwoord/apikey-waarden.
- Een laatste regel zonder afsluitend regeleinde werd in een eenmalige (niet
  `--follow`) plain-run stilzwijgend weggelaten; die staartregel verschijnt nu
  alsnog, geredigeerd.
- Een losse NEL, LINE SEPARATOR, PARAGRAPH SEPARATOR of NBSP in het gat vóór een
  quoted waarde kon de redactie van die waarde omzeilen; die tekens worden nu net
  als andere format-tekens overgeslagen.

## 0.1.0 — eerste lokale versie

Eerste werkende versie van `symlog`: een read-only terminal-logviewer voor
Symfony/Monolog-logs.

### Toegevoegd

- Parser voor Monolog `LineFormatter` (Monolog 1–3) en `JsonFormatter` (NDJSON), ook
  gemengd in één bestand, met multiline-stacktraces en JSON-exceptions als
  gestructureerde frames.
- Veilige terugval op "raw"-records voor kapotte regels, ongeldige JSON en ongeldige
  UTF-8, met neutralisatie van ANSI-escapes en NUL-bytes.
- Streaming reader met een offset-index (geen volledige bestandsload in het geheugen)
  en een begrensde recordcache.
- Filteren op kanaal en niveau, met regex-zoeken in de TUI.
- Interactieve TUI (`ratatui`/`crossterm`) met vim-achtige toetsen, een detailpaneel,
  kanaal- en niveaukiezers, en correcte kolombreedte voor brede en samengestelde
  tekens.
- Live volgen (`tail -f`) in plain-uitvoer, met detectie van groei, rename-rotatie en
  afkappen.
- Plain-tekstuitvoer zonder terminal (pipes, redirects, CI), met optionele kleur.
- Geheimen in plain-uitvoer en in de TUI worden standaard `***`. `--no-redact` toont
  de oorspronkelijke tekst. Het logbestand op schijf blijft ongewijzigd.
- Duidelijke exitcodes voor een leeg, ontbrekend of onleesbaar bestand, en voor een
  map in plaats van een bestand.
- Bewijs van read-only gedrag (bestandsmetadata blijft ongewijzigd; geen schrijf- of
  netwerkoproepen) als geautomatiseerde test.

### Bekende beperkingen

Zie de sectie "Open punten" in de README: de editor-sprong, padmapping, tijdzone-
omrekening, een config-bestand en live volgen binnen de interactieve TUI-lus zelf zijn
nog niet aangesloten in deze versie.
