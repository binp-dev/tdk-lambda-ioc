mod param;
pub mod parser;

use param::*;
use parser::*;

use ferrite::{variable::*, Context};
use std::{fmt::Debug, sync::Arc};
use thiserror::Error;
use tokio::{join, runtime};

use crate::serial::{Handle, Priority};

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
            ser_numb: Param::new("SN", epics, &format!("{}ser_numb", prefix), StringParser),
            out_ena: Param::new("OUT", epics, &format!("{}out_ena", prefix), B::default()),
            volt_real: Param::new("MV", epics, &format!("{}volt_real", prefix), NumParser),
            curr_real: Param::new("MC", epics, &format!("{}curr_real", prefix), NumParser),
            over_volt_set_point: Param::new(
                "OVP",
                epics,
                &format!("{}over_volt_set_point", prefix),
                NumParser,
            ),
            under_volt_set_point: Param::new(
                "UVL",
                epics,
                &format!("{}under_volt_set_point", prefix),
                NumParser,
            ),
            volt_set: Param::new("PV", epics, &format!("{}volt_set", prefix), NumParser),
            curr_set: Param::new("PC", epics, &format!("{}curr_set", prefix), NumParser),
        }
    }
}

pub struct Device<B: ParserBool> {
    addr: u8,
    params: Params<B>,
    serial: Handle,
}

pub type DeviceOld = Device<parser::BoolParser>;
pub type DeviceNew = Device<parser::NumParser>;

impl<B: ParserBool> Device<B> {
    pub fn new(addr: u8, epics: &mut Context, serial: Handle) -> Self {
        let prefix = format!("PS{}:", addr);
        Self {
            addr,
            serial,
            params: Params::new(epics, &prefix),
        }
    }
}

macro_rules! async_loop {
    (($($bdst:ident = $bsrc:expr),*), $code:block) => {{
        #[allow(unused_parens, unused_mut)]
        let (mut $($bdst),*) = ($($bsrc),*);
        async move {
            loop {
                $code
            }
        }
    }};
}

impl<B: ParserBool> Device<B> {
    pub async fn run(self) -> ! {
        let rt = runtime::Handle::current();
        let mut params = self.params;
        let cmdr = Arc::new(self.serial.req);

        rt.spawn(async_loop!((sig = self.serial.sig), {
            let msg = sig.recv().await.unwrap();
            log::warn!("PS{}: Signal {:?}", self.addr, msg);
        }));

        log::debug!("PS{}: Initialize", self.addr);
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

        log::debug!("PS{}: Start monitors", self.addr);
        rt.spawn(async_loop!((cmdr = cmdr.clone()), {
            params
                .out_ena
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr = cmdr.clone()), {
            params
                .volt_set
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr = cmdr.clone()), {
            params
                .curr_set
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr = cmdr.clone()), {
            params
                .over_volt_set_point
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr = cmdr.clone()), {
            params
                .under_volt_set_point
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));

        log::debug!("PS{}: Enter scan loop", self.addr);
        loop {
            join!(
                params.volt_real.read_or_log(&cmdr, Priority::Queued),
                params.curr_real.read_or_log(&cmdr, Priority::Queued),
            );
            cmdr.yield_();
        }
    }
}
