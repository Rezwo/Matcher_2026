#![allow(non_snake_case)]

use tokio::time;

async fn Run() {
    println!("Sleeping");
    tokio::time::sleep(time::Duration::from_secs(2)).await;
    println!("Awake");
}
fn main() {
    let Runtime = tokio::runtime::Runtime::new().unwrap();
    let Future = Run();

    Runtime.block_on(Future);
}