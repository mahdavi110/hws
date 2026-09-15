# HWS — Market History API Guide

> **For AI agents working on the HWS codebase.**
> HWS is an **HTTPS API + static file server** that exposes aggregated historical market data (cumulative money flow, buyer/seller power, trading volume) as a single JSON endpoint consumed by `cumchart.html`. It reads directly from the `bourse` PostgreSQL database — it **does not write anything**.
>
> This document describes the code as of 2026-09-15.

---

## 1. Project Overview

**HWS** is a small read-only HTTP service:

- **Language:** Rust (edition 2021)
- **Framework:** Actix-web 4 with OpenSSL (HTTPS)
- **Runtime:** `#[actix_web::main]` — multi-threaded tokio
- **DB driver:** `tokio-postgres` (raw SQL, no ORM)
- **Serves:** a JSON API + a single HTML page (`cumchart.html`) + its assets
- **Repo:** `https://github.com/mahdavi110/hws.git` (branch `main`)

### What it does

1. Loads TLS cert + key from disk (`cert.pem`, `key.pem`)
2. Reads `config.json` (for the default port)
3. Starts an HTTPS server on `0.0.0.0:{port}` (default 8084, overridable via `HWS_PORT`)
4. Serves four routes:
   - `GET /` — a diagnostic page listing request headers + visit counter (not the main UI)
   - `GET /healthz` — JSON health check (checks DB connectivity + data freshness)
   - `GET /getAllCumulatives` — **the main API** consumed by the frontend
   - `GET /htmls/*` — static file server (serves `cumchart.html` + JS + CSS)
5. Every request opens a fresh Postgres connection (no pooling).

### What it does NOT do

