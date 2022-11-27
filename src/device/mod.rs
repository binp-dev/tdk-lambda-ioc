mod param;
pub mod parser;

use param::*;
use parser::*;

use ferrite::{variable::*, Context};
use std::{fmt::Debug, sync::Arc};
use thiserror::Error;
use tokio::{join, runtime, select, sync::Notify, task::JoinHandle};

use crate::serial::{Commander, Handle, Priority, Signal};

#[derive(Error, Debug)]
pub enum Error {
    #[error("No response from device")]
    NoResponse,
    #[error("Unexpected response: {0}")]
    Parse(String),
}

pub trait ParserBool: Parser<u16> + Default + Send + 'static {}
impl<P: Parser<u16> + Default + Send + 'static> ParserBool for P {}

struct Params<B: ParserBool> {
    pub ser_numb: Param<String, StringParser, ArrayVariable<u8>>,
    pub out_ena: Param<u16, B, Variable<u16>>,
    pub volt_real: Param<f64, NumParser, Variable<f64>>,
    pub curr_real: Param<f64, NumParser, Variable<f64>>,
    pub over_volt_set_point: Param<f64, NumParser, Variable<f64>>,
    pub under_volt_set_point: Param<f64, NumParser, Variable<f64>>,
    pub volt_set: Param<f64, NumParser, Variable<f64>>,
    pub curr_set: Param<f64, NumParser, Variable<f64>>,
}

impl<B: ParserBool> Params<B> {
    pub fn new(epics: &mut Context, prefix: &str) -> Self {
        Self {
            ser_numb: Param::new("SN", epics, &format!("{}:ser_numb", prefix), StringParser),
            out_ena: Param::new("OUT", epics, &format!("{}:out_ena", prefix), B::default()),
            volt_real: Param::new("MV", epics, &format!("{}:volt_real", prefix), NumParser),
            curr_real: Param::new("MC", epics, &format!("{}:curr_real", prefix), NumParser),
            over_volt_set_point: Param::new(
                "OVP",
                epics,
                &format!("{}:over_volt_set_point", prefix),
                NumParser,
            ),
            under_volt_set_point: Param::new(
                "UVL",
                epics,
                &format!("{}:under_volt_set_point", prefix),
                NumParser,
            ),
            volt_set: Param::new("PV", epics, &format!("{}:volt_set", prefix), NumParser),
            curr_set: Param::new("PC", epics, &format!("{}:curr_set", prefix), NumParser),
        }
    }
}

pub struct Device<B: ParserBool> {
    name: String,
    params: Params<B>,
    serial: Handle,
}

pub type DeviceOld = Device<parser::BoolParser>;
pub type DeviceNew = Device<parser::NumParser>;

impl<B: ParserBool> Device<B> {
    pub fn new(addr: u8, epics: &mut Context, serial: Handle) -> Self {
        let name = format!("PS{}", addr);
        Self {
            params: Params::new(epics, &name),
            serial,
            name,
        }
    }
}

macro_rules! async_loop {
    ($($dst:ident = $src:expr,)* $body:block) => {
        async {
            $( let $dst = $src; )*
            loop {
                $body
            }
        }
    };
}

enum DeviceState<P> {
    Running(JoinHandle<P>),
    Stopped(P),
}

impl<B: ParserBool> Device<B> {
    async fn scan_loop(params: &mut Params<B>, cmdr: Arc<Commander>) {
        join!(
            params.ser_numb.read_or_log(&cmdr, Priority::Queued),
            params.out_ena.init_or_log(&cmdr, Priority::Queued),
            params.volt_set.init_or_log(&cmdr, Priority::Queued),
            params.curr_set.init_or_log(&cmdr, Priority::Queued),
            params
                .over_volt_set_point
                .init_or_log(&cmdr, Priority::Queued),
            params
                .under_volt_set_point
                .init_or_log(&cmdr, Priority::Queued),
        );
        cmdr.yield_();

        join!(
            async_loop!({
                params
                    .out_ena
                    .write_or_log(&cmdr, Priority::Immediate)
                    .await;
            }),
            async_loop!({
                params
                    .volt_set
                    .write_or_log(&cmdr, Priority::Immediate)
                    .await;
            }),
            async_loop!({
                params
                    .curr_set
                    .write_or_log(&cmdr, Priority::Immediate)
                    .await;
            }),
            async_loop!({
                params
                    .over_volt_set_point
                    .write_or_log(&cmdr, Priority::Immediate)
                    .await;
            }),
            async_loop!({
                params
                    .under_volt_set_point
                    .write_or_log(&cmdr, Priority::Immediate)
                    .await;
            }),
            async_loop!({
                join!(
                    params.volt_real.read_or_log(&cmdr, Priority::Queued),
                    params.curr_real.read_or_log(&cmdr, Priority::Queued),
                );
                cmdr.yield_();
            })
        );
    }

    pub async fn run(self) -> ! {
        let rt = runtime::Handle::current();
        let cmdr = Arc::new(self.serial.req);
        let done = Arc::new(Notify::new());

        let mut state = DeviceState::Stopped(self.params);

        let mut sig = self.serial.sig;
        loop {
            match sig.recv().await.unwrap() {
                Signal::On => match state {
                    DeviceState::Stopped(mut params) => {
                        let done = done.clone();
                        let cmdr = cmdr.clone();
                        let name = self.name.clone();
                        state = DeviceState::Running(rt.spawn(async move {
                            log::info!("{}: Running", name);
                            select! {
                                biased;
                                () = done.notified() => (),
                                () = Self::scan_loop(
                                    &mut params,
                                    cmdr,
                                ) => (),
                            }
                            log::info!("{}: Stopped", name);
                            params
                        }));
                    }
                    DeviceState::Running(_) => {
                        log::warn!("{}: Already running", self.name);
                    }
                },
                Signal::Off => match state {
                    DeviceState::Running(jh) => {
                        done.notify_waiters();
                        state = DeviceState::Stopped(jh.await.unwrap());
                    }
                    DeviceState::Stopped(..) => {
                        log::warn!("{}: Already stopped", self.name);
                    }
                },
                Signal::Intr => {
                    log::warn!("{}: Interrupt caught", self.name);
                }
            }
        }
    }
}
