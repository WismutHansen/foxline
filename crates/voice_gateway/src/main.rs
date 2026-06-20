use anyhow::Result;
use clap::Parser;
use foxline_voice_gateway::{
    config::{Cli, GatewayConfig},
    ws,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("foxline_voice_gateway=info".parse()?),
        )
        .init();

    let cli = Cli::parse();
    if cli.print_config_schema {
        println!("{}", GatewayConfig::schema_json()?);
        return Ok(());
    }

    let config = GatewayConfig::load(&cli)?;
    ws::serve(config).await
}
