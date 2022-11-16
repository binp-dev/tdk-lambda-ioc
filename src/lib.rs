#![forbid(unsafe_code)]

#[cfg(not(any(feature = "real", feature = "emul")))]
compile_error!("You need to enable either 'real' or 'emul' feature.");
#[cfg(all(feature = "real", feature = "emul"))]
compile_error!("Features 'real' and 'emul' cannot be enabled both at once.");

mod device;
#[cfg(feature = "emul")]
mod emulator;
mod serial;

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
    env_logger::Builder::from_env(Env::default().default_filter_or("trace")).init();
    block_on(async_main(ctx));
}

async fn async_main(mut ctx: Context) -> ! {
    log::info!("start");
    let rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();
    let addrs = 0..7;
    #[cfg(feature = "emul")]
    let port = {
        let (emu, port) = emulator::Emulator::new(addrs.clone());
        rt.spawn(emu.run());
        port
    };
    #[cfg(feature = "real")]
    let port = {
        use tokio_serial::SerialPortBuilderExt;
        tokio_serial::new("/dev/ttyUSB0", 19200)
            .open_native_async()
            .unwrap()
    };
    let mut mux = Multiplexer::new(port);
    for addr in addrs {
        let dev = Device::new(addr, &mut ctx, mux.add_client(addr).unwrap());
        rt.spawn(dev.run());
    }
    assert!(ctx.registry.is_empty());
    rt.block_on(mux.run())
}
