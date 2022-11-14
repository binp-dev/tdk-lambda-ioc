#![forbid(unsafe_code)]

mod device;
#[cfg(feature = "emulate")]
mod emulator;
mod serial;
mod utils;

/// *Export symbols being called from IOC.*
pub use ferrite::export;

use ferrite::{entry_point, Context};
use futures::executor::block_on;
use macro_rules_attribute::apply;
use tokio::runtime;

use crate::{device::Device, serial::Multiplexer};

#[apply(entry_point)]
fn app_main(mut ctx: Context) {
    use env_logger::Env;
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();
    block_on(async_main(ctx));
}

async fn async_main(mut ctx: Context) -> ! {
    log::info!("start");
    let rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let addrs = 0..7;
    let (emu, port) = emulator::Emulator::new(addrs.clone());
    rt.spawn(emu.run());
    let mut mux = Multiplexer::new(port);
    for addr in addrs {
        let dev = Device::new(addr, &mut ctx, mux.add_client(addr).unwrap());
        rt.spawn(dev.run());
    }
    assert!(ctx.registry.is_empty());
    rt.block_on(mux.run())
}
