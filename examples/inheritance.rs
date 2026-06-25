use teamy_cancellation::CancellationToken;

fn main() {
    let parent = CancellationToken::new();
    let child = parent.child_token();

    parent.request_cancel("parent requested shutdown");

    println!("parent cancelled: {}", parent.is_cancelled());
    println!("child cancelled: {}", child.is_cancelled());
    println!(
        "child reason: {}",
        child.cancellation_reason().unwrap_or_default()
    );
}
