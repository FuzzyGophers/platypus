# Respecting upstream data sources

Platypus treats upstream data as a **privilege, not an entitlement**. Radio data comes from services
other people run and pay to curate, so every source Platypus integrates — networked APIs today,
whatever we add tomorrow — is built to lean on the provider as lightly as possible. These are the
principles every source integration follows; the per-source detail lives in [`sources.md`](sources.md)
and the sourcing policy in [`../CREDITS.md`](../CREDITS.md).

- **Cache-first.** Every response is cached on disk, keyed by the exact query, so a given request hits
  the provider **at most once** — every repeat is served from the local cache.

- **Throttled & identified.** Live requests are spaced out (a minimum gap between real network calls)
  and carry a descriptive `User-Agent`, so traffic is gentle and honest about who's calling. Cache
  hits skip the throttle entirely — being polite never means being slow on data you already have.

- **Only what you need.** Fetches are **scoped and on-demand** — location-first, "give me my area" —
  not bulk scraping of a provider's whole database. You pull what's near you, not the country.

- **Freshness without waste.** Cached entries carry a TTL, and where a source exposes per-record
  change timestamps, a cheap parent query is used to invalidate a heavy child query **precisely** — so
  we re-download a detail only when the provider actually changed it. If the network is unavailable, we
  fall back to the cached copy: staying up never costs an extra request.

- **Your account, your data.** Where a source is authenticated, access uses **your own credentials** —
  stored in the OS keychain, never proxied through a shared account or redistributed — and the cache is
  scoped per account.

- **Facts only.** From any source we take **facts** — frequencies, offsets, service-type codes, format
  constants — never its copyrighted *expression*. See [`../CREDITS.md`](../CREDITS.md).

- **Offline & public sources cost nothing.** Local files and public records (e.g. Sentinel/HPDB card
  files, FCC ULS data) place **no load** on any live service at all.

**In practice today:** the one networked source is [RadioReference](https://www.radioreference.com/)
over its SOAP API, and the cache, throttle, TTL, per-record freshness, and `User-Agent` all live in
[`crates/platypus-rr`](../crates/platypus-rr/src/lib.rs) (see its `Options`). A new networked source is
expected to meet the same bar; the per-source acquisition and freshness strategy is tracked in
[`sources.md`](sources.md).
