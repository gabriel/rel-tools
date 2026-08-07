# Contributing

Issues and focused pull requests for the clients and public API documentation
are welcome. For security problems, follow [SECURITY.md](SECURITY.md) instead of
opening a public issue.

Before submitting a change, run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd docs
npm ci
npm run check
```

API behavior is implemented by the proprietary Rel.app runtime. Proposals for
new routes or runtime behavior may be discussed here, but the implementation is
not part of this repository.
