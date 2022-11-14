use ferrite::{variable::*, Context};
use std::{
    fmt::{Debug, Display},
    str::FromStr,
    sync::Arc,
};
use thiserror::Error;
use tokio::{join, runtime};

use crate::serial::{Commander, Handle, Priority};

#[derive(Error, Debug)]
enum Error {
    #[error("Variable isn't ready to process")]
    NotReady,
    #[error("UNexpected response")]
    Parse(String),
}

struct Param<V: Var> {
    pub cmd: String,
    pub var: V,
}

impl<V: Var> Param<V>
where
    AnyVariable: Downcast<V>,
{
    fn new(cmd: &str, epics: &mut Context, name: &str) -> Self {
        log::trace!("parameter: {}", name);
        let any = epics
            .registry
            .remove(name)
            .expect(&format!("No such name: {}", name));
        let info = any.info();
        let var = any
            .downcast()
            .expect(&format!("Bad type, {:?} expected", info));
        Self {
            cmd: String::from(cmd),
            var,
        }
    }
}

impl<V: Var> Param<V> {
    fn log_err(&self, err: Error) {
        log::error!(
            "({}, {}) error: {:?}",
            self.cmd.clone(),
            self.var.name(),
            err
        );
    }
}

impl<T: Copy + FromStr, const R: bool, const A: bool> Param<Variable<T, R, true, A>> {
    async fn read_from_device(
        &mut self,
        cmdr: &Commander,
        priority: Priority,
    ) -> Result<T, String> {
        let cmd = format!("{}?", self.cmd);
        let cmd_res = cmdr.execute(cmd, priority).await;
        cmd_res.parse::<T>().map_err(|_| cmd_res)
    }

    pub async fn init(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let value = self
            .read_from_device(cmdr, priority)
            .await
            .map_err(Error::Parse)?;
        match self.var.try_acquire() {
            Some(guard) => {
                guard.write(value).await;
                Ok(())
            }
            None => Err(Error::NotReady),
        }
    }

    pub async fn init_or_log(&mut self, cmdr: &Commander, priority: Priority) {
        if let Err(e) = self.init(cmdr, priority).await {
            self.log_err(e);
        }
    }
}

impl<T: Copy + FromStr, const R: bool> Param<Variable<T, R, true, true>> {
    pub async fn read(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let value = self
            .read_from_device(cmdr, priority)
            .await
            .map_err(Error::Parse)?;
        self.var.request().await.write(value).await;
        Ok(())
    }

    pub async fn read_or_log(&mut self, cmdr: &Commander, priority: Priority) {
        if let Err(e) = self.read(cmdr, priority).await {
            self.log_err(e);
        }
    }
}

impl<T: Copy + Display, const W: bool, const A: bool> Param<Variable<T, true, W, A>> {
    pub async fn write(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let value = self.var.acquire().await.read().await;
        let cmd = format!("{} {}", self.cmd, value);
        let cmd_res = cmdr.execute(cmd.clone(), priority).await;
        match cmd_res.as_str() {
            "OK" => Ok(()),
            _ => Err(Error::Parse(cmd_res)),
        }
    }

    pub async fn write_or_log(&mut self, cmdr: &Commander, priority: Priority) {
        if let Err(e) = self.write(cmdr, priority).await {
            self.log_err(e);
        }
    }
}

impl<const R: bool> Param<ArrayVariable<u8, R, true, true>> {
    pub async fn read(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let cmd = format!("{}?", self.cmd);
        let value = cmdr.execute(cmd, priority).await;
        self.var
            .request()
            .await
            .write_from_slice(value.as_bytes())
            .await;
        Ok(())
    }

    pub async fn read_or_log(&mut self, cmdr: &Commander, priority: Priority) {
        if let Err(e) = self.read(cmdr, priority).await {
            self.log_err(e);
        }
    }
}

struct Params {
    pub ser_numb: Param<ArrayVariable<u8, false, true, true>>,
    pub out_ena: Param<Variable<u16, true, true, false>>,
    pub volt_real: Param<Variable<f64, false, true, true>>,
    pub curr_real: Param<Variable<f64, false, true, true>>,
    pub over_volt_set_point: Param<Variable<f64, true, true, false>>,
    pub under_volt_set_point: Param<Variable<f64, true, true, false>>,
    pub volt_set: Param<Variable<f64, true, true, false>>,
    pub curr_set: Param<Variable<f64, true, true, false>>,
}

impl Params {
    pub fn new(epics: &mut Context, prefix: &str) -> Self {
        Self {
            ser_numb: Param::new("SN", epics, &format!("{}ser_numb", prefix)),
            out_ena: Param::new("OUT", epics, &format!("{}out_ena", prefix)),
            volt_real: Param::new("MV", epics, &format!("{}volt_real", prefix)),
            curr_real: Param::new("MC", epics, &format!("{}curr_real", prefix)),
            over_volt_set_point: Param::new(
                "OVP",
                epics,
                &format!("{}over_volt_set_point", prefix),
            ),
            under_volt_set_point: Param::new(
                "UVL",
                epics,
                &format!("{}under_volt_set_point", prefix),
            ),
            volt_set: Param::new("PV", epics, &format!("{}volt_set", prefix)),
            curr_set: Param::new("PC", epics, &format!("{}curr_set", prefix)),
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
    (($($vars:ident),*), $code:block) => {{
        #[allow(unused_parens)]
        let ($($vars),*) = ($($vars.clone()),*);
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
        rt.spawn(async_loop!((cmdr), {
            params
                .out_ena
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr), {
            params
                .volt_set
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr), {
            params
                .curr_set
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr), {
            params
                .over_volt_set_point
                .write_or_log(&cmdr, Priority::Immediate)
                .await;
        }));
        rt.spawn(async_loop!((cmdr), {
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
