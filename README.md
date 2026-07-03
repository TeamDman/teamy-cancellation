# teamy-cancellation

Reusable cancellation primitives for Teamy Rust tools.

## Public API

- `CancellationToken`
  - shared cancellation state with `request_cancel`, `is_cancelled`,
    `cancellation_reason`, `bail_if_cancelled`, and optional cancellation hooks
- `CtrlCHandler` with feature `ctrlc`
  - configurable process-wide Ctrl+C installer, including double-Ctrl+C force-exit policy
- `StopAfterLayer` with feature `tracing-subscriber`
  - `tracing-subscriber` layer that cancels a token after a matching span close
    or event message

## Features

- `ctrlc` (default)
  - enables `CtrlCHandler`
- `tracing` (default)
  - emits tracing events at crate surface areas such as cancellation requests
    and Ctrl+C handling
- `tracing-subscriber` (default)
  - enables `StopAfterLayer`
- `inheritance` (default)
  - enables parent/child cancellation propagation via `CancellationToken::child_token()`
- `figue`
  - enables `StopAfterArgs`, a `facet`/`figue` CLI argument struct for
    `--stop-after` span/log-message cancellation and `--stop-after-duration`
    timed cancellation
- `facet`
  - derives `facet::Facet` for the public types; runtime handle types are marked
    opaque because they wrap process-local state

`tracing-subscriber` implies `tracing`. If you only want instrumentation hooks
and not the subscriber integration layer, use `default-features = false,
features = ["tracing"]`.

## Usage

Basic token usage:

```rust
use teamy_cancellation::CancellationToken;

let token = CancellationToken::new();
token.request_cancel("shutting down");
token.bail_if_cancelled()?;
```

Token with cancellation hook:

```rust
use teamy_cancellation::CancellationToken;

let token = CancellationToken::new_with_on_cancel_request(|reason, was_first| {
    if was_first {
        // cancel external worker that isn't using our cancellation token
    }
});
```

Install Ctrl+C handling:

```rust
use teamy_cancellation::CtrlCHandler;

let token = CtrlCHandler::default().install()?;
token.bail_if_cancelled()?;
```

Customize Ctrl+C handling:

```rust
use teamy_cancellation::CtrlCHandler;

let token = CtrlCHandler {
    should_eprintln_on_ctrl_c: false,
    should_force_exit_on_repeated_ctrl_c: true,
    repeated_ctrl_c_window: std::time::Duration::from_secs(2),
}
.install()?;
```

Parent/child propagation with feature `inheritance`:

```rust
use teamy_cancellation::CancellationToken;

let parent = CancellationToken::new();
let child = parent.child_token();

parent.request_cancel("shutting down");
assert!(child.is_cancelled());
```

Tracing-driven cancellation with feature `tracing-subscriber`:

```rust
use teamy_cancellation::StopAfterLayer;
use teamy_cancellation::CtrlCHandler;
use tracing_subscriber::prelude::*;

let token = CtrlCHandler::default().install()?;
tracing_subscriber::registry()
    .with(StopAfterLayer::new("job_stage_2_complete", token.clone()))
    .try_init()?;
```

Figue-driven CLI cancellation with feature `figue`:

```rust
use teamy_cancellation::CancellationToken;
use teamy_cancellation::StopAfterArgs;
use tracing_subscriber::prelude::*;

let args: StopAfterArgs = figue::from_slice(&["--stop-after", "job_stage_2_complete"])?;
let token = CancellationToken::new();

let _timer = args.start_stop_after_duration_thread(token.clone())?;
let subscriber = tracing_subscriber::registry()
    .with(args.stop_after_span_layer(token.clone()));
```

## Examples

- `cargo run --example cancellation_token`
  - manual cancellation on a shared token
- `cargo run --example ctrlc`
  - interactive Ctrl+C handling
- `cargo run --example inheritance`
  - parent token cancellation propagating to a child token
- `cargo run --example stop_after`
  - `StopAfterLayer` cancelling a token after a matching event
- `cargo run --features figue --example stop_after -- --stop-after job_stage_2_complete`
  - parse a span/log-message stop condition with `figue`
- `cargo run --features figue --example stop_after -- --stop-after-duration 2s`
  - parse a human-readable duration with `figue` and automatically request
    cancellation from a named thread after 2 seconds

`ctrlc` is interactive: run it in a terminal and press `Ctrl+C` to trigger the
installed handler.
