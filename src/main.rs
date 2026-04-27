use anyhow::Result;
use rmcp::{ServiceExt, transport::io::stdio};

mod router;
mod tools;

use crate::router::Charta;

#[tokio::main]
async fn main() -> Result<()> {
    Charta::default().serve(stdio()).await?.waiting().await?;
    Ok(())
}
