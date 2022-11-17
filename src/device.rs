use ferrite::{variable::*, Context};
use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
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
    #[error("Unexpected response")]
    Parse(String),
}

struct Param<V: Var, P: Parser> {
    pub cmd: String,
    pub var: V,
    pub parser: P,
}

impl<V: Var, P: Parser> Param<V, P>
where
    AnyVariable: Downcast<V>,
{
    fn new(cmd: &str, epics: &mut Context, name: &str, parser: P) -> Self {
        log::trace!("parameter: {}", name);
        let any = epics
            .registry
            .remove(name)
            .unwrap_or_else(|| panic!("No such name: {}", name));
        let info = any.info();
        let var = any
            .downcast()
            .unwrap_or_else(|| panic!("Bad type, {:?} expected", info));
        Self {
            cmd: String::from(cmd),
            var,
            parser,
        }
    }
}

impl<V: Var, P: Parser> Param<V, P> {
    fn log_err(&self, err: Error) {
        log::error!(
            "({}, {}) error: {:?}",
            self.cmd.clone(),
            self.var.name(),
            err
        );
    }
}

impl<T: Copy + FromStr, P: Parser<Item = T>, const R: bool, const A: bool>
    Param<Variable<T, R, true, A>, P>
{
    async fn read_from_device(
        &mut self,
        cmdr: &Commander,
        priority: Priority,
    ) -> Result<T, String> {
        let cmd = format!("{}?", self.cmd);
        let cmd_res = cmdr.execute(cmd, priority).await;
        self.parser.load(cmd_res)
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

impl<T: Copy + FromStr, P: Parser<Item = T>, const R: bool> Param<Variable<T, R, true, true>, P> {
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

impl<T: Copy + Display, P: Parser<Item = T>, const W: bool, const A: bool>
    Param<Variable<T, true, W, A>, P>
{
    pub async fn write(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let value = self.var.acquire().await.read().await;
        let cmd = format!("{} {}", self.cmd, self.parser.store(value));
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

impl<P: Parser<Item = String>, const R: bool> Param<ArrayVariable<u8, R, true, true>, P> {
    pub async fn read(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let cmd = format!("{}?", self.cmd);
        let value = self
            .parser
            .load(cmdr.execute(cmd, priority).await)
            .map_err(Error::Parse)?;
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

trait Parser {
    type Item;
    fn load(&self, text: String) -> Result<Self::Item, String>;
    fn store(&self, value: Self::Item) -> String;
}

#[derive(Debug, Clone)]
struct NumParser<T: FromStr + Display> {
    _p: PhantomData<T>,
}
impl<T: FromStr + Display> Default for NumParser<T> {
    fn default() -> Self {
        Self { _p: PhantomData }
    }
}
impl<T: FromStr + Display> Parser for NumParser<T> {
    type Item = T;
    fn load(&self, text: String) -> Result<Self::Item, String> {
        text.parse::<T>().map_err(|_| text)
    }
    fn store(&self, value: Self::Item) -> String {
        format!("{}", value)
    }
}

#[derive(Debug, Clone)]
struct BoolParser {
    false_: String,
    true_: String,
}
impl BoolParser {
    fn new(false_: String, true_: String) -> Self {
        Self { false_, true_ }
    }
}
impl Parser for BoolParser {
    type Item = u16;
    fn load(&self, text: String) -> Result<Self::Item, String> {
        if text == self.false_ {
            Ok(0)
        } else if text == self.true_ {
            Ok(1)
        } else {
            Err(text)
        }
    }
    fn store(&self, value: Self::Item) -> String {
        if value == 0 {
            self.false_.clone()
        } else {
            self.true_.clone()
        }
    }
}

#[derive(Debug, Clone)]
struct StringParser;
impl Parser for StringParser {
    type Item = String;
    fn load(&self, text: String) -> Result<Self::Item, String> {
        Ok(text)
    }
    fn store(&self, value: Self::Item) -> String {
        value
    }
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
