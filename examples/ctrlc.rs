use std::thread;
use std::time::Duration;
use teamy_cancellation::CtrlCHandler;

fn main() -> eyre::Result<()> {
    let token = CtrlCHandler::default().install()?;

    println!("Press Ctrl+C to request graceful shutdown.");

    while !token.is_cancelled() {
        println!("waiting for Ctrl+C...");
        thread::sleep(Duration::from_secs(1));
    }

    println!(
        "cancelled: {}",
        token.cancellation_reason().unwrap_or_default()
    );
    token.bail_if_cancelled()
}
