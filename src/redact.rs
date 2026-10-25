//! Maskeert geheimen in tekst die naar de gebruiker gaat.
//!
//! Het logbestand zelf blijft onaangeroerd: dit draait alleen over tekst die plain
//! of de TUI al gaat tonen. `--no-redact` slaat deze stap over.
//!
//! Vervangen door `***`:
//! - de waarde na `password`, `passwd`, `token`, `secret`, `api_key` of `apikey`
//!   (hoofdletterongevoelig, met `=` of `:`, ook als JSON-veld);
//! - het token na `Authorization: Bearer`, ook als JSON-veld
//!   (`"Authorization": "Bearer …"`);
//! - het wachtwoord in URL-userinfo na `://` (`user:wachtwoord@host`); staat in
//!   de gebruikersnaam een hoog-risico token (`ghp_`, `github_pat_`, `xoxb-`,
//!   `sk_live_` / `sk_test_`, `AKIA` of een JWT), ook ná een voorvoegsel
//!   (`x-access-token-ghp_…`, `svc-AKIA…`, `bearer-<jwt>`), dan worden
//!   gebruikersnaam én wachtwoord `***` (`scheme://***:***@host`), ook in de
//!   git-vorm `scheme://<token>:x-oauth-basic@host`. Een gewone naam (`git`,
//!   `user`, `ada`) blijft staan. `scheme://<token>@host` (geen `:wachtwoord`)
//!   valt onder het token zelf;
//! - een sleutel `sk_live_…` of `sk_test_…`;
//! - een AWS-toegangssleutel `AKIA` plus precies 16 hoofdletters of cijfers;
//! - een GitHub-token `ghp_` plus minstens 36 letters of cijfers;
//! - een fijnmazig GitHub-token `github_pat_` plus minstens 82 letters, cijfers of `_`;
//! - een Slack-bottoken `xoxb-` met een lichaam van minstens 10 tekens;
//! - een JWT: drie base64url-delen van minstens 16 tekens, gescheiden door één punt,
//!   waarvan het eerste deel met `eyJ` begint.

use crate::parse::raw::sanitize_for_display;

const KEYS: [&str; 6] = ["password", "api_key", "apikey", "passwd", "secret", "token"];

struct Span {
    /// Einde (exclusief) van het stuk dat blijft staan, vóór `***`.
    keep_end: usize,
    /// Einde (exclusief) van de waarde die door `***` vervangen wordt.
    value_end: usize,
    /// Index waar het scannen verdergaat (na een sluitend aanhalingsteken).
    resume: usize,
}

/// Saniteert `text` voor weergave en maskeert geheimen als `redact` aan staat.
/// Saniteren komt ná het maskeren, zodat een stuurteken midden in een waarde de
/// waarde niet in tweeën knipt.
pub fn for_display(text: &str, redact: bool) -> String {
    let masked = if redact {
        redact_secrets(text)
    } else {
        text.to_string()
    };
    sanitize_for_display(&masked)
}

