#[tokio::main]
async fn main() -> anyhow::Result<()> {
    terrain_api::run().await
}
