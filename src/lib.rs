mod device;

/// *Export symbols being called from IOC.*
pub use ferrite::export;

use ferrite::{entry_point, Context};
use macro_rules_attribute::apply;
use std::collections::HashMap;
use tokio::runtime;

use crate::device::Device;

#[apply(entry_point)]
fn app_main(mut ctx: Context) {
    use env_logger::Env;
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();
    let rt = runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(async_main(ctx));
}

async fn async_main(mut ctx: Context) {
    log::info!("start");
    let devices = (0..7)
        .into_iter()
        .map(|addr| (addr, Device::new(addr, &mut ctx)))
        .collect::<HashMap<_, _>>();
    for (addr, _device) in devices.iter() {
        log::info!("device: addr {}", addr);
    }
    log::info!("stop");
}
