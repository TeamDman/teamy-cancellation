use teamy_cancellation::CancellationToken;
use teamy_cancellation::StopAfterLayer;
use tracing::info;
use tracing_subscriber::prelude::*;

fn main() {
    let token = CancellationToken::new();
    let subscriber = tracing_subscriber::registry()
        .with(StopAfterLayer::new("job_stage_2_complete", token.clone()));

    tracing::subscriber::with_default(subscriber, || {
        info!("startup");
        info!("job_stage_2_complete");
        info!("this event still runs, but the token is now cancelled");
    });

    println!("cancelled: {}", token.is_cancelled());
    println!(
        "reason: {}",
        token.cancellation_reason().unwrap_or_default()
    );
}
