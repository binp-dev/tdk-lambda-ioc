mod param;
mod parser;

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
    NotReady,
    #[error("Unexpected response: {0}")]
    Parse(String),
}

struct Params {
    pub ser_numb: Param<ArrayVariable<u8, false, true, true>, StringParser>,
    pub out_ena: Param<Variable<u16, true, true, false>, BoolParser>,
    pub volt_real: Param<Variable<f64, false, true, true>, NumParser<f64>>,
    pub curr_real: Param<Variable<f64, false, true, true>, NumParser<f64>>,
    pub over_volt_set_point: Param<Variable<f64, true, true, false>, NumParser<f64>>,
    pub under_volt_set_point: Param<Variable<f64, true, true, false>, NumParser<f64>>,
    pub volt_set: Param<Variable<f64, true, true, false>, NumParser<f64>>,
    pub curr_set: Param<Variable<f64, true, true, false>, NumParser<f64>>,
}

impl Params {
    pub fn new(epics: &mut Context, prefix: &str) -> Self {
        let bool_parser = if prefix == "PS0:" {
            BoolParser::new("OFF".to_string(), "ON".to_string())
        } else {
            BoolParser::new("0".to_string(), "1".to_string())
        };
        Self {
            ser_numb: Param::new("SN", epics, &format!("{}ser_numb", prefix), StringParser),
            out_ena: Param::new("OUT", epics, &format!("{}out_ena", prefix), bool_parser),
            volt_real: Param::new(
                "MV",
                epics,
                &format!("{}volt_real", prefix),
                NumParser::default(),
            ),
            curr_real: Param::new(
                "MC",
                epics,
                &format!("{}curr_real", prefix),
                NumParser::default(),
            ),
            over_volt_set_point: Param::new(
                "OVP",
                epics,
                &format!("{}over_volt_set_point", prefix),
                NumParser::default(),
            ),
            under_volt_set_point: Param::new(
                "UVL",
                epics,
                &format!("{}under_volt_set_point", prefix),
                NumParser::default(),
            ),
            volt_set: Param::new(
                "PV",
                epics,
                &format!("{}volt_set", prefix),
                NumParser::default(),
            ),
            curr_set: Param::new(
                "PC",
                epics,
                &format!("{}curr_set", prefix),
                NumParser::default(),
            ),
        }
    }
}

pub struct Device {
    addr: u8,
    params: Params,
    serial: Handle,
}

impl Device {
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

impl Device {
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
