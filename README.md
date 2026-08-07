# Rel tools

Open-source command-line, MCP, and Rust clients for
[Rel](https://rel.me), plus the public contract for its local versioned API.

Rel is not an open-source product. This repository intentionally contains only
clients and documentation for integrating with the proprietary macOS app. It
does not contain Rel.app, the local agent implementation, Chromium integration,
session storage, proxy credentials, or internal service code.

## What is here

- [`rel-cli`](crates/rel-cli): the `rel` command and its stdio MCP adapter.
- [`rel-client`](crates/rel-client): a typed synchronous Rust client for RPC v1.
- [`docs`](docs): the canonical CLI, MCP, RPC, and Rust SDK documentation that
  powers [docs.rel.me](https://docs.rel.me).

All clients require Rel.app in `/Applications`; they connect only to its
loopback API. Download the app from [rel.me](https://rel.me).

## Install the CLI

The app's **Settings → General → Install Command Line** task installs the
bundled, matching CLI into a writable directory already in `PATH`. To build the
public client independently:

```sh
cargo install --git https://github.com/gabriel/rel-tools \
  --tag v0.1.1 \
  --package rel-cli
```

Then try:

```sh
rel health
rel navigate https://example.com
rel capture > example.html
```

## Use the Rust client

Until a crates.io release is announced, pin the public repository tag:

```toml
[dependencies]
rel-client = { git = "https://github.com/gabriel/rel-tools", tag = "v0.1.1" }
```

```rust,no_run
use rel_client::RelClient;

let client = RelClient::local();
let status = client.status()?;
println!("{}", status.data.overall_status);
# Ok::<(), rel_client::ClientError>(())
```

See the [Rust SDK guide](docs/SDK.md) and [RPC v1 contract](docs/RPC.md).

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd docs
npm ci
npm run check
```

The source code in this repository is MIT licensed. Rel.app and the Rel brand
are not covered by that license.
