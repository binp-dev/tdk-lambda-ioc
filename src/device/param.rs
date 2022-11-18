use ferrite::{variable::*, Context};
use std::{fmt::Display, str::FromStr};

use super::{Error, Parser};
use crate::serial::{Commander, Priority};

pub struct Param<T, P: Parser<T>, V: Var> {
    cmd: String,
    var: V,
    parser: P,
    value: Option<T>,
}

impl<T, P: Parser<T>, V: Var> Param<T, P, V>
where
    AnyVariable: Downcast<V>,
{
    pub fn new(cmd: &str, epics: &mut Context, name: &str, parser: P) -> Self {
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
            value: None,
        }
    }
}

impl<T, P: Parser<T>, V: Var> Param<T, P, V> {
    fn log_err(&self, err: Error) {
        log::error!("({}, {}) error: {}", self.cmd.clone(), self.var.name(), err);
    }
}

impl<T: Copy + FromStr, P: Parser<T>, const R: bool, const A: bool>
    Param<T, P, Variable<T, R, true, A>>
{
    async fn read_from_device(&mut self, cmdr: &Commander, priority: Priority) -> Result<T, Error> {
        let cmd = format!("{}?", self.cmd);
        let cmd_res = cmdr.execute(cmd, priority).await.ok_or(Error::NoResponse)?;
        self.parser.load(cmd_res).map_err(Error::Parse)
    }

    pub async fn init(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let val_res = self.read_from_device(cmdr, priority).await;
        match self.var.try_acquire() {
            Some(var) => match val_res {
                Ok(value) => {
                    self.value.replace(value);
                    var.write(value).await;
                    Ok(())
                }
                Err(err) => {
                    var.reject(&format!("{}", err)).await;
                    Err(err)
                }
            },
            None => Err(Error::VarNotReady),
        }
    }

    pub async fn init_or_log(&mut self, cmdr: &Commander, priority: Priority) {
        if let Err(e) = self.init(cmdr, priority).await {
            self.log_err(e);
        }
    }
}

impl<T: Copy + FromStr, P: Parser<T>, const R: bool> Param<T, P, Variable<T, R, true, true>> {
    pub async fn read(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let val_res = self.read_from_device(cmdr, priority).await;
        let var = self.var.request().await;
        match val_res {
            Ok(value) => {
                self.value.replace(value);
                var.write(value).await;
                Ok(())
            }
            Err(err) => {
                var.reject(&format!("{}", err)).await;
                Err(err)
            }
        }
    }

    pub async fn read_or_log(&mut self, cmdr: &Commander, priority: Priority) {
        if let Err(e) = self.read(cmdr, priority).await {
            self.log_err(e);
        }
    }
}

impl<T: Copy + Display, P: Parser<T>, const A: bool> Param<T, P, Variable<T, true, true, A>> {
    pub async fn write(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let mut var = self.var.acquire().await;
        let value = *var;
        let cmd = format!("{} {}", self.cmd, self.parser.store(value));
        match cmdr
            .execute(cmd.clone(), priority)
            .await
            .ok_or(Error::NoResponse)
            .and_then(|cmd_res| match cmd_res.as_str() {
                "OK" => Ok(()),
                _ => Err(Error::Parse(cmd_res)),
            }) {
            Ok(()) => {
                self.value.replace(value);
                var.accept().await;
                Ok(())
            }
            Err(err) => {
                if let Some(value) = self.value {
                    *var = value;
                }
                var.reject(&format!("{}", err)).await;
                Err(err)
            }
        }
    }

    pub async fn write_or_log(&mut self, cmdr: &Commander, priority: Priority) {
        if let Err(e) = self.write(cmdr, priority).await {
            self.log_err(e);
        }
    }
}

impl<P: Parser<String>, const R: bool> Param<String, P, ArrayVariable<u8, R, true, true>> {
    pub async fn read(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let cmd = format!("{}?", self.cmd);
        let value = self
            .parser
            .load(cmdr.execute(cmd, priority).await.ok_or(Error::NoResponse)?)
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