/// Vervangt herkende geheimen in `input` door `***`. Sleutelnamen blijven staan.
pub fn redact_secrets(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut jwt_run_cache = B64UrlRunCache::new();
    while i < bytes.len() {
        if let Some(span) = match_at(bytes, i, &mut jwt_run_cache) {
            if span_is_usable(i, bytes.len(), &span) {
                // `match_url_userinfo` laat de gebruikersnaam staan en maskeert alleen
                // het wachtwoord. Staat in die naam een hoog-risico token, ook ná een
                // voorvoegsel, dan zou de scan daarna pas bij `@` verdergaan en het
                // token nooit meer zien. De hele naam wordt dan `***`.
                if url_username_is_high_risk_token(bytes, i, &span) {
                    out.push_str("***:");
                } else {
                    out.push_str(&input[i..span.keep_end]);
                }
                out.push_str("***");
                out.push_str(&input[span.value_end..span.resume]);
                i = span.resume;
                continue;
            }
        }
        let Some(ch) = input[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn span_is_usable(start: usize, len: usize, span: &Span) -> bool {
    span.resume > start
        && span.keep_end >= start
        && span.keep_end <= span.value_end
        && span.value_end <= span.resume
        && span.resume <= len
}

fn match_at(bytes: &[u8], i: usize, jwt_run_cache: &mut B64UrlRunCache) -> Option<Span> {
    // Userinfo vóór sleutel-waarde: anders eet `password` in
    // `https://password:geheim@host` de rest van de URL op.
    // Een token als gebruikersnaam wordt in de uitvoer alsnog gemaskeerd
    // (`url_username_is_high_risk_token`); de volgorde hier blijft, zodat
    // `https://password:geheim@host` niet als sleutel-waarde wordt gelezen.
    match_url_userinfo(bytes, i)
        .or_else(|| match_assignment(bytes, i))
        .or_else(|| match_bearer(bytes, i))
        .or_else(|| match_high_risk_token(bytes, i, jwt_run_cache))
}

fn match_high_risk_token(
    bytes: &[u8],
    i: usize,
    jwt_run_cache: &mut B64UrlRunCache,
) -> Option<Span> {
    find_high_risk_token(bytes, i, true, jwt_run_cache)
}

/// `require_boundary` geldt in vrije tekst, zodat een token midden in een woord
/// blijft staan. In een URL-gebruikersnaam is de grens al `://` … `:`, en mag
/// het token ná een voorvoegsel staan.
fn find_high_risk_token(
    bytes: &[u8],
    i: usize,
    require_boundary: bool,
    jwt_run_cache: &mut B64UrlRunCache,
) -> Option<Span> {
    match_stripe(bytes, i)
        .or_else(|| match_akia(bytes, i))
        .or_else(|| match_github_pat(bytes, i, require_boundary))
        .or_else(|| match_ghp(bytes, i, require_boundary))
        .or_else(|| match_xoxb(bytes, i, require_boundary))
        .or_else(|| match_jwt(bytes, i, require_boundary, jwt_run_cache))
}

/// `scheme://<naam>:wachtwoord@host`, waarbij in `<naam>` een hoog-risico token
/// staat. Het token mag vooraan staan of ná een voorvoegsel (`x-access-token-ghp_…`).
/// De userinfo-span houdt die naam en vervangt alleen het wachtwoord.
fn url_username_is_high_risk_token(bytes: &[u8], i: usize, span: &Span) -> bool {
    if i < 3 || bytes[i - 3] != b':' || bytes[i - 2] != b'/' || bytes[i - 1] != b'/' {
        return false;
    }
    if span.keep_end <= i {
        return false;
    }
    let Some(sep_len) = colon_sep_len_before(bytes, i, span.keep_end) else {
        return false;
    };
    let user_end = span.keep_end - sep_len;
    // Eén tabel voor de hele gebruikersnaam (`b64url_run_ends`) i.p.v. een losse
    // `scan_while` per startpositie: anders scant een naam vol herhaalde
    // `eyJ`-vulling bij ~1 op de 3 posities opnieuw tot het einde, O(n^2).
    let run_end = b64url_run_ends(bytes, i, user_end);
    (i..user_end).any(|j| high_risk_token_covers(bytes, i, j, user_end, &run_end))
}

/// `[start, end)` begint met een hoog-risico token dat niet voorbij `end` loopt.
/// De grens vóór `start` telt niet: in een URL-gebruikersnaam mag er een
/// voorvoegsel staan. Korter dan de gebruikersnaam telt mee (`<token>.extra`);
/// een match die doorloopt in het wachtwoord telt niet.
fn high_risk_token_covers(
    bytes: &[u8],
    base: usize,
    start: usize,
    end: usize,
    run_end: &[usize],
) -> bool {
    high_risk_token_at(bytes, base, start, run_end).is_some_and(|span| {
        span.keep_end == start && span.value_end > start && span.value_end <= end
    })
}

/// Zelfde kandidaten als `find_high_risk_token(bytes, start, false)`, maar de
/// JWT-tak leest `run_end` (zie `b64url_run_ends`) in plaats van zelf te scannen.
fn high_risk_token_at(bytes: &[u8], base: usize, start: usize, run_end: &[usize]) -> Option<Span> {
    match_stripe(bytes, start)
        .or_else(|| match_akia(bytes, start))
        .or_else(|| match_github_pat(bytes, start, false))
        .or_else(|| match_ghp(bytes, start, false))
        .or_else(|| match_xoxb(bytes, start, false))
        .or_else(|| match_jwt_with_run_table(bytes, base, start, run_end))
}

/// Gelijk aan `match_jwt(bytes, i, false)`, maar het einde van elk base64url-deel
/// komt uit `run_end` (O(1)) i.p.v. uit een eigen `scan_while` (O(resterende lengte)).
fn match_jwt_with_run_table(
    bytes: &[u8],
    base: usize,
    i: usize,
    run_end: &[usize],
) -> Option<Span> {
    if !bytes[i..].starts_with(b"eyJ") {
        return None;
    }
    let end1 = run_end[i - base];
    if end1 - i < JWT_SEGMENT_MIN || !dot_at(bytes, end1) {
        return None;
    }
    let start2 = end1 + 1;
    let end2 = run_end[start2 - base];
    if end2 - start2 < JWT_SEGMENT_MIN || !dot_at(bytes, end2) {
        return None;
    }
    let start3 = end2 + 1;
    let end3 = run_end[start3 - base];
    if end3 - start3 < JWT_SEGMENT_MIN {
        return None;
    }
    // `run_end` stopt al op het eerste niet-base64url-byte, dus `end3` kan geen
    // base64url-byte zijn; alleen een vierde punt moet de match nog afkeuren.
    if dot_at(bytes, end3) {
        return None;
    }
    Some(Span {
        keep_end: i,
        value_end: end3,
        resume: end3,
    })
}

/// `run_end[k - i]` = eerste positie `>= k` (tot en met `user_end`) die geen
/// base64url-byte is. `user_end` zelf staat altijd op het scheidingsteken `:`
/// (geen base64url-byte), dus die grens is al correct als startwaarde.
fn b64url_run_ends(bytes: &[u8], i: usize, user_end: usize) -> Vec<usize> {
    let mut run_end = vec![user_end; user_end - i + 1];
    for k in (i..user_end).rev() {
        run_end[k - i] = if is_b64url_byte(bytes[k]) {
            run_end[k + 1 - i]
        } else {
            k
        };
    }
    run_end
}

fn match_assignment(bytes: &[u8], i: usize) -> Option<Span> {
    if !at_boundary(bytes, i) {
        return None;
    }
    for key in KEYS {
        let Some(mut j) = match_key_ignore_ascii(bytes, i, key) else {
            continue;
        };
        // U+200B, U+200C of U+FEFF tussen de laatste sleutelletter en het
        // sluitende aanhalingsteken, en een overbodig tweede aanhalingsteken
        // (met of zonder format/witruimte ertussen) vóór `=`/`:`.
        // `skip_key_operator_gap` slaat zo'n teken wel over, maar stopt op
        // `"`/`'`, en dat is geen `=` of `:`.
        j = skip_quotes(bytes, j);
        j = skip_key_operator_gap(bytes, j);
        if j >= bytes.len() || (bytes[j] != b'=' && bytes[j] != b':') {
            continue;
        }
        j += 1;
        // CR, LF en FF zijn ASCII-witruimte. `skip_inline_ws` slaat ze niet over,
        // en `scan_value` ziet ze als scheidingsteken en geeft de match dan op.
        j = skip_ascii_ws(bytes, j);
        if let Some(span) = scan_value(bytes, j) {
            return Some(span);
        }
    }
    None
}

fn match_bearer(bytes: &[u8], i: usize) -> Option<Span> {
    if !at_boundary(bytes, i) {
        return None;
    }
    const WORD: &str = "authorization";
    let mut j = match_key_ignore_ascii(bytes, i, WORD)?;
    // JSON-sleutel: "Authorization" : "Bearer …". Zelfde gat als hierboven:
    // een format-teken of een overbodig tweede aanhalingsteken vóór het
    // sluitende aanhalingsteken van de sleutel.
    j = skip_quotes(bytes, j);
    j = skip_key_operator_gap(bytes, j);
    if j >= bytes.len() || bytes[j] != b':' {
        return None;
    }
    j += 1;
    // CR, LF, FF en VT ná `:` lieten de match stuklopen: skip_quotes kent via
    // skip_ws_and_format alleen spatie en tab. skip_bearer_gap slaat die vier,
    // net als NBSP en de onzichtbare format-tekens, wel over.
    j = skip_bearer_gap(bytes, j);
    // Ná `:` en vóór het openende aanhalingsteken van een gequote
    // Bearer-waarde kunnen witruimte, U+200B/U+200C/U+FEFF en een overbodig
    // herhaald aanhalingsteken afgewisseld voorkomen (bv. format, spatie,
    // format, quote, quote). `skip_quotes` blijft al die soorten afwisselen
    // tot er niets meer overblijft om over te slaan.
    j = skip_quotes(bytes, j);
    const BEARER: &str = "bearer";
    let j = match_key_ignore_ascii(bytes, j, BEARER)?;
    // Spatie en tab werkten al. CR, VT, FF, NBSP en U+200B/U+200C/U+FEFF
    // tussen `Bearer` en het token lieten de match stuklopen.
    let value_start = skip_bearer_gap(bytes, j);
    if value_start == j || value_start >= bytes.len() || is_bearer_value_end(bytes[value_start]) {
        return None;
    }
    let mut value_end = value_start;
    while value_end < bytes.len() && !is_bearer_value_end(bytes[value_end]) {
        value_end += 1;
    }
    if value_end == value_start {
        return None;
    }
    Some(Span {
        keep_end: value_start,
        value_end,
        resume: value_end,
    })
}

fn is_bearer_value_end(b: u8) -> bool {
    b.is_ascii_whitespace() || b == b'"' || b == b'\''
}

/// `https://user:wachtwoord@host` → het wachtwoord wordt `***`, user en host blijven.
/// Alleen na `://`, zodat `fout:tekst@ergens` geen URL is. Een lege gebruikersnaam,
/// een leeg wachtwoord of een ontbrekende `@host` matcht niet. Is `user` een
/// hoog-risico token, dan maskeert de aanroeper ook die naam (zie
/// `url_username_is_high_risk_token`).
fn match_url_userinfo(bytes: &[u8], i: usize) -> Option<Span> {
    if i < 3 || bytes[i - 3] != b':' || bytes[i - 2] != b'/' || bytes[i - 1] != b'/' {
        return None;
    }
    let pass_start = userinfo_colon(bytes, i)?;
    let (k, sep_len) = userinfo_at(bytes, pass_start)?;
    let host = k + sep_len;
    if host >= bytes.len() || !is_url_host_start(bytes[host]) {
        return None;
    }
    Some(Span {
        keep_end: pass_start,
        value_end: k,
        resume: k,
    })
}

/// Grens tussen gebruikersnaam en wachtwoord. Een rauwe `:` gaat voor; alleen
/// als die nergens in de genaam voorkomt (bevinding 2: `%3A` is dan de enige
/// scheiding die een aanvaller heeft gebruikt) telt een percent-encoded `%3A`.
/// Dat voorkomt dat een `%3A`/`%40` die toevallig in een gewoon wachtwoord zit
/// vóór de echte, rauwe scheider verandert waar de naam of het wachtwoord stopt.
fn userinfo_colon(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    while j < bytes.len() && is_url_user_byte(bytes[j]) {
        j += 1;
    }
    if j == i {
        return None;
    }
    if j < bytes.len() && bytes[j] == b':' {
        return Some(j + 1);
    }
    let pos = find_pct_escape(bytes, i, j, b':')?;
    Some(pos + 3)
}

/// Lengte van de scheider die eindigt op `end` (exclusief dat punt): `1` voor
/// een rauwe `:`, `3` voor `%3A` (hoofdletterongevoelig). Spiegelt de keuze die
/// `userinfo_colon` aan het begin van de scheider maakt, zodat
/// `url_username_is_high_risk_token` ook de percent-vorm als scheider herkent.
/// Geen van beide op die positie geeft `None`.
fn colon_sep_len_before(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    if end > start && bytes[end - 1] == b':' {
        return Some(1);
    }
    if end >= start + 3 {
        let (b, len) = decode_pct_byte(bytes, end - 3);
        if len == 3 && b == b':' {
            return Some(3);
        }
    }
    None
}

/// Zelfde idee voor de grens tussen wachtwoord en host (`@` of `%40`).
fn userinfo_at(bytes: &[u8], pass_start: usize) -> Option<(usize, usize)> {
    // CR en tab direct na de `:` lieten `is_url_pass_byte` meteen stoppen
    // (het zijn ASCII-witruimte), waardoor de leeg-check hieronder faalde en
    // het wachtwoord erna ongemaskeerd bleef. Sla ze over vóór die check.
    let mut k = pass_start;
    while k < bytes.len() && matches!(bytes[k], b'\r' | b'\t') {
        k += 1;
    }
    let value_start = k;
    while k < bytes.len() && is_url_pass_byte(bytes[k]) {
        k += 1;
    }
    if k == value_start {
        return None;
    }
    if k < bytes.len() && bytes[k] == b'@' {
        return Some((k, 1));
    }
    let pos = find_pct_escape(bytes, pass_start, k, b'@')?;
    Some((pos, 3))
}

/// Eerste `%XY` in `bytes[start..end]` die decodeert naar `want` (hex-cijfers
/// hoofdletterongevoelig). Gebruikt als terugval in `userinfo_colon`/`userinfo_at`
/// wanneer de rauwe scheider in dat bereik ontbreekt.
fn find_pct_escape(bytes: &[u8], start: usize, end: usize, want: u8) -> Option<usize> {
    let mut j = start;
    while j < end {
        let (b, len) = decode_pct_byte(bytes, j);
        if len == 3 && b == want {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// Decodeert `%XY` (hex, hoofdletterongevoelig) op positie `j` naar de bedoelde
/// byte. Geen geldige escape op `j`, dan het rauwe byte zelf met lengte 1.
fn decode_pct_byte(bytes: &[u8], j: usize) -> (u8, usize) {
    let b0 = bytes[j];
    if b0 == b'%' {
        if let (Some(&h), Some(&l)) = (bytes.get(j + 1), bytes.get(j + 2)) {
            if let (Some(hv), Some(lv)) = (hex_val(h), hex_val(l)) {
                return (hv * 16 + lv, 3);
            }
        }
    }
    (b0, 1)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn is_url_user_byte(b: u8) -> bool {
    !matches!(
        b,
        b':' | b'@' | b'/' | b'?' | b'#' | b'"' | b'\'' | b'<' | b'>'
    ) && !b.is_ascii_whitespace()
}

fn is_url_pass_byte(b: u8) -> bool {
    !matches!(b, b'@' | b'/' | b'?' | b'#' | b'"' | b'\'' | b'<' | b'>') && !b.is_ascii_whitespace()
}

fn is_url_host_start(b: u8) -> bool {
    !b.is_ascii_whitespace() && !matches!(b, b'"' | b'\'' | b'<' | b'>' | b'@')
}

/// GitHub classic PAT: `ghp_` plus 36 `[A-Za-z0-9]`. Langer wordt helemaal gemaskeerd.
const GHP_BODY_MIN: usize = 36;

fn match_ghp(bytes: &[u8], i: usize, require_boundary: bool) -> Option<Span> {
    if require_boundary && !at_boundary(bytes, i) {
        return None;
    }
    const PREFIX: &[u8] = b"ghp_";
    // Percent-decoded, zodat `%67hp_…` (of `%xx` in het lichaam) niet langs de
    // letterlijke bytevergelijking glipt (bevinding 2, een URL-gebruikersnaam
    // vol percent-encoding). Zonder `%` in de invoer is dit gelijk aan de
    // oude rauwe `starts_with`/`scan_while`.
    let body = pct_match_prefix(bytes, i, PREFIX)?;
    let (end, count) = pct_scan_while(bytes, body, |b| b.is_ascii_alphanumeric());
    if count < GHP_BODY_MIN {
        return None;
    }
    Some(Span {
        keep_end: i,
        value_end: end,
        resume: end,
    })
}

/// Vergelijkt `expected` met de percent-gedecodeerde bytes vanaf `i`. Geeft de
/// rauwe eindpositie terug zodra alle bytes van `expected` gematcht zijn.
fn pct_match_prefix(bytes: &[u8], i: usize, expected: &[u8]) -> Option<usize> {
    let mut j = i;
    for &want in expected {
        if j >= bytes.len() {
            return None;
        }
        let (b, len) = decode_pct_byte(bytes, j);
        if b != want {
            return None;
        }
        j += len;
    }
    Some(j)
}

/// Zoals `scan_while`, maar telt gedecodeerde tekens i.p.v. rauwe bytes: een
/// `%41` in het lichaam telt als één alfanumeriek teken. Geeft de rauwe
/// eindpositie en het aantal gedecodeerde tekens terug.
fn pct_scan_while(bytes: &[u8], mut j: usize, pred: impl Fn(u8) -> bool) -> (usize, usize) {
    let mut count = 0;
    while j < bytes.len() {
        let (b, len) = decode_pct_byte(bytes, j);
        if !pred(b) {
            break;
        }
        j += len;
        count += 1;
    }
    (j, count)
}

/// Fijnmazig GitHub-token. Het lichaam is `[A-Za-z0-9_]`, minstens 82 tekens:
/// de lengte die GitHub voor `github_pat_` publiceert. Korter blijft staan.
const GITHUB_PAT_BODY_MIN: usize = 82;

fn match_github_pat(bytes: &[u8], i: usize, require_boundary: bool) -> Option<Span> {
    if require_boundary && !at_boundary(bytes, i) {
        return None;
    }
    const PREFIX: &[u8] = b"github_pat_";
    if !bytes[i..].starts_with(PREFIX) {
        return None;
    }
    let body = i + PREFIX.len();
    let end = scan_while(bytes, body, |b| b.is_ascii_alphanumeric() || b == b'_');
    if end - body < GITHUB_PAT_BODY_MIN {
        return None;
    }
    Some(Span {
        keep_end: i,
        value_end: end,
        resume: end,
    })
}

/// Slack-bottoken. Het lichaam is `[A-Za-z0-9-]`, minstens 10 tekens, zodat `xoxb-`
/// in een zin blijft staan.
const XOXB_BODY_MIN: usize = 10;

fn match_xoxb(bytes: &[u8], i: usize, require_boundary: bool) -> Option<Span> {
    if require_boundary && !at_boundary(bytes, i) {
        return None;
    }
    const PREFIX: &[u8] = b"xoxb-";
    if !bytes[i..].starts_with(PREFIX) {
        return None;
    }
    let body = i + PREFIX.len();
    let end = scan_while(bytes, body, |b| b.is_ascii_alphanumeric() || b == b'-');
    if end - body < XOXB_BODY_MIN {
        return None;
    }
    Some(Span {
        keep_end: i,
        value_end: end,
        resume: end,
    })
}

/// Drie base64url-delen. `eyJ` is base64url van `{"` en hoort bij elke JWT-header.
/// Korter dan 16 tekens per deel (versies, IP-adressen) blijft staan. Een vierde
/// punt hoort niet bij een JWT.
const JWT_SEGMENT_MIN: usize = 16;

/// Onthoudt het einde van de laatst gescande base64url-runs (zoals
/// `scan_while` met `is_b64url_byte` die zou opleveren), zodat een latere
/// kandidaat die binnen een van die runs begint niet opnieuw tot het eind
/// van de run scant. Zonder dit geheugen scant `match_jwt` op een vrije
/// regel vol herhaald `-eyJ` bij elke `-` opnieuw tot het regeleinde (of tot
/// het eerste punt): `-` is zelf een base64url-byte. Dezelfde aanpak als
/// `b64url_run_ends`, maar lui gevuld i.p.v. in één tabel vooraf, omdat hier
/// (anders dan bij een URL-gebruikersnaam) geen vaste einde-grens vooraf
/// bekend is.
///
/// `match_jwt` doet per kandidaat tot drie `end_from`-aanroepen (één per
/// JWT-segment). Met maar één onthouden bereik overschreef de tweede
/// aanroep (ná het eerste punt) het bereik van de eerste: zodra de run een
/// punt bevat, verloor elke volgende `eyJ`-start in diezelfde run zijn
/// cache-hit en scande opnieuw tot dat punt -> O(n²) voor de hele run. Drie
/// sleuven (één per segment, round-robin overschreven) houden het bereik van
/// elk segment vast, want elke herhaalde `eyJ`-start in dezelfde run vraagt
/// voor elk segment telkens hetzelfde `[start, end)`-paar op.
const B64URL_RUN_CACHE_SLOTS: usize = 3;

struct B64UrlRunCache {
    slots: [(usize, usize); B64URL_RUN_CACHE_SLOTS],
    next: usize,
}

impl B64UrlRunCache {
    fn new() -> Self {
        Self {
            slots: [(0, 0); B64URL_RUN_CACHE_SLOTS],
            next: 0,
        }
    }

    /// Eerste positie `>= i` die geen base64url-byte is. Valt `i` binnen een
    /// van de onthouden runs, dan is dat antwoord al bekend (de hele run
    /// tussen `start` en `end` is toen al als base64url bevestigd, dus elk
    /// deel daarvan geldt nog); anders scant dit opnieuw vanaf `i` en
    /// onthoudt het resultaat in de oudste sleuf.
    fn end_from(&mut self, bytes: &[u8], i: usize) -> usize {
        for &(start, end) in &self.slots {
            if start <= i && i < end {
                return end;
            }
        }
        let end = scan_while(bytes, i, is_b64url_byte);
        // Een lege range (`i` zelf al geen base64url-byte, bijvoorbeeld meteen ná
        // een punt aan het einde van de tekst) kan nooit opnieuw een hit geven
        // (`start <= i && i < end` eist `end > start`). Zo'n range toch opslaan
        // kost niets aan scan-tijd (er is niets gescand), maar verbruikt wél een
        // sleuf in de round-robin en zou daarmee op een gegeven moment de sleuf
        // met de lange, wél herbruikbare run overschrijven.
        if end > i {
            self.slots[self.next] = (i, end);
            self.next = (self.next + 1) % self.slots.len();
        }
        end
    }
}

fn match_jwt(
    bytes: &[u8],
    i: usize,
    require_boundary: bool,
    jwt_run_cache: &mut B64UrlRunCache,
) -> Option<Span> {
    if require_boundary && !at_jwt_boundary(bytes, i) {
        return None;
    }
    if !bytes[i..].starts_with(b"eyJ") {
        return None;
    }
    let end1 = jwt_run_cache.end_from(bytes, i);
    if end1 - i < JWT_SEGMENT_MIN || !dot_at(bytes, end1) {
        return None;
    }
    let start2 = end1 + 1;
    let end2 = jwt_run_cache.end_from(bytes, start2);
    if end2 - start2 < JWT_SEGMENT_MIN || !dot_at(bytes, end2) {
        return None;
    }
    let start3 = end2 + 1;
    let end3 = jwt_run_cache.end_from(bytes, start3);
    if end3 - start3 < JWT_SEGMENT_MIN {
        return None;
    }
    if end3 < bytes.len() && (bytes[end3] == b'.' || is_b64url_byte(bytes[end3])) {
        return None;
    }
    Some(Span {
        keep_end: i,
        value_end: end3,
        resume: end3,
    })
}

fn at_jwt_boundary(bytes: &[u8], i: usize) -> bool {
    i == 0 || {
        let prev = bytes[i - 1];
        // `-` hoort bij base64url, maar is ook het koppelteken in `bearer-<jwt>`.
        // Een afgekapte URL heeft geen userinfo-span, dus die grens moet hier al
        // een token-begin zijn. Een punt blijft een scheiding binnen het JWT.
        prev == b'-' || (prev != b'.' && !is_b64url_byte(prev))
    }
}

fn dot_at(bytes: &[u8], i: usize) -> bool {
    i < bytes.len() && bytes[i] == b'.'
}

fn is_b64url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn scan_while(bytes: &[u8], mut j: usize, pred: impl Fn(u8) -> bool) -> usize {
    while j < bytes.len() && pred(bytes[j]) {
        j += 1;
    }
    j
}

fn match_stripe(bytes: &[u8], i: usize) -> Option<Span> {
    for prefix in ["sk_live_", "sk_test_"] {
        let prefix_bytes = prefix.as_bytes();
        if !bytes[i..].starts_with(prefix_bytes) {
            continue;
        }
        let mut j = i + prefix_bytes.len();
        let alnum_start = j;
        while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
            j += 1;
        }
        if j > alnum_start {
            return Some(Span {
                keep_end: i,
                value_end: j,
                resume: j,
            });
        }
    }
    None
}

fn match_akia(bytes: &[u8], i: usize) -> Option<Span> {
    const PREFIX: &[u8] = b"AKIA";
    if !bytes[i..].starts_with(PREFIX) {
        return None;
    }
    let start = i + PREFIX.len();
    let end = start + 16;
    if end > bytes.len() {
        return None;
    }
    if !bytes[start..end]
        .iter()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return None;
    }
    if end < bytes.len() && (bytes[end].is_ascii_uppercase() || bytes[end].is_ascii_digit()) {
        return None;
    }
    Some(Span {
        keep_end: i,
        value_end: end,
        resume: end,
    })
}

/// Waarde na `=` of `:`. `keep_end` ligt op de grens tussen scheidingsteken en waarde,
/// zodat aanhalingstekens blijven staan en alleen de inhoud `***` wordt.
fn scan_value(bytes: &[u8], i: usize) -> Option<Span> {
    if i >= bytes.len() {
        return None;
    }
    match bytes[i] {
        b'"' | b'\'' => scan_quoted(bytes, i),
        b'{' | b'[' => scan_balanced(bytes, i),
        b if !is_unquoted_delimiter(b) => scan_unquoted(bytes, i),
        _ => None,
    }
}

fn scan_quoted(bytes: &[u8], i: usize) -> Option<Span> {
    let quote = bytes[i];
    let mut j = i + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' && j + 1 < bytes.len() {
            j += 2;
            continue;
        }
        if bytes[j] == quote {
            return Some(Span {
                keep_end: i + 1,
                value_end: j,
                resume: j + 1,
            });
        }
        j += 1;
    }
    if j > i + 1 {
        return Some(Span {
            keep_end: i + 1,
            value_end: j,
            resume: j,
        });
    }
    None
}

fn scan_unquoted(bytes: &[u8], i: usize) -> Option<Span> {
    let mut j = i;
    while j < bytes.len() && !is_unquoted_delimiter(bytes[j]) {
        j += 1;
    }
    if j == i {
        return None;
    }
    Some(Span {
        keep_end: i,
        value_end: j,
        resume: j,
    })
}

fn scan_balanced(bytes: &[u8], i: usize) -> Option<Span> {
    let open = bytes[i];
    let close = match open {
        b'{' => b'}',
        b'[' => b']',
        _ => return None,
    };
    let mut depth = 0usize;
    let mut j = i;
    while j < bytes.len() {
        let b = bytes[j];
        if b == b'"' || b == b'\'' {
            let quote = b;
            let mut k = j + 1;
            let mut closed = false;
            while k < bytes.len() {
                if bytes[k] == b'\\' && k + 1 < bytes.len() {
                    k += 2;
                    continue;
                }
                if bytes[k] == quote {
                    closed = true;
                    k += 1;
                    break;
                }
                k += 1;
            }
            if !closed {
                return None;
            }
            j = k;
            continue;
        }
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(Span {
                    keep_end: i,
                    value_end: j + 1,
                    resume: j + 1,
                });
            }
        }
        j += 1;
    }
    None
}

fn is_unquoted_delimiter(b: u8) -> bool {
    b.is_ascii_whitespace()
        || matches!(
            b,
            b',' | b';' | b'"' | b'\'' | b'{' | b'}' | b'[' | b']' | b'<' | b'>' | b'&' | b'|'
        )
}

fn at_boundary(bytes: &[u8], i: usize) -> bool {
    i == 0 || !is_ident(bytes[i - 1])
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Matcht `key` (ASCII, hoofdletterongevoelig) vanaf `i`. U+200B, U+200C en
/// U+FEFF mogen tussen de sleuteltekens staan. Die zijn geen witruimte en geen
/// stuurteken, dus een vaste bytevergelijking ziet `pass\u{200b}word` anders niet.
/// Een format-teken vóór de eerste letter telt niet mee.
fn match_key_ignore_ascii(bytes: &[u8], i: usize, key: &str) -> Option<usize> {
    let mut j = i;
    for (n, expected) in key.bytes().enumerate() {
        if n > 0 {
            while let Some((ch, len)) = decode_char(bytes, j) {
                if !is_invisible_format(ch) {
                    break;
                }
                j += len;
            }
        }
        if j >= bytes.len() || !bytes[j].eq_ignore_ascii_case(&expected) {
            return None;
        }
        j += 1;
    }
    Some(j)
}

fn is_invisible_format(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}' | '\u{200C}' | '\u{FEFF}' | '\u{00AD}' | '\u{2060}' | '\u{180E}'
    )
}

