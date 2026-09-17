# Native PsyNet MITM certs

The PsyNet proxy is **in-process Rust** (`src-tauri/src/proxy.rs`), not Go.

Compile requires (via `include_bytes!`):

- `velocityrl_ca.crt`
- `leaf_config.psynet.gg.{crt,key}`
- `leaf_ws.rlpp.psynet.gg.{crt,key}`

`velocityrl_ca.key` stays local (gitignored) — only needed to mint new leaves.

Regenerate: `python3 gen_certs.py` then update `BUNDLED_CA_THUMBPRINT` in `src-tauri/src/psynet.rs`.
