#![forbid(unsafe_code)]

#[cfg(not(any(feature = "tcp", feature = "serial", feature = "emulator")))]
compile_error!("You need to enable either 'tcp',  'serial' or 'emulator' feature.");
#[cfg(all(feature = "tcp", feature = "serial", feature = "emulator"))]
compile_error!("Features 'tcp', 'serial' and 'emulator' cannot be enabled both at once.");

mod device;
#[cfg(feature = "emulator")]
mod emulator;
mod interface;
#[cfg(feature = "tcp")]
mod net;
mod serial;
mod task;

/// *Export symbols being called from IOC.*
pub use ferrite::export;

use ferrite::{entry_point, Context};
use macro_rules_attribute::apply;
use tokio::runtime;

use crate::{
    device::{DeviceNew, DeviceOld},
    interface::Interface,
    serial::Multiplexer,
};

pub type Addr = u8;

#[apply(entry_point)]
fn app_main(mut ctx: Context) {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let rt = runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    rt.block_on(async_main(ctx));
}

async fn async_main(mut ctx: Context) -> ! {
    log::info!("start");
    let rt = runtime::Handle::current();

    let addrs_old = [0];
    let addrs_new = 1..7;

    #[cfg(feature = "serial")]
    let port = {
        use tokio_serial::SerialPortBuilderExt;
        tokio_serial::new("/dev/ttyUSB0", 19200)
            .open_native_async()
            .unwrap()
    };

    #[cfg(feature = "tcp")]
    let port = net::PersistentTcpStream::connect("10.0.0.79:4001")
        .await
        .unwrap();

    #[cfg(feature = "emulator")]
    let port = {
        let (emu, port) =
            emulator::Emulator::new(addrs_old.into_iter().chain(addrs_new.clone().into_iter()));
        rt.spawn(emu.run());
        port
    };

    let mut mux = Multiplexer::new(port);

    for addr in addrs_old {
        rt.spawn(task::run(
            addr,
            Interface::new(&mut ctx, addr),
            DeviceOld::new(mux.add_client(addr).unwrap()),
        ));
    }
    for addr in addrs_new {
        rt.spawn(task::run(
            addr,
            Interface::new(&mut ctx, addr),
            DeviceNew::new(mux.add_client(addr).unwrap()),
        ));
    }

    assert!(ctx.registry.is_empty());
    rt.spawn(mux.run()).await.unwrap()
}