- It does **not** write to the database.
- It does **not** refresh materialized views (that is `haho`'s job).
- It does **not** authenticate users (no auth, no cookies except a visit counter).
- It does **not** schedule anything.

---

## 2. Live State (as of 2026-09-15)

| Item | Value |
|------|-------|
| Source location on brx | `/root/dock/hws/` (submodule, just checked out) |
| Image `hwsdk:latest` on brx | **NOT PRESENT** |
| Container on brx | **NOT running** |
| `docker-compose.yml` entry | yes (`hwsdk` service, port `8084:8084`, `devnet` network) |
| Crontab | none |
| Systemd unit | none |
| **Previous location** | `eepa` (`m.eepaco.ir:10037` → NAT → host:8084) — removed 2026-09-15 |
| Public URL (before removal) | `https://m.eepaco.ir:10037/htmls/cumchart.html` |

### TLS certificate state

The `cert.pem` / `key.pem` in the repo are a **self-signed** cert for `CN = m.eepaco.ir`:

```
subject=CN = m.eepaco.ir
issuer=CN = m.eepaco.ir
notBefore=Nov 26 08:34:58 2025 GMT
notAfter=Nov 26 08:34:58 2026 GMT
```

⚠ This cert is **bound to the eepa hostname**. On brx, browsers will reject it — either regenerate for the new hostname, or terminate TLS in a reverse proxy (Caddy) and run HWS over plain HTTP internally.

---

## 3. Repository Layout

```
/root/dock/hws/
├── AGENTS.md                     this file
├── Cargo.toml                    dependencies
├── Cargo.lock
├── config.json                   {"port": 8084}
├── cert.pem                      self-signed cert (CN=m.eepaco.ir)
├── key.pem                       matching private key
├── deploy                        legacy systemd deploy script (not used in containers)
├── .gitignore
├── .vscode/
├── src/
│   └── main.rs                   everything (~637 lines)
└── htmls/
    ├── cumchart.html             the UI (single page)
    ├── chart.js                  Chart.js (vendored, v3.x)
    ├── chartjs-adapter-date-fns.bundle.min.js
    ├── optimizedGaussianSmoother.js
    └── styles.css
```

Related files outside this repo (in the parent `dock`):

```
/root/dock/Dockerfile.hws        image definition
/root/dock/docker-compose.yml    hwsdk service definition
```

---

## 4. Source — `src/main.rs`

### 4.1 Entry point (`main`)

```rust
let config = parse("config.json");              // → { port: 8084 }
let port   = env HWS_PORT  or  config.port;
SslAcceptor::mozilla_intermediate(SslMethod::tls())
    .set_private_key_file("key.pem", PEM)
    .set_certificate_chain_file("cert.pem")

HttpServer::new(|| {
    App::new()
        .wrap(Cors::default().allow_any_origin().allow_any_method().allow_any_header())
        .wrap(Logger::default())
        .service(Files::new("/htmls", "htmls"))
        .service(kindex)              // GET /
        .service(healthz)             // GET /healthz
        .service(get_all_cumulatives) // GET /getAllCumulatives
        .route("/ok", web::to(HttpResponse::Ok))
})
.bind_openssl(format!("0.0.0.0:{port}"), builder)?
.keep_alive(75s)
.run()
```

### 4.2 `GET /` — `kindex()`

A **debug page**, not the main UI. Returns plain text with:
- Visit counter (incrementing per request, doubled oddly by a `fetch_add(3)` — likely a bug)
- Full request details: method, URI, headers, cookies, peer address
- Cookie `visit_count` is set (30-day expiry)
- Cookies `last_visit` set with RFC3339 timestamp

Query parameters: `username`, `name`, `id` (`KInfo` struct) — all unused besides being echoed back.

### 4.3 `GET /healthz` — `healthz()`

Returns JSON:

```json
{
  "ok": true,
  "at": "2026-09-15T15:30:00Z",
  "db_ok": true,
  "max_dt": 20260914,
  "max_dt_error": null,
  "days_behind": 1,
  "max_stale_days": 7
}
```

Logic:
1. Connect to Postgres (`select 1`) → if fails: `503`, `db_ok: false`
2. `select max(recdate)::bigint from mv_stock_cumulative_saham` → `max_dt`
3. Compare `max_dt` (parsed as `YYYYMMDD`) with `today` (UTC)
4. `stale = diff_days > HWS_HEALTH_MAX_STALE_DAYS` (default **7**)
5. `ok = !stale`

⚠ **Uses UTC for "today"**, but `max_dt` is a **Tehran-local** date from the DB. Around midnight Tehran time, `days_behind` may be off by one.

### 4.4 `GET /getAllCumulatives` — the main API

Returns one big JSON object keyed by class name / prefixed class name.

**Classes queried (hardcoded):**

```rust
let classes = [
    "saham", "s_saham", "ahrom", "s_tala", "s_sabet", "s_zamin",
    "s_amlak", "e_forush", "ati_ahrom", "s_dar_s", "sokuk",
    "e_kharid", "s_kala_ghaza", "saham_majmu",
];
```

For each class, three matviews are read:

| Key pattern | Source matview | Shape |
|-------------|----------------|-------|
| `<class>` | `mv_stock_cumulative_<class>` | `[{dt, cs}, ...]` — cumulative money flow |
| `g<class>` | `mv_daily_power_<class>` | `[{dt, cs}, ...]` — cumulative power difference |
| `v<class>` | `mv_daily_vol_<class>` | `[{dt, cs}, ...]` — daily traded value |

**Plus four extra keys:**

| Key | Source | Notes |
|-----|--------|-------|
| `apartment` | `mv_divar_apartment` | `[{dt, cs}]` — Divar apartment count |
| `plotold` | `mv_divar_plotold` | `[{dt, cs}]` — Divar old-plot count |
| `shakhes` | `b2_history` | `[{dt, cs}]` — TSE index |
| `dollar` | `dollar` | `[{dt, cs}]` — USD/IRR rate |
| `shakhes_dollar_ratio` | computed | Forward-filled ratio of `shakhes.cs / dollar.cs` |

If a matview is missing or the query fails, the key is set to `null` — the API never errors out for one bad source.

### 4.5 JSON mappers

| Function | Table | Keys |
|----------|-------|------|
| `row_to_json` | `mv_stock_cumulative_*` | `dt` = `recDate` (i64), `cs` = `cumulative_sum` (f64) |
| `prow_to_json` | `mv_daily_power_*` | `dt` = `trade_date` (i64), `cs` = `cumulative_power_difference` (f64) |
| `vrow_to_json` | `mv_daily_vol_*` | `dt` = `trading_date` (i64), `cs` = `sum_cap` (f64) |
| `drow_to_json` | `mv_divar_*` | `dt` = `date` (Date → YYYYMMDD i64), `cs` = `sum` (i64→f64) |
| `srow_to_json` | `b2_history` | `dt` = `d_even` (i32), `cs` = `x_niv_inu_cl_mres_ibs` (f64) |
| `lrow_to_json` | `dollar` | `dt` = `date` (i64), `cs` = `dollar_price` (i32→f64) |

### 4.6 `shakhes_dollar_ratio` computation

Done in memory:

1. Collect all dates from `shakhes[]` and `dollar[]`
2. Sort + dedupe
3. For each date, forward-fill the last-seen value of each series
4. Emit `{dt, cs: shakhes / dollar}` only when both series have been seen at least once

This means the ratio series begins at the **later** of the two start dates.

---

## 5. Frontend — `htmls/cumchart.html`

Single-page app. Calls **only one endpoint**:

```js
const response = await fetch('../getAllCumulatives');
```

Served at `/htmls/cumchart.html` → the relative URL resolves to `/getAllCumulatives`. This is why HWS mounts the static files at `/htmls` and the API at `/getAllCumulatives` — they are intentionally siblings.

### UI features

- **Combined normalized chart** (`#combinedNormalizedChart`) — all symbols on one canvas, each normalized to its start date
- **Individual charts** (`#chartsContainer`) — one Chart.js instance per selected symbol, rendered dynamically
- **Symbol checkboxes** (`#symbolCheckboxes`) — toggles each series on the combined chart
- **Date range slider** (`#dateSlider`) — visual zoom across the full timeline
- **Gaussian smoothing** — window + sigma inputs, applied client-side via `optimizedGaussianSmoother.js`
- **Colour palette** is hardcoded in the module script (see `symbolsAndColors` array)

### Data shape expected

```json
{
  "saham":         [{"dt": 20210102, "cs": 12345.6}, ...],
  "gsaham":        [{"dt": 20210102, "cs": 0.34},    ...],
  "vsaham":        [{"dt": 20210102, "cs": 1.2e9},   ...],
  "shakhes":       [{"dt": 20210102, "cs": 1234567.8}, ...],
  "dollar":        [{"dt": 20210102, "cs": 250000.0},  ...],
  "shakhes_dollar_ratio": [{"dt": 20210102, "cs": 4.94}, ...],
  "apartment":     [...],
  "plotold":       [...]
}
```

---

## 6. Environment Variables

| Var | Default | Purpose |
|-----|---------|---------|
| `PGHOST` | `pgdk` | DB host |
| `PGPORT` | `5432` | DB port |
| `PGUSER` | `dev` | DB user |
| `PGPASSWORD` | `vatanampareyetanameyiran` | DB password |
| `PGDATABASE` | `bourse` | DB name |
| `HWS_PORT` | `config.json` → 8084 | HTTPS listen port |
| `HWS_HEALTH_MAX_STALE_DAYS` | 7 | `/healthz` staleness threshold |
| `RUST_LOG` | `info` | logging level (actix-server, actix-web) |

`config.json` (in the repo) has `{"port": 8084}` as fallback.

---

## 7. Container / Deployment

### 7.1 Dockerfile.hws

```
Stage 1 (builder): rust:1-bookworm
    → cargo build --release
Stage 2 (runtime): debian:bookworm-slim
    + ca-certificates, tzdata, openssl, libssl3, libpq5
    → copies hws binary + config.json + cert.pem + key.pem + htmls/
WORKDIR /app
EXPOSE 8084
CMD ["/usr/local/bin/hws"]
```

### 7.2 docker-compose service

```yaml
hwsdk:
  container_name: hwsdk
  build:
    context: .
    dockerfile: Dockerfile.hws
  image: hwsdk:latest
  depends_on:
    pgdk:
      condition: service_healthy
  environment:
    PGHOST: pgdk
    PGPORT: "5432"
    PGUSER: ${POSTGRES_USER:-dev}
    PGPASSWORD: ${POSTGRES_PASSWORD:-vatanampareyetanameyiran}
    PGDATABASE: ${POSTGRES_DB:-bourse}
    RUST_LOG: info
    HWS_PORT: "8084"
  ports:
    - "8084:8084"
  networks:
    - devnet
```

### 7.3 Build & run

```bash
cd /root/dock
docker build -f Dockerfile.hws -t hwsdk:latest .
docker run -d \
  --name hwsdk \
  --network dock_net \
  --restart unless-stopped \
  -e PGHOST=pgdk \
  -e PGUSER=dev \
  -e PGPASSWORD=vatanampareyetanameyiran \
  -e PGDATABASE=bourse \
  -e HWS_PORT=8084 \
  -e RUST_LOG=info \
  -p 8084:8084 \
  hwsdk:latest
```

The container must have `cert.pem`, `key.pem`, `config.json`, and `htmls/` in `/app` (WORKDIR). These are copied at build time from the submodule.

### 7.4 Legacy `deploy` script

The `deploy` file in the repo is a **systemd-based deployment script** that writes to `/opt/hws/`, stops/starts `hws.service`, and copies the binary. **It does not apply to the containerized flow** and depends on a systemd unit that no longer exists. Kept for historical reference.

---

## 8. Operations

### 8.1 Health check

```bash
curl -k https://localhost:8084/healthz | jq
```

Expected response:
```json
{
  "ok": true,
  "db_ok": true,
  "max_dt": 20260914,
  "days_behind": 1,
  "max_stale_days": 7
}
```

### 8.2 Verify the main API

```bash
curl -k https://localhost:8084/getAllCumulatives | jq 'keys'
# expect: ["ahrom","apartment","ati_ahrom","dollar","e_forush",...]
```

### 8.3 Check individual class has data

```bash
curl -k https://localhost:8084/getAllCumulatives | jq '.saham | length'
```

### 8.4 Logs

```bash
docker logs hwsdk --tail 50 -f
```

---

## 9. Known Quirks

### 9.1 `kindex` has a weird counter
```rust
let previous = data.global_count.fetch_add(3, Ordering::SeqCst);
```
Adds **3** per request, not 1. Almost certainly a bug from earlier experimentation. Harmless because the counter isn't used for anything real.

### 9.2 New DB connection per request
Every call to `get_client()` opens a fresh TCP connection to Postgres — there's no pool. For a page that loads ~20 JSON keys, that's ~20 DB round-trips to open connections. Under load this could exhaust Postgres connections. Fix would be `deadpool-postgres` but nothing is broken today.

### 9.3 `SQL injection by table name`
```rust
client.query(&format!("SELECT * FROM {}", table_name), &[])
```
`table_name` comes from a hardcoded array, so it's not exploitable **as written**. But if anyone ever makes the class list configurable, this becomes a vulnerability. Do not accept table names from user input.

### 9.4 Self-signed cert is bound to `m.eepaco.ir`
The cert in the repo (`cert.pem`/`key.pem`) has `CN = m.eepaco.ir`. On any other host, browsers will reject the certificate. Options:
- Regenerate for the new hostname
- Terminate TLS at a reverse proxy (Caddy) and run HWS on plain HTTP behind it

### 9.5 `HWS_PORT` vs `config.json`
The code prefers `HWS_PORT` env if set, else `config.json["port"]`. Both default to 8084. In the container both mechanisms exist — no conflict today, but they can disagree.

### 9.6 `/healthz` uses UTC for "today"
`Utc::now().date_naive()` is compared against `max_dt` (a Tehran-local date). Around Tehran midnight this can produce off-by-one `days_behind`. If it matters, switch to `chrono_tz::Asia::Tehran`.

### 9.7 Static files mount at `/htmls`
`Files::new("/htmls", "htmls")` — so URLs are `/htmls/cumchart.html`, `/htmls/chart.js`, etc. Do not confuse this with the `hws` binary's own route `/` (which is the diagnostic page, not the chart).

### 9.8 The ratio series has no value before the later start
`shakhes_dollar_ratio` only appears after **both** `shakhes` and `dollar` have at least one data point. If one of them is empty (e.g. divarel stopped updating `dollar`), the ratio is `null`.

### 9.9 `logs show "no such table"` when matviews are missing
If `haho` hasn't been run recently, some `mv_*` matviews may not exist yet. `get_all_cumulatives()` catches this and puts `null` in the JSON — the frontend then has to hide that series. No hard error, but silent degradation.

---

## 10. Server Context (brx)

- **Server:** `185.105.239.25` (`srv9201603445`)
- **Container network:** `dock_net`
- **DB:** `pgdk` (PostgreSQL 16, `bourse`)
- **Related services on same host:**
  - `haho` (via `appdk`) — populates the matviews this service reads
  - `divarel` — populates `dollar` (currently not running)
  - `brx`, `brx-staging` — unrelated UI services
- **eepa (192.168.10.7):** previously hosted this service on port 8084 with NAT `10037 → 8084`. Source removed 2026-09-15.

### Dependencies

- **Upstream:** `pgdk` (Postgres) — specifically the `mv_stock_cumulative_*`, `mv_daily_power_*`, `mv_daily_vol_*`, `mv_divar_*` matviews, plus `b2_history` and `dollar` tables.
- **Downstream:** any browser fetching `/htmls/cumchart.html`.

---

## 11. Migration History

- 2026-09-15: Source was extracted from `/root/dock/hws/` on eepa. The image and container on eepa were deleted as part of the bourse-pipeline cleanup. Source now lives on brx as a `dock` submodule, but **no image has been built yet**.
- The **self-signed cert** that was used on eepa is still in the repo (bound to `m.eepaco.ir`). Regenerate or bypass before deploying on brx.

---

## 12. Future Improvements (Roadmap)

1. **Connection pooling** — replace per-request `tokio_postgres::connect` with `deadpool-postgres`.
2. **Regenerate the TLS cert** for the brx hostname (or terminate TLS upstream).
3. **Make the class list config-driven** — currently hardcoded in `get_all_cumulatives`.
4. **Fix `/healthz` timezone** — use `Asia/Tehran` for "today".
5. **Remove or repurpose `/`** — the diagnostic page is not intended for end users.
6. **Add `/readyz`** — separate liveness from readiness.
7. **Cache the matview read** — the API recomputes on every page load; a 30-second cache would dramatically reduce DB load.

---

## 13. Quick Reference

| Task | Command |
|------|---------|
| Build image | `cd /root/dock && docker build -f Dockerfile.hws -t hwsdk:latest .` |
| Run | `docker run -d --name hwsdk --network dock_net -e PGHOST=pgdk -e PGUSER=dev -e PGPASSWORD=... -e PGDATABASE=bourse -p 8084:8084 hwsdk:latest` |
| Health | `curl -k https://localhost:8084/healthz` |
| Main API | `curl -k https://localhost:8084/getAllCumulatives` |
| UI | `https://<host>:8084/htmls/cumchart.html` |
| Logs | `docker logs hwsdk --tail 50 -f` |
| Repo | `https://github.com/mahdavi110/hws.git` |

---

**Last updated:** 2026-09-15 (brx migration)
