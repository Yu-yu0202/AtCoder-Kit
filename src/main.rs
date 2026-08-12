use log::error;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = atcoder_kit::run().await {
        error!("{error}");
        std::process::exit(1);
    }
}