/// Aaneengesloten U+200B, U+200C en U+FEFF. Stopt op het eerste andere teken,
/// zodat een latere startpositie die staart niet opnieuw tot het einde scant.
fn skip_invisible_format(bytes: &[u8], mut j: usize) -> usize {
    while let Some((ch, len)) = decode_char(bytes, j) {
        if !is_invisible_format(ch) {
            break;
        }
        j += len;
    }
    j
}

fn skip_inline_ws(bytes: &[u8], mut j: usize) -> usize {
    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    j
}

/// Afwisselend `skip_inline_ws` en `skip_invisible_format`, tot geen van
/// beide nog iets overslaat. Zo blijft elke gemengde reeks van spatie/tab en
/// U+200B/U+200C/U+FEFF overgeslagen, ook als de twee soorten elkaar meer dan
/// één keer afwisselen (format, spatie, format, …), terwijl elke stap zelf
/// nog steeds één keer lineair over de bytes loopt.
fn skip_ws_and_format(bytes: &[u8], mut j: usize) -> usize {
    loop {
        let before = j;
        j = skip_inline_ws(bytes, j);
        j = skip_invisible_format(bytes, j);
        if j == before {
            return j;
        }
    }
}

/// Spatie, tab, CR, LF, FF, VT, en U+200B, U+200C, U+FEFF. Alleen ná `=`/`:`,
/// vóór de waarde. Een format-teken is geen ASCII-witruimte; zonder deze tak
/// maskeert `scan_unquoted` alleen dat teken en blijft de gequote waarde staan.
/// VT (`\u{000B}`) is evenmin ASCII-witruimte volgens `is_ascii_whitespace`
/// (zie `skip_bearer_gap`, die dat gat al dicht ná `Bearer`): zonder deze tak
/// is VT vóór een gequote waarde geen `is_unquoted_delimiter`-byte, waardoor
/// `scan_unquoted` stopt op het openende aanhalingsteken en alleen VT zelf
/// maskeert terwijl de rest van de gequote waarde ongemaskeerd blijft staan.
/// Stopt op het eerste andere teken.
fn skip_ascii_ws(bytes: &[u8], mut j: usize) -> usize {
    while j < bytes.len() {
        if bytes[j].is_ascii_whitespace() || bytes[j] == b'\x0B' {
            j += 1;
            continue;
        }
        match decode_char(bytes, j) {
            Some((ch, len)) if is_invisible_format(ch) => j += len,
            _ => break,
        }
    }
    j
}

