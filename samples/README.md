# Sample card files — test fixtures

The committed fixtures are the **synthetic** set under [`synthetic/`](synthetic/) — fictional,
hand-built card files that name no real place, used by the value-asserting tests and the
byte-exact round-trip gate. See [`synthetic/README.md`](synthetic/README.md);
`synthetic/generate.py` builds them.

Real card dumps (a scanner's actual SD-card files) are **not committed** — they are
Uniden/RadioReference data and stay **local only**, opaque round-trip inputs. The round-trip
gate walks whatever fixtures are present, so it runs on the synthetic set on a fresh clone and
can additionally cover real files when a developer has them locally.

## Validate

```
python3 ../tools/inspect_hpd.py  synthetic   # format recon: record types, field counts, headers
python3 ../tools/roundtrip_hpd.py synthetic  # byte-exact round-trip gate (must be N/N clean)
```
