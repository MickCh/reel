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