/// Scheiding tussen `Bearer` en het token. Naast spatie en tab ook CR, LF, FF,
/// VT, NBSP en de onzichtbare format-tekens U+200B, U+200C, U+FEFF, U+00AD,
/// U+2060 en U+180E.
fn skip_bearer_gap(bytes: &[u8], mut j: usize) -> usize {
    while let Some((ch, len)) = decode_char(bytes, j) {
        if ch.is_ascii_whitespace()
            || ch == '\u{000B}'
            || ch == '\u{00A0}'
            || is_invisible_format(ch)
        {
            j += len;
        } else {
            break;
        }
    }
    j
}

/// Zoals `skip_inline_ws`, maar ook een los stuurteken (NUL, `\v`, `\f`, een
/// non-breaking space, …) of een onzichtbaar format-teken (U+200B, U+200C,
/// U+FEFF) tussen de sleutelnaam en `=`/`:` wordt overgeslagen. Zonder dit geeft
/// `match_assignment`/`match_bearer` de match stil op zodra zo'n byte in de weg
/// staat, en blijft de waarde erna ongemaskeerd. `\n`/`\r` tellen niet mee: die
/// horen bij regelstructuur, niet bij een verdwaald stuurteken.
fn skip_key_operator_gap(bytes: &[u8], mut j: usize) -> usize {
    while let Some((ch, len)) = decode_char(bytes, j) {
        if matches!(ch, '\n' | '\r') {
            break;
        }
        if ch == ' '
            || ch == '\t'
            || ch.is_whitespace()
            || ch.is_control()
            || is_invisible_format(ch)
        {
            j += len;
        } else {
            break;
        }
    }
    j
}

/// Decodeert het UTF-8-teken dat op byte-positie `j` begint. Gaat ervan uit dat
/// `j` al op een tekengrens staat (zo wordt deze altijd aangeroepen), dus de
/// validatie raakt nooit meer dan de paar bytes van dat ene teken.
fn decode_char(bytes: &[u8], j: usize) -> Option<(char, usize)> {
    let &b0 = bytes.get(j)?;
    let len = match b0 {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => return None,
    };
    let chunk = bytes.get(j..j + len)?;
    let ch = std::str::from_utf8(chunk).ok()?.chars().next()?;
    Some((ch, len))
}

fn skip_one_quote(bytes: &[u8], j: usize) -> usize {
    if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
        j + 1
    } else {
        j
    }
}

