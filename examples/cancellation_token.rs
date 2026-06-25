use teamy_cancellation::CancellationToken;

fn main() {
    let token = CancellationToken::new();
    let worker_token = token.clone();

    worker_token.request_cancel("example requested shutdown");

    println!("cancelled: {}", token.is_cancelled());
    println!(
        "reason: {}",
        token.cancellation_reason().unwrap_or_default()
    );
}
