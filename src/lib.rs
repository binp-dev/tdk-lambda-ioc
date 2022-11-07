use ferrite::{entry_point, Context};
use macro_rules_attribute::apply;
use tokio::runtime;

/// *Export symbols being called from IOC.*
pub use ferrite::export;

#[apply(entry_point)]
fn app_main(mut ctx: Context) {
    use env_logger::Env;
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let rt = runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(async_main(ctx));
}

async fn async_main(ctx: Context) {
    log::info!("start");
    for var in ctx.registry.values() {
        log::info!("variable: {}", var.name());
    }
    log::info!("stop");
}