/// Eén of meer herhaalde aanhalingstekens (`"`/`'`), met format-tekens en
/// witruimte ertussen toegestaan. `skip_one_quote` verwacht precies één
/// quote; een overbodig tweede aanhalingsteken (bv. `""sleutel""`) laat die
/// ene stap het échte sluitende teken missen, waarna `match_assignment`/
/// `match_bearer` de hele match opgeven en het geheim ongemaskeerd blijft.
/// Blijft daarom, net als `skip_ws_and_format`, afwisselen tot geen van
/// beide nog iets overslaat: nog steeds één lineaire doorloop over de
/// overgeslagen tekens, alleen in meer stapjes.
fn skip_quotes(bytes: &[u8], mut j: usize) -> usize {
    loop {
        let before = j;
        j = skip_ws_and_format(bytes, j);
        j = skip_one_quote(bytes, j);
        if j == before {
            return j;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canary_line_hides_password_and_stripe_key() {
        let input = "password=hunter2 leaked token=sk_live_abcdef123456";
        let out = redact_secrets(input);
        assert_eq!(out, "password=*** leaked token=***");
        assert!(!out.contains("hunter2"));
        assert!(!out.contains("sk_live_abcdef123456"));
    }

    #[test]
    fn json_fields_keep_the_key_and_mask_the_value() {
        let input = r#"{"password":"hunter2","api_key":"abcd","user":"ada"}"#;
        assert_eq!(
            redact_secrets(input),
            r#"{"password":"***","api_key":"***","user":"ada"}"#
        );
        let spaced = r#""token" : "sk_test_zzzz9999""#;
        assert_eq!(redact_secrets(spaced), r#""token" : "***""#);
    }

    #[test]
    fn json_string_escapes_do_not_end_the_value_early() {
        let input = r#"{"password":"hun\"ter2","user":"ada"}"#;
        assert_eq!(redact_secrets(input), r#"{"password":"***","user":"ada"}"#);
    }

    #[test]
    fn bearer_and_akia_are_masked() {
        assert_eq!(
            redact_secrets("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig"),
            "Authorization: Bearer ***"
        );
        assert_eq!(
            redact_secrets("aws AKIAIOSFODNN7EXAMPLE done"),
            "aws *** done"
        );
    }

    #[test]
    fn key_names_are_case_insensitive_and_accept_a_colon() {
        assert_eq!(redact_secrets("PASSWORD=Hunter2"), "PASSWORD=***");
        assert_eq!(redact_secrets("API_KEY=UpperKey"), "API_KEY=***");
        assert_eq!(redact_secrets("APIKEY=apk"), "APIKEY=***");
        assert_eq!(redact_secrets("PASSWD=pwtoken"), "PASSWD=***");
        assert_eq!(redact_secrets("SECRET: secval"), "SECRET: ***");
        assert_eq!(
            redact_secrets("AUTHORIZATION: BEARER S3cretToken"),
            "AUTHORIZATION: BEARER ***"
        );
    }

    #[test]
    fn ordinary_log_text_is_unchanged() {
        let input = "[2026-01-15T10:30:45.123456+01:00] app.DEBUG: User cache warmed";
        assert_eq!(redact_secrets(input), input);
    }

    #[test]
    fn lowercase_akia_lookalike_is_not_a_key() {
        let input = "AKIAiosfodnn7example";
        assert_eq!(redact_secrets(input), input);
    }

    #[test]
    fn a_key_name_that_is_only_a_suffix_of_a_longer_word_is_not_masked() {
        // `at_boundary` moet de vorige byte bekijken: "my" + "secret" is geen los
        // sleutelwoord, dus mag niet maskeren alsof het `secret=` was.
        assert_eq!(redact_secrets("mysecret=val"), "mysecret=val");
        assert_eq!(redact_secrets("subtoken: val"), "subtoken: val");
        assert_eq!(redact_secrets("xpasswd=hunter2"), "xpasswd=hunter2");
    }

    #[test]
    fn an_assignment_without_a_value_is_left_unchanged() {
        assert_eq!(redact_secrets("token="), "token=");
        assert_eq!(redact_secrets("token=,"), "token=,");
        assert_eq!(redact_secrets("token=\n"), "token=\n");
    }

    #[test]
    fn an_unterminated_quoted_value_is_masked_to_the_end_of_the_string() {
        let input = r#"password="still open"#;
        assert_eq!(redact_secrets(input), r#"password="***"#);
        assert!(!redact_secrets(input).contains("still open"));
    }

    #[test]
    fn a_jwt_directly_preceded_by_a_dot_is_not_masked_in_plain_text() {
        // `at_jwt_boundary` wijst een `.` vlak vóór `eyJ` af: dat hoort bij een
        // grotere dotted-tekst (bv. een versienummer), niet bij een los token.
        let input = format!("v1.2.{JWT}");
        assert_eq!(redact_secrets(&input), input);
        assert_eq!(for_display(&input, true), input);
    }

    #[test]
    fn for_display_can_leave_the_original_value() {
        let input = "password=hunter2";
        assert_eq!(for_display(input, true), "password=***");
        assert_eq!(for_display(input, false), input);
    }

    #[test]
    fn json_authorization_bearer_field_masks_only_the_token() {
        let compact = r#"{"Authorization":"Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig"}"#;
        assert_eq!(redact_secrets(compact), r#"{"Authorization":"Bearer ***"}"#);
        let spaced = r#""Authorization" : "Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig""#;
        assert_eq!(redact_secrets(spaced), r#""Authorization" : "Bearer ***""#);
        assert_eq!(
            redact_secrets(r#"{"authorization":"bearer json-field-token"}"#),
            r#"{"authorization":"bearer ***"}"#
        );
        assert_eq!(for_display(compact, false), compact);
        assert!(!redact_secrets(compact).contains("payload.sig"));
    }

    #[test]
    fn url_userinfo_masks_the_password_only() {
        assert_eq!(
            redact_secrets("connect https://ada:s3cret-pass@db.internal:5432/app"),
            "connect https://ada:***@db.internal:5432/app"
        );
        assert_eq!(
            redact_secrets("https://ada:pa:ss@[2001:db8::1]/x"),
            "https://ada:***@[2001:db8::1]/x"
        );
        assert_eq!(
            redact_secrets("https://password:geheim@host/a"),
            "https://password:***@host/a"
        );
        let plain = "see https://example.com/health and http://example.com:8080/a 10.0.0.5";
        assert_eq!(redact_secrets(plain), plain);
        assert_eq!(for_display(plain, false), plain);
    }

    #[test]
    fn ghp_xoxb_and_jwt_are_masked() {
        let ghp = "ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        let xoxb = "xoxb-1234567890-123456789012-AbCdEfGhIjKlMnOpQrStUvWx";
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let input =
            format!("gh {ghp} slack {xoxb} jwt {jwt} ip 10.0.0.5 ver 1.2.3 ghp_short xoxb-");
        let out = redact_secrets(&input);
        assert_eq!(
            out,
            "gh *** slack *** jwt *** ip 10.0.0.5 ver 1.2.3 ghp_short xoxb-"
        );
        assert!(!out.contains(ghp));
        assert!(!out.contains(xoxb));
        assert!(!out.contains(jwt));
        assert_eq!(for_display(&input, false), input);
    }

    const GHP: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    /// `github_pat_` plus 82 `[A-Za-z0-9_]`, waarvan het laatste teken `_` is.
    const GITHUB_PAT: &str = "github_pat_11AAAAAAA0abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz012345678_";
    const XOXB: &str = "xoxb-1234567890-123456789012-AbCdEfGhIjKlMnOpQrStUvWx";
    const SK_LIVE: &str = "sk_live_abcdef123456789";
    const SK_TEST: &str = "sk_test_zzzz9999abcd";
    const AKIA_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

    fn assert_token_username_masked(token: &str, host: &str) {
        let with_password = format!("fetch https://{token}:x@{host}/p");
        let masked_password = redact_secrets(&with_password);
        assert_eq!(masked_password, format!("fetch https://***:***@{host}/p"));
        assert!(!masked_password.contains(token), "{masked_password}");
        assert_eq!(for_display(&with_password, false), with_password);

        let bare = format!("fetch https://{token}@{host}/p");
        let masked_bare = redact_secrets(&bare);
        assert_eq!(masked_bare, format!("fetch https://***@{host}/p"));
        assert!(!masked_bare.contains(token), "{masked_bare}");
        assert_eq!(for_display(&bare, false), bare);
    }

    #[test]
    fn ghp_username_in_url_is_masked() {
        assert_token_username_masked(GHP, "github.com");
        assert_eq!(
            redact_secrets(&format!("http://{GHP}:x@github.com/org/repo.git")),
            "http://***:***@github.com/org/repo.git"
        );
        // Te kort voor een PAT: de naam blijft, het wachtwoord niet.
        assert_eq!(
            redact_secrets("https://ghp_short:s3cret@github.com/org/repo.git"),
            "https://ghp_short:***@github.com/org/repo.git"
        );
        // Token plus een vastgeplakt achtervoegsel is nog steeds die token.
        assert_eq!(
            redact_secrets(&format!("https://{GHP}.note:x@github.com/a")),
            "https://***:***@github.com/a"
        );
    }

    #[test]
    fn xoxb_username_in_url_is_masked() {
        assert_token_username_masked(XOXB, "slack.example");
    }

    #[test]
    fn stripe_username_in_url_is_masked() {
        assert_token_username_masked(SK_LIVE, "api.stripe.com");
        assert_token_username_masked(SK_TEST, "api.stripe.com");
    }

    #[test]
    fn akia_username_in_url_is_masked() {
        assert_token_username_masked(AKIA_KEY, "s3.amazonaws.com");
    }

    #[test]
    fn jwt_username_in_url_is_masked() {
        assert_token_username_masked(JWT, "auth.example");
    }

    #[test]
    fn git_pat_over_https_masks_the_token_username() {
        // De vorm die git zelf in HTTPS-fouten zet: PAT als gebruikersnaam,
        // `x-oauth-basic` als vast wachtwoord.
        let input = format!("git fetch https://{GHP}:x-oauth-basic@github.com/org/repo.git failed");
        assert_eq!(
            redact_secrets(&input),
            "git fetch https://***:***@github.com/org/repo.git failed"
        );
        assert!(!redact_secrets(&input).contains(GHP));
        assert_eq!(for_display(&input, false), input);

        let bare = format!("git fetch https://{GHP}@github.com/org/repo.git failed");
        assert_eq!(
            redact_secrets(&bare),
            "git fetch https://***@github.com/org/repo.git failed"
        );

        // Zelfde PAT, maar als wachtwoord (`x-access-token:<pat>@host`).
        let as_password = format!("https://x-access-token:{GHP}@github.com/org/repo.git");
        assert_eq!(
            redact_secrets(&as_password),
            "https://x-access-token:***@github.com/org/repo.git"
        );
        assert!(!redact_secrets(&as_password).contains(GHP));
    }

    #[test]
    fn pct_encoded_colon_as_the_only_userinfo_separator_is_masked() {
        // breaktest-symlog-0003 bevinding 2: `%3A` i.p.v. een rauwe `:` liet het
        // wachtwoord staan omdat `is_url_user_byte` `%` toestaat en de naam dan
        // doorloopt tot de rauwe `@`.
        assert_eq!(
            redact_secrets("https://user%3As3cret-pass@host.example/a"),
            "https://user%3A***@host.example/a"
        );
        assert!(
            !redact_secrets("https://user%3As3cret-pass@host.example/a").contains("s3cret-pass")
        );
    }

    #[test]
    fn pct_encoded_at_as_the_only_userinfo_separator_is_masked() {
        assert_eq!(
            redact_secrets("https://user:s3cret-pass%40host.example/a"),
            "https://user:***%40host.example/a"
        );
        assert!(
            !redact_secrets("https://user:s3cret-pass%40host.example/a").contains("s3cret-pass")
        );
    }

    #[test]
    fn pct_in_password_does_not_shift_the_real_at_boundary() {
        // Controle uit het rapport: een rauwe `:` én `@` blijven de echte
        // scheiding; een `%40` middenin het wachtwoord mag die niet naar voren
        // trekken.
        assert_eq!(
            redact_secrets("https://user:s3cret-pass%40extra@host.example/a"),
            "https://user:***@host.example/a"
        );
    }

    #[test]
    fn pct_encoded_ghp_prefix_in_url_username_is_masked() {
        let with_password =
            "https://%67hp_abcdefghijklmnopqrstuvwxyz0123456789:x@github.com/org/repo.git";
        assert_eq!(
            redact_secrets(with_password),
            "https://***:***@github.com/org/repo.git"
        );

        let bare = "https://%67hp_abcdefghijklmnopqrstuvwxyz0123456789@github.com/org/repo.git";
        assert_eq!(redact_secrets(bare), "https://***@github.com/org/repo.git");
    }

    #[test]
    fn pct_encoded_byte_inside_ghp_body_is_masked() {
        let input =
            "https://ghp_abcdefghij%41abcdefghijklmnopqrstuvwxyz0123456789:x@github.com/org/repo.git";
        assert_eq!(
            redact_secrets(input),
            "https://***:***@github.com/org/repo.git"
        );
    }

    #[test]
    fn pct_encoded_ghp_prefix_entirely_is_masked() {
        let input =
            "https://%67%68%70%5Fabcdefghijklmnopqrstuvwxyz0123456789:x@github.com/org/repo.git";
        assert_eq!(
            redact_secrets(input),
            "https://***:***@github.com/org/repo.git"
        );
    }

    #[test]
    fn ghp_username_with_pct_encoded_colon_separator_is_masked() {
        // symlog-pctfixrev-0001: `url_username_is_high_risk_token` eiste een
        // rauwe `:` op `span.keep_end - 1`, maar `userinfo_colon` kan sinds
        // a923718 ook `%3A` als scheider kiezen (als er geen rauwe `:` in de
        // naam staat). Dan bleef het ghp_-token in de gebruikersnaam onvervangen
        // staan terwijl alleen het wachtwoord werd gemaskeerd.
        let input = format!("https://{GHP}%3Ax@github.com/org/repo.git");
        assert_eq!(
            redact_secrets(&input),
            "https://***:***@github.com/org/repo.git"
        );
        assert!(!redact_secrets(&input).contains(GHP));

        // Hoofdletterongevoelig: `%3a` geeft hetzelfde resultaat.
        let lower = format!("https://{GHP}%3ax@github.com/org/repo.git");
        assert_eq!(
            redact_secrets(&lower),
            "https://***:***@github.com/org/repo.git"
        );
        assert!(!redact_secrets(&lower).contains(GHP));
    }

    #[test]
    fn empty_urls_are_left_unchanged() {
        let inputs = [
            "",
            "https://",
            "http://",
            "://",
            "https://@host",
            "https://:@host",
            "https://user:@host/x",
            "https://git:@github.com/org/repo.git",
            "http://host/no-userinfo",
            "https://github.com/org/repo.git",
        ];
        for input in inputs {
            assert_eq!(redact_secrets(input), input, "{input:?}");
            assert_eq!(for_display(input, false), input);
        }
    }

    #[test]
    fn oversized_url_still_masks_a_token_username() {
        let bulk = "B".repeat(1024 * 1024);
        let with_password = format!("fetch https://{GHP}:{bulk}@github.com/org/repo.git tail");
        assert_eq!(
            redact_secrets(&with_password),
            "fetch https://***:***@github.com/org/repo.git tail"
        );
        assert!(!redact_secrets(&with_password).contains(GHP));
        assert!(!redact_secrets(&with_password).contains(&bulk));

        let huge_body = "a".repeat(1024 * 1024);
        let huge_token = format!("ghp_{huge_body}");
        let bare = format!("fetch https://{huge_token}@github.com/org/repo.git tail");
        assert_eq!(
            redact_secrets(&bare),
            "fetch https://***@github.com/org/repo.git tail"
        );
        assert!(!redact_secrets(&bare).contains(&huge_token));

        let ordinary = format!("fetch https://ada:{bulk}@github.com/org/repo.git tail");
        assert_eq!(
            redact_secrets(&ordinary),
            "fetch https://ada:***@github.com/org/repo.git tail"
        );
        assert!(redact_secrets(&ordinary).contains("ada:"));
    }

    #[test]
    fn broken_urls_do_not_leak_a_token_or_panic() {
        let missing_host = format!("https://{GHP}:x-oauth-basic@");
        assert_eq!(redact_secrets(&missing_host), "https://***:x-oauth-basic@");
        assert!(!redact_secrets(&missing_host).contains(GHP));

        let missing_at = format!("https://{GHP}:x-oauth-basic");
        assert_eq!(redact_secrets(&missing_at), "https://***:x-oauth-basic");

        let one_slash = format!("http:/{GHP}@github.com/org/repo.git");
        assert_eq!(
            redact_secrets(&one_slash),
            "http:/***@github.com/org/repo.git"
        );

        let truncated_scheme = format!("://{AKIA_KEY}:x@");
        assert_eq!(redact_secrets(&truncated_scheme), "://***:x@");
        assert!(!redact_secrets(&truncated_scheme).contains(AKIA_KEY));

        // Afgekapte sleutel: geen AKIA (precies 16), dus geen masker.
        let truncated_akia = "AKIAIOSFODNN7EXAMPL";
        assert_eq!(redact_secrets(truncated_akia), truncated_akia);

        assert_eq!(redact_secrets("https://["), "https://[");
        assert_eq!(redact_secrets("https://user:pass"), "https://user:pass");
        assert_eq!(redact_secrets("https://ada:s3cret"), "https://ada:s3cret");
        assert_eq!(for_display("https://[", true), "https://[");
    }

    #[test]
    fn ordinary_usernames_are_not_masked() {
        let scp = "git@github.com:org/repo.git";
        assert_eq!(redact_secrets(scp), scp);
        let mail = "wrote user@example.com today";
        assert_eq!(redact_secrets(mail), mail);
        let ssh = "ssh://git@github.com/org/repo.git";
        assert_eq!(redact_secrets(ssh), ssh);
        let https_user = "https://user@github.com/org/repo.git";
        assert_eq!(redact_secrets(https_user), https_user);
        let https_git = "https://git@github.com/org/repo.git";
        assert_eq!(redact_secrets(https_git), https_git);

        assert_eq!(
            redact_secrets("https://git:s3cret-pass@github.com/org/repo.git"),
            "https://git:***@github.com/org/repo.git"
        );
        assert_eq!(
            redact_secrets("https://user:s3cret-pass@github.com/org/repo.git"),
            "https://user:***@github.com/org/repo.git"
        );
        assert_eq!(
            redact_secrets("clone git@github.com:org/repo.git and ping user@host"),
            "clone git@github.com:org/repo.git and ping user@host"
        );

        let mixed = format!(
            "https://{GHP}:x@github.com/a https://ada:s3cret@db/a git@github.com:org/repo.git user@host"
        );
        assert_eq!(
            redact_secrets(&mixed),
            "https://***:***@github.com/a https://ada:***@db/a git@github.com:org/repo.git user@host"
        );
        assert_eq!(for_display(&mixed, false), mixed);
    }

    #[test]
    fn github_pat_username_is_masked() {
        assert_eq!(GITHUB_PAT.len(), "github_pat_".len() + 82);
        assert_token_username_masked(GITHUB_PAT, "github.com");

        let git_form =
            format!("git fetch https://{GITHUB_PAT}:x-oauth-basic@github.com/org/repo.git failed");
        assert_eq!(
            redact_secrets(&git_form),
            "git fetch https://***:***@github.com/org/repo.git failed"
        );
        assert!(!redact_secrets(&git_form).contains(GITHUB_PAT));
        assert_eq!(for_display(&git_form, false), git_form);

        let plain = format!("note {GITHUB_PAT} end");
        assert_eq!(redact_secrets(&plain), "note *** end");
        assert_eq!(for_display(&plain, false), plain);

        // Te kort: de naam blijft, het wachtwoord niet. Ook los in tekst.
        let too_short = &GITHUB_PAT[..GITHUB_PAT.len() - 1];
        assert_eq!(redact_secrets(too_short), too_short);
        assert_eq!(
            redact_secrets("https://github_pat_short:s3cret@github.com/org/repo.git"),
            "https://github_pat_short:***@github.com/org/repo.git"
        );

        // Zelfde token ná een voorvoegsel in de gebruikersnaam.
        let prefixed = format!("https://x-access-token-{GITHUB_PAT}:x@github.com/org/repo.git");
        assert_eq!(
            redact_secrets(&prefixed),
            "https://***:***@github.com/org/repo.git"
        );
        assert!(!redact_secrets(&prefixed).contains(GITHUB_PAT));
        assert_eq!(for_display(&prefixed, false), prefixed);
    }

    #[test]
    fn prefixed_token_in_username_is_masked() {
        let ghp = format!(
            "git fetch https://x-access-token-{GHP}:x-oauth-basic@github.com/org/repo.git failed"
        );
        assert_eq!(
            redact_secrets(&ghp),
            "git fetch https://***:***@github.com/org/repo.git failed"
        );
        assert!(!redact_secrets(&ghp).contains(GHP));

        // `_` is een identificatie-teken, dus geen gewone tokengrens. In de
        // gebruikersnaam telt het voorvoegsel toch.
        let glued = format!("https://pre_{GHP}:x@github.com/org/repo.git");
        assert_eq!(
            redact_secrets(&glued),
            "https://***:***@github.com/org/repo.git"
        );

        let akia = format!("https://svc-{AKIA_KEY}:x@s3.amazonaws.com/bucket");
        assert_eq!(
            redact_secrets(&akia),
            "https://***:***@s3.amazonaws.com/bucket"
        );
        assert!(!redact_secrets(&akia).contains(AKIA_KEY));

        let jwt = format!("https://bearer-{JWT}:x@auth.example/v1");
        assert_eq!(redact_secrets(&jwt), "https://***:***@auth.example/v1");
        assert!(!redact_secrets(&jwt).contains(JWT));
        assert!(!redact_secrets(&jwt).contains("eyJ"));
        assert_eq!(for_display(&jwt, false), jwt);

        // Zonder wachtwoord is er geen userinfo-span: alleen het token gaat weg.
        assert_eq!(
            redact_secrets(&format!(
                "https://x-access-token-{GHP}@github.com/org/repo.git"
            )),
            "https://x-access-token-***@github.com/org/repo.git"
        );
        assert_eq!(
            redact_secrets(&format!("https://bearer-{JWT}@auth.example/v1")),
            "https://bearer-***@auth.example/v1"
        );

        // Gewone namen, ook met een koppelteken, blijven staan.
        assert_eq!(
            redact_secrets("https://x-access-token:x-oauth-basic@github.com/org/repo.git"),
            "https://x-access-token:***@github.com/org/repo.git"
        );
        assert_eq!(
            redact_secrets("https://git:s3cret-pass@github.com/org/repo.git"),
            "https://git:***@github.com/org/repo.git"
        );
        assert_eq!(
            redact_secrets("https://user:s3cret-pass@github.com/org/repo.git"),
            "https://user:***@github.com/org/repo.git"
        );
        let ordinary = "clone git@github.com:org/repo.git and ping user@example.com";
        assert_eq!(redact_secrets(ordinary), ordinary);
        assert_eq!(for_display(ordinary, false), ordinary);
    }

    #[test]
    fn empty_and_broken_urls_do_not_leak_github_pat_or_prefixed_tokens() {
        let inputs = [
            "",
            "https://",
            "http://",
            "://",
            "https://@host",
            "https://:@host",
            "https://user:@host/x",
            "https://git:@github.com/org/repo.git",
            "https://[",
            "https://user:pass",
        ];
        for input in inputs {
            assert_eq!(redact_secrets(input), input, "{input:?}");
            assert_eq!(for_display(input, true), input, "{input:?}");
            assert_eq!(for_display(input, false), input, "{input:?}");
        }

        let missing_host = format!("https://{GITHUB_PAT}:x-oauth-basic@");
        assert_eq!(redact_secrets(&missing_host), "https://***:x-oauth-basic@");
        assert!(!redact_secrets(&missing_host).contains(GITHUB_PAT));

        let missing_at = format!("https://{GITHUB_PAT}:x-oauth-basic");
        assert_eq!(redact_secrets(&missing_at), "https://***:x-oauth-basic");
        assert!(!redact_secrets(&missing_at).contains(GITHUB_PAT));

        let one_slash = format!("http:/{GITHUB_PAT}@github.com/org/repo.git");
        assert_eq!(
            redact_secrets(&one_slash),
            "http:/***@github.com/org/repo.git"
        );
        assert!(!redact_secrets(&one_slash).contains(GITHUB_PAT));

        let pref_ghp = format!("https://x-access-token-{GHP}:x-oauth-basic@");
        assert_eq!(
            redact_secrets(&pref_ghp),
            "https://x-access-token-***:x-oauth-basic@"
        );
        assert!(!redact_secrets(&pref_ghp).contains(GHP));

        let pref_akia = format!("https://svc-{AKIA_KEY}:x@");
        assert_eq!(redact_secrets(&pref_akia), "https://svc-***:x@");
        assert!(!redact_secrets(&pref_akia).contains(AKIA_KEY));

        let pref_jwt = format!("https://bearer-{JWT}:x@");
        assert_eq!(redact_secrets(&pref_jwt), "https://bearer-***:x@");
        assert!(!redact_secrets(&pref_jwt).contains(JWT));
        assert!(!redact_secrets(&pref_jwt).contains("eyJ"));
        assert_eq!(for_display(&pref_jwt, false), pref_jwt);
        assert_eq!(for_display(&pref_jwt, true), "https://bearer-***:x@");
    }

    #[test]
    fn invisible_format_chars_and_whitespace_after_separator_do_not_leak() {
        // breaktest-symlog-0003, bevinding 1 en 3. Percent-encoding en de
        // kwadratische `-eyJ`-scan horen niet bij deze gevallen.
        let cases = [
            // U+200B, U+200C, U+FEFF in de sleutel
            ("pass\u{200b}word=hunter2", "pass\u{200b}word=***"),
            ("pass\u{200c}word=hunter2", "pass\u{200c}word=***"),
            ("pass\u{feff}word=hunter2", "pass\u{feff}word=***"),
            // hetzelfde vlak vóór `=`
            ("password\u{200b}=hunter2", "password\u{200b}=***"),
            ("password\u{200c}=hunter2", "password\u{200c}=***"),
            ("password\u{feff}=hunter2", "password\u{feff}=***"),
            // tussen Bearer en het token
            (
                "Authorization: Bearer\u{200b}opaque-token-999",
                "Authorization: Bearer\u{200b}***",
            ),
            (
                "Authorization: Bearer\u{200c}opaque-token-999",
                "Authorization: Bearer\u{200c}***",
            ),
            (
                "Authorization: Bearer\u{feff}opaque-token-999",
                "Authorization: Bearer\u{feff}***",
            ),
            // CR, LF, FF ná `=`
            ("password=\rhunter2", "password=\r***"),
            ("password=\nhunter2", "password=\n***"),
            ("password=\u{c}hunter2", "password=\u{c}***"),
            // CR, VT, FF, NBSP ná Bearer
            (
                "Authorization: Bearer\ropaque-token-999",
                "Authorization: Bearer\r***",
            ),
            (
                "Authorization: Bearer\u{b}opaque-token-999",
                "Authorization: Bearer\u{b}***",
            ),
            (
                "Authorization: Bearer\u{c}opaque-token-999",
                "Authorization: Bearer\u{c}***",
            ),
            (
                "Authorization: Bearer\u{a0}opaque-token-999",
                "Authorization: Bearer\u{a0}***",
            ),
            (
                "{\"Authorization\": \"Bearer\u{a0}opaque-token-999\"}",
                "{\"Authorization\": \"Bearer\u{a0}***\"}",
            ),
        ];
        for (input, expected) in cases {
            let out = redact_secrets(input);
            assert_eq!(out, expected, "{input:?}");
            let shown = for_display(input, true);
            assert!(shown.contains("***"), "{input:?} -> {shown:?}");
            assert!(!shown.contains("hunter2"), "{input:?} -> {shown:?}");
            assert!(
                !shown.contains("opaque-token-999"),
                "{input:?} -> {shown:?}"
            );
        }
    }

    #[test]
    fn a_stray_control_byte_between_key_and_operator_does_not_bypass_redaction() {
        // Bevinding breaktest-symlog-0002 #1: `skip_inline_ws` skipt alleen
        // spatie/tab. Eén ander stuurteken (NUL, `\v`, non-breaking space) tussen
        // de sleutelnaam en `=`/`:` liet `match_assignment`/`match_bearer` de
        // match stil opgeven, waardoor de waarde ongemaskeerd bleef.
        for input in [
            "password\u{0}=hunter2",
            "apikey\u{b}=rawkeyvalue",
            "secret\u{a0}: plainvalue",
            "Authorization\u{0}: Bearer opaque-leak-999",
        ] {
            let out = redact_secrets(input);
            assert!(
                out.contains("***"),
                "geheim lekt door een stuurteken vóór de scheider: {input:?} -> {out:?}"
            );
        }
    }

    #[test]
    fn url_username_high_risk_scan_stays_linear_for_jwt_like_filler() {
        // Bevinding breaktest-symlog-0002 #2: `url_username_is_high_risk_token`
        // riep `high_risk_token_covers` aan voor elke byte-positie in de
        // gebruikersnaam met `require_boundary = false`, waardoor `match_jwt` op
        // ~1 op de 3 posities een volledige scan tot het einde van de naam deed.
        // Dat was O(n^2) voor één URL.
        let user = "eyJ".repeat(20_000); // 60 000 bytes onschuldige, JWT-achtige vulling
        let input = format!("fetch https://{user}:x@github.com/org/repo.git tail");
        let start = std::time::Instant::now();
        let out = redact_secrets(&input);
        let elapsed = start.elapsed();
        assert!(out.contains("***"));
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "redact_secrets nam {elapsed:?} voor een {}-byte gebruikersnaam: \
             kwadratische blow-up in url_username_is_high_risk_token",
            user.len()
        );
    }

    #[test]
    fn invisible_format_before_closing_key_quote_does_not_leak() {
        // symlog-redactfix2rev-0001, bevinding 1. Percent-encoding en `-eyJ`
        // horen niet bij deze gevallen.
        for mark in ['\u{200b}', '\u{200c}', '\u{feff}'] {
            let input = format!("\"password{mark}\":\"hunter2\"");
            let expected = format!("\"password{mark}\":\"***\"");
            assert_eq!(redact_secrets(&input), expected, "{input:?}");
            let shown = for_display(&input, true);
            assert!(shown.contains("***"), "{input:?} -> {shown:?}");
            assert!(!shown.contains("hunter2"), "{input:?} -> {shown:?}");

            let spaced = format!("\"password{mark}\": \"hunter2\"");
            let spaced_expected = format!("\"password{mark}\": \"***\"");
            assert_eq!(redact_secrets(&spaced), spaced_expected, "{spaced:?}");
            assert!(
                !for_display(&spaced, true).contains("hunter2"),
                "{spaced:?}"
            );

            let auth = format!("\"Authorization{mark}\": \"Bearer opaque-token-999\"");
            let auth_expected = format!("\"Authorization{mark}\": \"Bearer ***\"");
            assert_eq!(redact_secrets(&auth), auth_expected, "{auth:?}");
            let auth_shown = for_display(&auth, true);
            assert!(auth_shown.contains("***"), "{auth:?} -> {auth_shown:?}");
            assert!(
                !auth_shown.contains("opaque-token-999"),
                "{auth:?} -> {auth_shown:?}"
            );
        }

        // Ná het sluitende aanhalingsteken werkte al; dat moet zo blijven.
        assert_eq!(
            redact_secrets("\"password\"\u{200b}:\"hunter2\""),
            "\"password\"\u{200b}:\"***\""
        );
        // Geen overredactie van gewone tekst of een sleutel die alleen lijkt.
        assert_eq!(
            redact_secrets("the password is hunter2"),
            "the password is hunter2"
        );
        assert_eq!(
            redact_secrets("pass\u{200b}port=abc"),
            "pass\u{200b}port=abc"
        );
        assert_eq!(redact_secrets("mysecret=val"), "mysecret=val");
    }

    #[test]
    fn invisible_format_after_separator_before_quoted_value_does_not_leak() {
        // symlog-redactfix2rev-0001, bevinding 2.
        for mark in ['\u{200b}', '\u{200c}', '\u{feff}'] {
            let eq = format!("password={mark}\"hunter2\"");
            let eq_expected = format!("password={mark}\"***\"");
            assert_eq!(redact_secrets(&eq), eq_expected, "{eq:?}");
            let shown = for_display(&eq, true);
            assert!(shown.contains("***"), "{eq:?} -> {shown:?}");
            assert!(!shown.contains("hunter2"), "{eq:?} -> {shown:?}");

            let colon = format!("\"password\":{mark}\"hunter2\"");
            let colon_expected = format!("\"password\":{mark}\"***\"");
            assert_eq!(redact_secrets(&colon), colon_expected, "{colon:?}");
            assert!(!for_display(&colon, true).contains("hunter2"), "{colon:?}");

            let mixed = format!("secret= {mark}\"plainvalue\"");
            let mixed_expected = format!("secret= {mark}\"***\"");
            assert_eq!(redact_secrets(&mixed), mixed_expected, "{mixed:?}");
            assert!(!redact_secrets(&mixed).contains("plainvalue"), "{mixed:?}");
        }

        assert_eq!(redact_secrets("hello world"), "hello world");
        assert_eq!(redact_secrets("just a token bucket"), "just a token bucket");
        assert_eq!(redact_secrets("token=\n"), "token=\n");
    }

    #[test]
    fn invisible_format_after_colon_before_quoted_bearer_value_does_not_leak() {
        // symlog-redactfix3rev-0001, bevinding 1. `match_bearer` sloeg na de `:`
        // alleen spatie/tab over (`skip_inline_ws`), niet U+200B/U+200C/U+FEFF,
        // waardoor `skip_one_quote` het openende aanhalingsteken miste en de
        // hele match werd opgegeven.
        for mark in ['\u{200b}', '\u{200c}', '\u{feff}'] {
            let direct = format!("\"Authorization\":{mark}\"Bearer opaque-token-999\"");
            let direct_expected = format!("\"Authorization\":{mark}\"Bearer ***\"");
            assert_eq!(redact_secrets(&direct), direct_expected, "{direct:?}");
            let shown = for_display(&direct, true);
            assert!(shown.contains("***"), "{direct:?} -> {shown:?}");
            assert!(
                !shown.contains("opaque-token-999"),
                "{direct:?} -> {shown:?}"
            );

            let spaced = format!("\"Authorization\": {mark}\"Bearer opaque-token-999\"");
            let spaced_expected = format!("\"Authorization\": {mark}\"Bearer ***\"");
            assert_eq!(redact_secrets(&spaced), spaced_expected, "{spaced:?}");
            assert!(
                !for_display(&spaced, true).contains("opaque-token-999"),
                "{spaced:?}"
            );
        }

        assert_eq!(
            redact_secrets("Authorization: Bearer opaque-token-999"),
            "Authorization: Bearer ***"
        );
    }

    #[test]
    fn two_format_runs_separated_by_space_after_colon_before_quoted_bearer_value_does_not_leak() {
        // symlog-bearerfmtfixrev-0001: de vorige fix sloeg na de `:` maar één
        // ws-run en één format-run over, elk precies één keer. Een tweede
        // format-run die pas ná de laatste ws-skip begint (format, spatie,
        // format, dan de quote) werd gemist, en `match_bearer` gaf de hele
        // match op.
        let input = "\"Authorization\":\u{200b} \u{200c}\"Bearer opaque-token-999\"";
        let expected = "\"Authorization\":\u{200b} \u{200c}\"Bearer ***\"";
        assert_eq!(redact_secrets(input), expected, "{input:?}");
        let shown = for_display(input, true);
        assert!(shown.contains("***"), "{input:?} -> {shown:?}");
        assert!(
            !shown.contains("opaque-token-999"),
            "{input:?} -> {shown:?}"
        );
    }

    #[test]
    fn free_text_dash_eyj_filler_scan_stays_linear_and_real_jwt_still_masked() {
        // breaktest-symlog-0003, bevinding 4. `-` is zelf een base64url-byte, dus
        // `at_jwt_boundary` laat een kandidaat ná elke `-` beginnen, en zonder
        // geheugen scande `match_jwt` elke kandidaat opnieuw tot het regeleinde:
        // O(n^2) op een vrije regel vol herhaald `-eyJ`. Deze lijn heeft geen punt,
        // dus geen van de kandidaten matcht; de uitvoer moet gelijk aan de invoer
        // blijven.
        let filler = "-eyJ".repeat(50_000); // 200 000 bytes
        let start = std::time::Instant::now();
        let out = redact_secrets(&filler);
        let elapsed = start.elapsed();
        assert_eq!(out, filler, "vrije -eyJ-vulling mag niet veranderen");
        assert!(
            elapsed < std::time::Duration::from_millis(2000),
            "redact_secrets nam {elapsed:?} voor {} bytes vrije -eyJ-vulling: \
             kwadratische blow-up in match_jwt",
            filler.len()
        );

        // Een echte (fictieve) JWT ertussen moet nog steeds gemaskeerd worden.
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9\
                   .eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0\
                   .SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let input = format!("log line before {jwt} log line after");
        let masked = redact_secrets(&input);
        assert!(masked.contains("***"), "{input:?} -> {masked:?}");
        assert!(!masked.contains(jwt), "{input:?} -> {masked:?}");
    }

    #[test]
    fn free_text_dash_eyj_filler_with_trailing_dot_scan_stays_linear_and_real_jwt_still_masked() {
        // symlog-eyjdotfix-0001, review symlog-eyjfixrev-0001 bevinding 1. Zodra de
        // `-eyJ`-run eindigt op een punt, doet `match_jwt` een tweede (en derde)
        // `end_from`-aanroep voorbij dat punt. Met één cache-sleuf overschreef die
        // aanroep het bereik van de eerste, waardoor elke volgende `eyJ`-start in
        // dezelfde run opnieuw tot het punt scande: O(n^2). Vergelijk de tijd bij
        // `n` en `10n`: bij lineair gedrag blijft de verhouding ruim onder de
        // ~100x die kwadratisch gedrag zou geven.
        fn ratio_for(build: impl Fn(usize) -> String) -> f64 {
            let n = 6_000;
            let small = build(n);
            let start = std::time::Instant::now();
            let out_small = redact_secrets(&small);
            let elapsed_small = start.elapsed().max(std::time::Duration::from_micros(1));
            assert_eq!(out_small, small, "vulling mag niet veranderen");

            let big = build(n * 10);
            let start = std::time::Instant::now();
            let out_big = redact_secrets(&big);
            let elapsed_big = start.elapsed();
            assert_eq!(out_big, big, "vulling mag niet veranderen");

            elapsed_big.as_secs_f64() / elapsed_small.as_secs_f64()
        }

        // Run die eindigt op een punt, zonder tweede segment erna.
        let ratio = ratio_for(|k| format!("{}.", "-eyJ".repeat(k)));
        assert!(
            ratio < 50.0,
            "10x invoer gaf {ratio:.1}x de tijd: kwadratische blow-up in match_jwt \
             bij een punt ná de -eyJ-run"
        );

        // Variant met een kort, mislukkend tweede segment ná het punt.
        let ratio_short_second = ratio_for(|k| format!("{}.a", "-eyJ".repeat(k)));
        assert!(
            ratio_short_second < 50.0,
            "10x invoer (kort tweede segment) gaf {ratio_short_second:.1}x de tijd: \
             kwadratische blow-up in match_jwt bij een punt ná de -eyJ-run"
        );

        // Een echte (fictieve) JWT ertussen moet nog steeds gemaskeerd worden.
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9\
                   .eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0\
                   .SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let input = format!("log line before {jwt} log line after");
        let masked = redact_secrets(&input);
        assert!(masked.contains("***"), "{input:?} -> {masked:?}");
        assert!(!masked.contains(jwt), "{input:?} -> {masked:?}");
    }

    #[test]
    fn doubled_quote_between_key_and_operator_does_not_leak() {
        // symlog-quotefix-0001. `skip_one_quote` verwacht precies één
        // aanhalingsteken; een overbodig tweede aanhalingsteken (al dan niet
        // met format-tekens/spaties ertussen) liet `match_assignment` en
        // `match_bearer` de hele match opgeven, waarna het geheim
        // ongemaskeerd bleef. Alle waarden hieronder zijn fictief.
        let gaps = ["", "\u{200b}", " ", " \u{200c} "];

        // match_assignment: dubbel aanhalingsteken ná de sleutel, vóór `:`.
        for gap in gaps {
            let input = format!("\"password\"\"{gap}:\"hunter3\"");
            let expected = format!("\"password\"\"{gap}:\"***\"");
            assert_eq!(redact_secrets(&input), expected, "{input:?}");
            assert!(!redact_secrets(&input).contains("hunter3"), "{input:?}");
        }

        // match_bearer, sleutel-kant: "Authorization" dubbel gequote.
        for gap in gaps {
            let input = format!("\"Authorization\"\"{gap}:\"Bearer opaque-token-321\"");
            let expected = format!("\"Authorization\"\"{gap}:\"Bearer ***\"");
            assert_eq!(redact_secrets(&input), expected, "{input:?}");
            assert!(
                !redact_secrets(&input).contains("opaque-token-321"),
                "{input:?}"
            );
        }

        // match_bearer, waarde-kant: dubbel aanhalingsteken vóór `Bearer`.
        for gap in gaps {
            let input = format!("\"Authorization\": \"\"{gap}Bearer opaque-token-654\"");
            let expected = format!("\"Authorization\": \"\"{gap}Bearer ***\"");
            assert_eq!(redact_secrets(&input), expected, "{input:?}");
            assert!(
                !redact_secrets(&input).contains("opaque-token-654"),
                "{input:?}"
            );
        }
    }

    #[test]
    fn soft_hyphen_word_joiner_and_mongolian_vowel_separator_do_not_leak() {
        // symlog-invischeck-0001, bevinding 1. `is_invisible_format` kende
        // alleen U+200B, U+200C en U+FEFF, dus U+00AD, U+2060 en U+180E tussen
        // `Bearer` en het token (via `skip_bearer_gap`) en tussen `=` en de
        // waarde (via `skip_ascii_ws`) sloegen niet mee en lieten het geheim
        // ongemaskeerd staan.
        for mark in ['\u{00ad}', '\u{2060}', '\u{180e}'] {
            let bearer = format!("Authorization: Bearer{mark}opaque-token-999");
            let bearer_expected = format!("Authorization: Bearer{mark}***");
            assert_eq!(redact_secrets(&bearer), bearer_expected, "{bearer:?}");
            let shown = for_display(&bearer, true);
            assert!(shown.contains("***"), "{bearer:?} -> {shown:?}");
            assert!(
                !shown.contains("opaque-token-999"),
                "{bearer:?} -> {shown:?}"
            );

            let assign = format!("token={mark}opaque-token-999");
            let assign_expected = format!("token={mark}***");
            assert_eq!(redact_secrets(&assign), assign_expected, "{assign:?}");
            let assign_shown = for_display(&assign, true);
            assert!(
                assign_shown.contains("***"),
                "{assign:?} -> {assign_shown:?}"
            );
            assert!(
                !assign_shown.contains("opaque-token-999"),
                "{assign:?} -> {assign_shown:?}"
            );
        }
    }

    #[test]
    fn cr_lf_ff_vt_after_colon_do_not_leak_bearer() {
        // symlog-crlfcheck-0001, bevinding 3. `match_bearer` deed ná de `:`
        // alleen `skip_quotes`, en die kent via `skip_ws_and_format` enkel
        // spatie en tab. CR, LF, FF en VT lieten `match_key_ignore_ascii` het
        // woord `Bearer` missen, waarna de hele match opgegeven werd en het
        // token ongemaskeerd bleef. `skip_bearer_gap` kent alle vier.
        for gap in ['\r', '\n', '\u{c}', '\u{b}'] {
            let input = format!("Authorization:{gap}Bearer opaque-token-999");
            let expected = format!("Authorization:{gap}Bearer ***");
            assert_eq!(redact_secrets(&input), expected, "{input:?}");
            let shown = for_display(&input, true);
            assert!(shown.contains("***"), "{input:?} -> {shown:?}");
            assert!(
                !shown.contains("opaque-token-999"),
                "{input:?} -> {shown:?}"
            );
        }
    }

    #[test]
    fn cr_and_tab_between_colon_and_url_password_do_not_leak() {
        // symlog-crlfcheck-0001, bevinding 3. `is_url_pass_byte` sluit ASCII-
        // witruimte uit, dus een CR of tab direct na de `:` liet `userinfo_at`
        // meteen op de leeg-check stranden en `match_url_userinfo` gaf de hele
        // match op, waardoor het wachtwoord erna ongemaskeerd bleef.
        for gap in ['\r', '\t'] {
            let input = format!("https://user:{gap}opaque-token-999@host.example/a");
            let expected = "https://user:***@host.example/a";
            assert_eq!(redact_secrets(&input), expected, "{input:?}");
            let shown = for_display(&input, true);
            assert!(shown.contains("***"), "{input:?} -> {shown:?}");
            assert!(
                !shown.contains("opaque-token-999"),
                "{input:?} -> {shown:?}"
            );
        }
    }

    #[test]
    fn cr_lf_ff_vt_tab_after_assignment_do_not_leak_a_quoted_value() {
        // symlog-assignvtfix-0001. CR, LF en FF zijn al ASCII-witruimte
        // (`u8::is_ascii_whitespace`) en werden door `skip_ascii_ws` al
        // overgeslagen. VT (`\u{000B}`) is dat niet: zonder VT in
        // `skip_ascii_ws` bleef die vóór een gequote waarde staan,
        // `scan_unquoted` stopte dan op het openende aanhalingsteken (dat is
        // wel een `is_unquoted_delimiter`-byte) en maskeerde alleen het VT-
        // teken zelf, waardoor de gequote waarde erna ongemaskeerd bleef:
        // `token=\u{B}"opaque-token-999"` -> `token=***"opaque-token-999"`.
        for gap in ['\r', '\n', '\x0C', '\x0B', '\t'] {
            let input = format!("token={gap}\"opaque-token-999\"");
            let expected = format!("token={gap}\"***\"");
            assert_eq!(redact_secrets(&input), expected, "{input:?}");
            let shown = for_display(&input, true);
            assert!(shown.contains("***"), "{input:?} -> {shown:?}");
            assert!(
                !shown.contains("opaque-token-999"),
                "{input:?} -> {shown:?}"
            );
        }
    }
}
