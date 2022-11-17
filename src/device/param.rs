use ferrite::{variable::*, Context};
use std::{fmt::Display, str::FromStr};

use super::{Error, Parser};
use crate::serial::{Commander, Priority};

pub struct Param<V: Var, P: Parser> {
    pub cmd: String,
    pub var: V,
    pub parser: P,
}

impl<V: Var, P: Parser> Param<V, P>
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
        }
    }
}

impl<V: Var, P: Parser> Param<V, P> {
    fn log_err(&self, err: Error) {
        log::error!("({}, {}) error: {}", self.cmd.clone(), self.var.name(), err);
    }
}

impl<T: Copy + FromStr, P: Parser<Item = T>, const R: bool, const A: bool>
    Param<Variable<T, R, true, A>, P>
{
    async fn read_from_device(
        &mut self,
        cmdr: &Commander,
        priority: Priority,
    ) -> Option<Result<T, String>> {
        let cmd = format!("{}?", self.cmd);
        let cmd_res = cmdr.execute(cmd, priority).await?;
        Some(self.parser.load(cmd_res))
    }

    pub async fn init(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
        let value = self
            .read_from_device(cmdr, priority)
            .await
            .ok_or(Error::NoResponse)?
            .map_err(Error::Parse)?;
        match self.var.try_acquire() {
            Some(guard) => {
                guard.write(value).await;
                Ok(())
            }
            None => Err(Error::VarNotReady),
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
            .ok_or(Error::NoResponse)?
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
        let cmd_res = cmdr
            .execute(cmd.clone(), priority)
            .await
            .ok_or(Error::NoResponse)?;
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
