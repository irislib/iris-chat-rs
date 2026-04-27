fn main() -> anyhow::Result<()> {
    let bind_addr = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("NDR_LOCAL_RELAY_BIND").ok())
        .unwrap_or_else(|| "0.0.0.0:4848".to_string());
    iris_chat_core::local_relay::run_forever(&bind_addr)
}
