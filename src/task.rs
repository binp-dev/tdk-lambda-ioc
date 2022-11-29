use crate::{
    device::{Device, DeviceVariable, Error, Parser, ParserBool},
    interface::{Adapter, IfaceVariable, Interface},
    serial::{Priority, Signal},
    Addr,
};
use ferrite::variable::*;
use futures::FutureExt;
use tokio::{join, runtime, select, sync::watch};

async fn init_loop<T, V: VarSync, A: Adapter<T, V>, P: Parser<T>>(
    mut iface: IfaceVariable<T, V, A>,
    mut device: DeviceVariable<T, P>,
    mut on: watch::Receiver<bool>,
) -> ! {
    loop {
        let result = if *on.borrow_and_update() {
            device.read(Priority::Queued).await
        } else {
            Err(Error::NoResponse)
        };
        iface.write(result).await;

        on.changed().await.unwrap();
    }
}

async fn output_loop<T, V: VarSync, A: Adapter<T, V>, P: Parser<T>>(
    mut iface: IfaceVariable<T, V, A>,
    mut device: DeviceVariable<T, P>,
    mut on: watch::Receiver<bool>,
) -> ! {
    loop {
        let result = if *on.borrow_and_update() {
            device.read(Priority::Queued).await
        } else {
            Err(Error::NoResponse)
        };
        iface.write(result).await;

        loop {
            select! {
                biased;
                () = on.changed().map(|r| r.unwrap()) => break,
                guard = iface.read() => {
                    let value = &*guard;
                    match device.write(value, Priority::Immediate).await {
                        Ok(()) => guard.accept().await,
                        Err(err) => guard.reject(err).await,
                    }
                }
            }
        }
    }
}

async fn input<T, V: VarSync, A: Adapter<T, V>, P: Parser<T>>(
    iface: &mut IfaceVariable<T, V, A>,
    device: &mut DeviceVariable<T, P>,
) {
    iface.write(device.read(Priority::Queued).await).await;
}

macro_rules! loop_if {
    ($flag:expr, $code:block) => {
        async move {
            let mut on = $flag;
            loop {
                if *on.borrow_and_update() {
                    loop {
                        select! {
                            biased;
                            () = on.changed().map(|r| r.unwrap()) => break,
                            () = async { $code } => (),
                        }
                    }
                } else {
                    on.changed().await.unwrap()
                }
            }
        }
    };
}

pub async fn run<B: ParserBool>(addr: Addr, mut iface: Interface, mut device: Device<B>) -> ! {
    let rt = runtime::Handle::current();
    let (on_send, on) = watch::channel(false);

    rt.spawn(init_loop(iface.ser_numb, device.vars.sn, on.clone()));

    rt.spawn(output_loop(iface.out_ena, device.vars.out, on.clone()));
    rt.spawn(output_loop(iface.volt_set, device.vars.pv, on.clone()));
    rt.spawn(output_loop(iface.curr_set, device.vars.pc, on.clone()));
    rt.spawn(output_loop(
        iface.over_volt_set_point,
        device.vars.ovp,
        on.clone(),
    ));
    rt.spawn(output_loop(
        iface.under_volt_set_point,
        device.vars.uvl,
        on.clone(),
    ));

    rt.spawn(loop_if!(on, {
        join!(
            input(&mut iface.volt_real, &mut device.vars.mv),
            input(&mut iface.curr_real, &mut device.vars.mc),
        );
        device.handle.req.yield_();
    }));

    loop {
        match device.handle.sig.recv().await.unwrap() {
            Signal::On => {
                if on_send.send_replace(true) {
                    log::warn!("Device {} task is already running", addr);
                } else {
                    log::info!("Starting device {}", addr);
                }
            }
            Signal::Off => {
                if on_send.send_replace(false) {
                    log::info!("Stopping device {}", addr);
                } else {
                    log::warn!("Device {} task is already stopped", addr);
                }
            }
            Signal::Intr => {
                log::error!("Device {} sent SRQ", addr);
            }
        }
    }
}
