# srvcs-percentageof

The percent-of orchestrator of the srvcs.cloud distributed standard library.

Its single concern: **arithmetic: percent% of whole.** It owns the *control
flow* — composing two float primitives — but does no arithmetic of its own. It
asks [`srvcs-floatdivide`](https://github.com/srvcs/floatdivide) for the
fraction, then [`srvcs-floatmultiply`](https://github.com/srvcs/floatmultiply)
to scale the whole.

```
percentageof(percent, whole):
    frac = floatdivide(percent, 100)   # percent as a fraction
    return floatmultiply(frac, whole)  # (percent / 100) * whole
```

`percentageof(20, 50) == 10.0`: `floatdivide(20, 100) == 0.2`, then
`floatmultiply(0.2, 50) == 10.0`.

The result is an `f64` (a JSON number that may be fractional).

Validation is not handled here. This service never calls `srvcs-isnumber`
directly; instead its dependencies validate their own operands, and any `422`
they raise is forwarded verbatim.

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/` | Service identity, concern, and dependency list |
| `POST` | `/` | Compute `percent% of whole` |
| `GET` | `/healthz` `/readyz` `/metrics` `/openapi.json` | srvcs service standard surface |

```sh
curl -s -X POST localhost:8080/ -H 'content-type: application/json' -d '{"percent": 20, "whole": 50}'
# {"percent":20.0,"whole":50.0,"result":10.0}
```

Responses:

- `200 {"percent": p, "whole": w, "result": n}` — evaluated; `result` is a float.
- `422` — a dependency rejected the input, forwarded verbatim.
- `500` — a reachable dependency returned a `200` without a numeric `result`
  (a contract violation).
- `503` — a dependency is unavailable.

## Dependencies

- [`srvcs-floatdivide`](https://github.com/srvcs/floatdivide)
- [`srvcs-floatmultiply`](https://github.com/srvcs/floatmultiply)

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `SRVCS_BIND_ADDR` | `0.0.0.0:8080` | Bind address |
| `SRVCS_FLOATDIVIDE_URL` | `http://127.0.0.1:8090` | Base URL of `srvcs-floatdivide` |
| `SRVCS_FLOATMULTIPLY_URL` | `http://127.0.0.1:8091` | Base URL of `srvcs-floatmultiply` |
| `SRVCS_ENV` | `development` | Environment label for logs |
| `RUST_LOG` | `info,tower_http=info` | Tracing filter |

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Orchestration tests stand up *computing* mock `srvcs-floatdivide` and
`srvcs-floatmultiply` services in-process — they read the request body and
return the real `a / b` / `a * b`, so the composition is genuinely exercised
against the asserted cases (with approximate `1e-9` float comparison). See
[`srvcs/platform`](https://github.com/srvcs/platform) for the shared standard.

> Note: the `cargoHash` in `flake.nix` is inherited from the template and must be
> refreshed with a `nix build` before the Nix gates pass.
