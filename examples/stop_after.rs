use std::time::Duration;

use teamy_cancellation::CancellationToken;
use teamy_cancellation::StopAfterArgs;
use teamy_cancellation::StopAfterLayer;
use tracing::info;
use tracing_subscriber::prelude::*;

fn main() -> eyre::Result<()> {
    let args: StopAfterArgs = figue::Driver::new(
        figue::builder::<StopAfterArgs>()
            .expect("schema should be valid")
            .cli(|cli| cli.args_os(std::env::args_os().skip(1)).strict())
            .build(),
    )
    .run()
    .unwrap();

    let token = CancellationToken::new();

    if args.stop_after_duration.is_some() {
        run_timed_demo(token, &args)?;
    } else {
        run_span_demo(token, args.stop_after_span.as_deref());
    }

    Ok(())
}

fn run_timed_demo(token: CancellationToken, args: &StopAfterArgs) -> eyre::Result<()> {
    let duration = args
        .stop_after_duration()?
        .expect("duration should exist before running timed demo");
    let _timer = args.start_stop_after_duration_thread(token.clone())?;

    println!(
        "starting work; cancellation will be requested after {}",
        humantime::format_duration(duration)
    );

    let mut iteration = 0;
    while !token.is_cancelled() {
        iteration += 1;
        println!("working iteration {iteration}");
        std::thread::sleep(Duration::from_millis(250));
    }

    println!("cancelled: {}", token.is_cancelled());
    println!(
        "reason: {}",
        token.cancellation_reason().unwrap_or_default()
    );
    Ok(())
}

fn run_span_demo(token: CancellationToken, stop_after_span: Option<&str>) {
    let stop_after_span = stop_after_span.unwrap_or("job_stage_2_complete");
    let subscriber =
        tracing_subscriber::registry().with(StopAfterLayer::new(stop_after_span, token.clone()));

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

#[cfg(test)]
mod tests {
    use teamy_cancellation::StopAfterArgs;

    #[test]
    fn parses_stop_after_duration_with_figue() {
        let args = figue::from_slice::<StopAfterArgs>(&["--stop-after-duration", "2s"]).unwrap();

        assert_eq!(args.stop_after_duration.as_deref(), Some("2s"));
        assert_eq!(
            args.stop_after_duration().unwrap().unwrap(),
            std::time::Duration::from_secs(2)
        );
    }

    #[test]
    fn parses_stop_after_span_with_figue_stop_after_name() {
        let args =
            figue::from_slice::<StopAfterArgs>(&["--stop-after", "job_stage_2_complete"]).unwrap();

        assert_eq!(
            args.stop_after_span.as_deref(),
            Some("job_stage_2_complete")
        );
    }
}
