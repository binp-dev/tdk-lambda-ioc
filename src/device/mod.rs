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
    #[error("Variable isn't ready to process")]
    VarNotReady,
    #[error("No response from device")]
    NoResponse,
    #[error("Unexpected response: {0}")]
    Parse(String),
}

pub trait ParserBool: Parser<u16> + Default + Send + 'static {}
impl<P: Parser<u16> + Default + Send + 'static> ParserBool for P {}

struct Params<B: ParserBool> {
    pub ser_numb: Param<String, StringParser, ArrayVariable<u8, false, true, true>>,
    pub out_ena: Param<u16, B, Variable<u16, true, true, false>>,
    pub volt_real: Param<f64, NumParser, Variable<f64, false, true, true>>,
    pub curr_real: Param<f64, NumParser, Variable<f64, false, true, true>>,
    pub over_volt_set_point: Param<f64, NumParser, Variable<f64, true, true, false>>,
    pub under_volt_set_point: Param<f64, NumParser, Variable<f64, true, true, false>>,
    pub volt_set: Param<f64, NumParser, Variable<f64, true, true, false>>,
    pub curr_set: Param<f64, NumParser, Variable<f64, true, true, false>>,
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
        #[allow(unused_parens)]
        let ($($bdst),*) = ($($bsrc.clone()),*);
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

        rt.spawn(async_loop!((intr = self.serial.intr), {
            intr.notified().await;
            log::warn!("PS{}: Interrupt caught!", self.addr);
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
        rt.spawn(async_loop!((cmdr = cmdr), {
            params
                .out_ena
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr = cmdr), {
            params
                .volt_set
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr = cmdr), {
            params
                .curr_set
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr = cmdr), {
            params
                .over_volt_set_point
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr = cmdr), {
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
