use crate::{
    serial::{Handle, Priority},
    utils::prelude::*,
};
use ferrite::*;
use std::sync::Arc;
use tokio::{join, runtime};

struct Vars {
    pub ser_numb: ArrayVariable<u8, false, true, true>,
    pub volt_real: Variable<f64, false, true, true>,
    pub curr_real: Variable<f64, false, true, true>,
    pub over_volt_set_point: Variable<f64, true, true, false>,
    pub under_volt_set_point: Variable<f64, true, true, false>,
    pub volt_set: Variable<f64, true, true, false>,
    pub curr_set: Variable<f64, true, true, false>,
}

fn take_var<V>(ctx: &mut Context, name: &str) -> V
where
    AnyVariable: Downcast<V>,
{
    log::debug!("take: {}", name);
    let var = ctx
        .registry
        .remove(name)
        .expect(&format!("No such name: {}", name));
    let info = var.info();
    var.downcast()
        .expect(&format!("Bad type, {:?} expected", info))
}

impl Vars {
    pub fn new(prefix: &str, ctx: &mut Context) -> Self {
        Self {
            ser_numb: take_var(ctx, &format!("{}ser_numb", prefix)),
            volt_real: take_var(ctx, &format!("{}volt_real", prefix)),
            curr_real: take_var(ctx, &format!("{}curr_real", prefix)),
            over_volt_set_point: take_var(ctx, &format!("{}over_volt_set_point", prefix)),
            under_volt_set_point: take_var(ctx, &format!("{}under_volt_set_point", prefix)),
            volt_set: take_var(ctx, &format!("{}volt_set", prefix)),
            curr_set: take_var(ctx, &format!("{}curr_set", prefix)),
        }
    }
}

pub struct Device {
    addr: u8,
    vars: Vars,
    serial: Arc<Handle>,
}

impl Device {
    pub fn new(addr: u8, epics: &mut Context, serial: Handle) -> Self {
        let prefix = format!("PS{}:", addr);
        Self {
            addr,
            serial: Arc::new(serial),
            vars: Vars::new(&prefix, epics),
        }
    }

    pub async fn run(mut self) -> ! {
        let rt = runtime::Handle::current();

        log::info!("PS{}: Initialize", self.addr);

        join!(
            async {
                let value = self
                    .serial
                    .req
                    .run(Vec::from("SN?".as_bytes()), Priority::Queued)
                    .await;
                self.vars
                    .ser_numb
                    .request()
                    .await
                    .write_from_slice(&value)
                    .await;
            },
            async {
                let value = self
                    .serial
                    .req
                    .run(Vec::from("PV?".as_bytes()), Priority::Queued)
                    .await
                    .parse_bytes::<f64>()
                    .unwrap();
                self.vars.volt_set.try_acquire().unwrap().write(value).await;
            },
            async {
                let value = self
                    .serial
                    .req
                    .run(Vec::from("PC?".as_bytes()), Priority::Queued)
                    .await
                    .parse_bytes::<f64>()
                    .unwrap();
                self.vars.curr_set.try_acquire().unwrap().write(value).await;
            }
        );
        self.serial.req.yield_();

        log::info!("PS{}: Start monitors", self.addr);

        let ser = self.serial.clone();
        rt.spawn(async move {
            loop {
                let value = self.vars.volt_set.acquire().await.read().await;
                assert_eq!(
                    ser.req
                        .run(format!("PV {}", value).into_bytes(), Priority::Immediate)
                        .await,
                    b"OK"
                );
            }
        });
        let ser = self.serial.clone();
        rt.spawn(async move {
            loop {
                let value = self.vars.curr_set.acquire().await.read().await;
                assert_eq!(
                    ser.req
                        .run(format!("PC {}", value).into_bytes(), Priority::Immediate)
                        .await,
                    b"OK"
                );
            }
        });

        log::info!("PS{}: Start scan loop", self.addr);

        loop {
            {
                let value = self
                    .serial
                    .req
                    .run(Vec::from("MV?".as_bytes()), Priority::Queued)
                    .await
                    .parse_bytes::<f64>()
                    .unwrap();
                self.vars.volt_real.request().await.write(value).await;
            }
            {
                let value = self
                    .serial
                    .req
                    .run(Vec::from("MC?".as_bytes()), Priority::Queued)
                    .await
                    .parse_bytes::<f64>()
                    .unwrap();
                self.vars.curr_real.request().await.write(value).await;
            }
            self.serial.req.yield_();
        }
    }
}
