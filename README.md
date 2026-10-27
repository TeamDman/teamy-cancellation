# teamy-cancellation

Reusable cancellation primitives for Teamy Rust tools.

## Public API

- `CancellationToken`
  - shared cancellation state with `request_cancel`, `is_cancelled`,
    `cancellation_reason`, listeners, and optional cancellation hooks
- `CancellationScope`
  - callback/future-owned execution policy, linked children, and `bail_if_cancelled`
- `CancellationError`
  - typed cancellation reason returned by scoped execution
- `CancellationErrorExt`
  - recognize, mutate, or extract typed cancellation in `eyre::Report`
  - support I/O errors with feature `io`, including source-chain detection
- `CtrlCHandler` with feature `ctrlc`
  - configurable process-wide Ctrl+C installer, including double-Ctrl+C force-exit policy
- `StopAfterLayer` with feature `tracing-subscriber`
  - `tracing-subscriber` layer that cancels a token after a matching span close
    or event message

With the optional async feature, CancellationWait is a sticky, multi-consumer
cancellation signal built on standard Rust wakers. It works without a particular
async runtime. CancellationListener provides an unregistering, once-only
callback registration, including notification when registered after a request.

Execution policy belongs to `CancellationScope`, supplied by a bounded callback
or future. Nested helpers create linked child scopes automatically. A protected
region shields its descendants and keeps its enclosing scope wrappers polling;
independent sibling scopes remain cancellable. Scope handles can be cloned, but
only the executing callback/future owns protection and its lifetime.

The independent `deferral` feature supports synchronous protected callbacks.
Together with `async`, it also supports protected futures across suspension.
`CancellationToken` remains the shared request/reason/notification handle;
`is_cancelled()` and `cancelled().await` always observe raw intent.
`CancellationScope`, synchronous `with_scope`, and `CancellationError` are
available without optional features. The error preserves the request's reason.
Operation code uses its supplied scope's `bail_if_cancelled()` so that required
completion is respected.

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
- `async`
  - enables runtime-independent cancellation waiting and future-owned execution scopes
- `deferral`
  - enables bounded protected scopes for synchronous work, and protected futures
    when `async` is also enabled; neither feature enables the other
- `io`
  - enables synchronous `CancellationWriteExt` helpers and `CancellableWriter`
    plus I/O support for `CancellationErrorExt`, independently of `async` and `deferral`
- `arbitrary`
  - generates isolated, uncancelled tokens and inert request-scope placeholders,
    without signal handlers or live state
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

## Facet and Figue integration dependencies

