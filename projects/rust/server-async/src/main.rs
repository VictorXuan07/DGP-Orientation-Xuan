use clap::Parser;
use rm_server_async::{Service, http::create_app_with_service};
use std::net::SocketAddr;
use std::num::NonZeroU64;
use std::sync::Arc;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:7878")]
    address: SocketAddr,
    #[arg(long, default_value = "300")]
    token_ttl_seconds: NonZeroU64,
}

#[rocket::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let app = create_app_with_service(Arc::new(Service::new(args.token_ttl_seconds)));
    let config = app
        .figment()
        .clone()
        .merge(("address", args.address.ip()))
        .merge(("port", args.address.port()))
        .merge(("log_level", "critical"));
    app.configure(config).launch().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_ttl_argument_accepts_only_positive_integers() {
        assert_eq!(
            Args::try_parse_from(["server"])
                .unwrap()
                .token_ttl_seconds
                .get(),
            300
        );
        assert_eq!(
            Args::try_parse_from(["server", "--token-ttl-seconds", "1"])
                .unwrap()
                .token_ttl_seconds
                .get(),
            1
        );
        for invalid in ["0", "-1", "abc", "1.5", "18446744073709551616"] {
            assert!(Args::try_parse_from(["server", "--token-ttl-seconds", invalid]).is_err());
        }
    }
}
