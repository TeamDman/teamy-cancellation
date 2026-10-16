use std::thread;
use std::time::Duration;
use teamy_cancellation::CtrlCHandler;

fn main() -> eyre::Result<()> {
    let token = CtrlCHandler::default().install()?;

    token.with_scope(|scope| {
        println!("Press Ctrl+C to request graceful shutdown.");

        while !scope.is_cancelled() {
            println!("waiting for Ctrl+C...");
            thread::sleep(Duration::from_secs(1));
        }

        println!(
            "cancelled: {}",
            scope.cancellation_reason().unwrap_or_default()
        );
        scope.bail_if_cancelled()?;
        Ok(())
    })
}