Version 0.7.1's optional Facet and Figue integrations use published TeamDman forks:
[teamy-facet](https://crates.io/crates/teamy-facet) at 0.50.0-rc.7 and
[teamy-figue](https://crates.io/crates/teamy-figue) at 6.0.0-rc.5.
Figue uses Teamy JSON and Format 0.50.0-rc.8; the core Facet crates remain at
0.50.0-rc.7.
These are separate packages from upstream
[Facet](https://github.com/facet-rs/facet) and
[Figue](https://github.com/bearcove/figue), with their existing Rust library names
retained as dependency aliases. No Git patch is required.

## Usage

Basic token usage:

```rust
use teamy_cancellation::CancellationToken;

let token = CancellationToken::new();
token.request_cancel("shutting down");
assert!(token.is_cancelled());
```

Synchronous completion with feature `deferral`:

```rust
use teamy_cancellation::CancellationToken;

let token = CancellationToken::new();
let result: eyre::Result<()> = token.with_scope(|scope| {
    scope.with_deferral("finishing related writes", |protected| {
        // Perform the related writes using this protected scope.
        protected.bail_if_cancelled()?;
        Ok(())
    })?;
    scope.bail_if_cancelled()?;
    Ok(())
});
```

The callback owns the scope. Returning an unpolled future from a synchronous
callback does not extend protection: its scope closes as soon as the callback
returns. Use the async helpers for work that suspends.

Synchronous output with feature `io`:

```rust
use std::io::Write;
use teamy_cancellation::CancellableWriter;
use teamy_cancellation::CancellationToken;
use teamy_cancellation::CancellationWriteExt;

let token = CancellationToken::new();
let mut bytes = Vec::new();
let result: eyre::Result<()> = token.with_scope(|scope| {
    bytes.write_all_with_cancellation(b"first record\n", &scope)?;
    let mut writer = CancellableWriter::new(&mut bytes, &scope);
    writeln!(writer, "second record")?;
    writer.flush()?;
    Ok(())
});
```

The helpers check the supplied scope before each write and interrupted retry,
limiting individual write requests to 64 KiB. The adapter also checks `flush`
and integrates with serializers and other APIs accepting `std::io::Write`.
Cancellation is returned as `io::ErrorKind::Other` containing a `CancellationError`
payload accessible through `CancellationErrorExt`; it is not `Interrupted`, which
ordinary `write_all` implementations retry. Deferral remains effective. Written bytes
are retained, so cancellation may leave partial output. These synchronous
helpers cannot interrupt an underlying write or flush that is blocked; they
check again when control returns and another write is needed.

Use `as_cancellation()` to search the payload and its sources, or
`as_cancellation_mut()` to mutably borrow the direct cancellation payload.
When owning the I/O error, prefer `into_cancellation()` to moving through a
temporary or default placeholder:

```rust
use std::io;
use teamy_cancellation::CancellationError;
use teamy_cancellation::CancellationErrorExt;

let error = io::Error::other(CancellationError::new("output cancelled"));
let cancellation = error.into_cancellation().expect("direct cancellation payload");
assert_eq!(cancellation.reason, "output cancelled");
```

Consuming extraction moves the original cancellation without cloning its reason.
Success removes the I/O wrapper and its error kind; failure returns the original
I/O error unchanged. Mutation and extraction inspect only the direct boxed
payload; shared detection also searches the source chain.

Cancellation in an `eyre::Report` is available without optional features:

```rust
use teamy_cancellation::CancellationError;
use teamy_cancellation::CancellationErrorExt;

let report = eyre::Report::new(CancellationError::new("work cancelled"))
    .wrap_err("finishing the command");
assert!(report.is_cancellation());
assert_eq!(report.as_cancellation().unwrap().reason, "work cancelled");
assert!(report.downcast_ref::<CancellationError>().is_some());
```

`CancellationErrorExt` prefers native typed report downcasts, including typed
contexts, then follows exposed `Error::source()` chains. With `io`, it also
checks a directly downcastable I/O context and traverses boxed I/O payloads,
including nested I/O wrappers. It does not inspect hidden error graphs or infer
cancellation from error messages.

`as_cancellation_mut()` and `into_cancellation()` use native report downcasts,
including typed cancellation contexts. They do not search source chains or I/O
payloads inside reports. Extraction moves the cancellation without cloning its
reason and discards surrounding report context; failure returns the report
unchanged. Shared detection can therefore succeed when extraction cannot.

Async cancellation with features `async` and `deferral`:

```rust
use teamy_cancellation::CancellationToken;

let invocation = CancellationToken::new();
let result: eyre::Result<()> = invocation.cancellable_scope(|scope| async move {
    scope.cancellable_scope(|read| async move {
        // Await an independently cancellable read using `read`.
        read.bail_if_cancelled()?;
        Ok::<_, eyre::Report>(())
    }).await?;

    scope.deferred("finishing mutation and cache invalidation", |protected| async move {
        // Await the mutation and all required completion using `protected`.
        // Deeper requests create linked children from this supplied scope.
        protected.bail_if_cancelled()?;
        Ok::<_, eyre::Report>(())
    }).await?;

    scope.bail_if_cancelled()?;
    Ok(())
}).await;
```

Use `token.with_async_scope(|scope| async move { ... })` for an application or
shutdown coordinator that owns its cancellation checkpoints and cleanup. After
admission this helper keeps polling the body when cancellation is requested;
opt into automatic stopping around its reads or jobs with
`scope.cancellable_scope(...)`. This lets the coordinator observe a child's
cancellation error and finish restoring its resources. Both helpers reject new
optional work when cancellation was already requested.

With `async` and `inheritance`, `scope.cancellable_child_scope(factory)` returns
`(control_token, future)`: the token can cancel that job without cancelling its
parent or siblings. Parent requests still propagate, and the child retains its
scope ownership links so deeply nested deferrals protect the required owners.
`with_async_child_scope` provides the same separate signal for a cooperative
coordinator. Pass the factory's supplied scope into the job's requests.

`CancellationScope::unbound()`, `Default`, and `Arbitrary` produce an isolated,
already-closed placeholder for reflected request input. They cannot run work or
required cleanup. Invocation infrastructure must bind the supplied live scope
before executing generated requests; a placeholder never substitutes for an
execution owner.

Each async factory runs only after first-poll admission. An unpolled deferred
future has no active protection. Cancellation before admission prevents optional
work from beginning. Protection is inherited only within the protected subtree;
ancestors retain their owning futures without shielding ordinary sibling scopes.
Once the protected body completes, protection ends even if scope clones survive.
Saved handles cannot admit new work after their owner closes.

For cleanup already owed when cancellation is requested, use
`scope.finish_required(reason, |protected| async move { ... })` or synchronous
`scope.finish_required_sync(reason, |protected| { ... })`. These allow admission
after a request but cannot reopen a closed owner. Prefer protecting a mutation
and its cleanup together before the irreversible phase. Each active protected
scope acknowledges cancellation once with its explanation and caller location
when tracing is enabled; raw listeners still receive the request immediately.

A cancellation request wakes pending wrappers while their work awaits unrelated
I/O. Checks occur at wrapper poll boundaries, not every syntactic await, and
cannot interrupt a running poll. A body's original Ready success or error wins
over concurrent cancellation. Completing a protected body preserves its result;
pending cancellation is observed at the next scope checkpoint or child admission.
Ordinary errors still follow the body's Rust control flow; protection does not
force later cleanup statements to run after `?` returns an error.

Only cancellation generated by these scope helpers honors the ownership links.
Explicitly dropping an owner, task aborts, process termination, or an early return
from `select!`/`try_join!` can still destroy protected work. Parallel required work
needs a coordinator that deliberately retains and drains protected children on
those paths. Starting another root from a raw token shares intent but creates
independent ownership; nested operations should use the supplied scope instead.
Use each callback's supplied child handle rather than a captured ancestor: scope
links describe the ownership you explicitly establish, not the async call stack.

## Migrating from token-owned deferrals

This is a breaking API cutover. The old APIs are removed rather than retaining
an unlinked compatibility path:

| Previous API | Scoped replacement |
| --- | --- |
| `token.cancellable(future)` / `CancellableFuture` | `token.cancellable_scope(|scope| future)` / `CancellableScopeFuture` |
| `token.for_operation()` | `scope.cancellable_scope(|child| future)` or synchronous `scope.with_scope(...)` |
| `token.defer_cancellation(reason)` / `CancellationDeferral` | `scope.deferred(reason, |protected| future)` or `scope.with_deferral(...)` |
| `token.bail_if_cancelled()` and token-based SDK checkpoints | Pass `CancellationScope` and check that supplied scope |

Raw token cloning, child-token request propagation, listeners, Ctrl+C handling,
and raw cancellation waiting keep their signal-only roles. There is no automatic
executor, tracing-span, or task-local ownership inference.

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
let result: eyre::Result<()> = token.with_scope(|scope| {
    // Pass `scope` into synchronous operation implementations.
    scope.bail_if_cancelled()?;
    Ok(())
});
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
