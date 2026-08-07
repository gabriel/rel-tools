# rel-client

`rel-client` is the typed synchronous Rust client for Rel RPC v1. It contains no
app lifecycle, SQLite, browser runtime, or local log-file code.

```rust,no_run
use rel_client::{CaptureRequest, RelClient};

let client = RelClient::local();
let mut capture = client.capture(&CaptureRequest::new("https://example.com"))?;
for event in capture.by_ref() {
    println!("{:?}", event?);
}
assert_eq!(capture.exit_code(), Some(0));
# Ok::<(), rel_client::ClientError>(())
```

Ordinary methods return a typed `RpcResponse<T>`. Server failures are preserved
as `ClientError::Rpc(RpcFailure)` with the stable error ID, HTTP code, message,
retryability, details, and request ID. Capture is a validated NDJSON iterator.

See [`docs/SDK.md`](../../docs/SDK.md) and
[`docs/RPC.md`](../../docs/RPC.md) for the complete API and wire contract.
