use super::{Error, Parser};
use crate::serial::{Commander, Priority};
use ferrite::{variable::*, Context};
use std::{fmt::Display, marker::PhantomData, str::FromStr, sync::Arc};

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

impl<T: Copy + FromStr, P: Parser<T>> Param<T, P, Variable<T>> {
    async fn read_from_device(&mut self, cmdr: &Commander, priority: Priority) -> Result<T, Error> {
        let cmd = format!("{}?", self.cmd);
        let cmd_res = cmdr.execute(cmd, priority).await.ok_or(Error::NoResponse)?;
        self.parser.load(cmd_res).map_err(Error::Parse)
    }

    pub async fn init(&mut self, cmdr: &Commander, priority: Priority) -> Result<(), Error> {
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

    pub async fn init_or_log(&mut self, cmdr: &Commander, priority: Priority) {
        if let Err(e) = self.init(cmdr, priority).await {
            self.log_err(e);
        }
    }
}

impl<T: Copy + FromStr, P: Parser<T>> Param<T, P, Variable<T>> {
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

impl<T: Copy + Display, P: Parser<T>> Param<T, P, Variable<T>> {
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

impl<P: Parser<String>> Param<String, P, ArrayVariable<u8>> {
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

pub struct DeviceVariable<T, P: Parser<T>> {
    cmdr: Arc<Commander>,
    name: String,
    parser: P,
    _p: PhantomData<T>,
}

impl<T: Copy + FromStr, P: Parser<T>> DeviceVariable<T, P> {
    async fn read(&mut self, priority: Priority) -> Result<T, Error> {
        let cmd = format!("{}?", self.name);
        let cmd_res = self
            .cmdr
            .execute(cmd, priority)
            .await
            .ok_or(Error::NoResponse)?;
        self.parser.load(cmd_res).map_err(Error::Parse)
    }
}

impl<T: Copy + Display, P: Parser<T>> DeviceVariable<T, P> {
    pub async fn write(&mut self, value: T, priority: Priority) -> Result<(), Error> {
        self.cmdr
            .execute(
                format!("{} {}", self.name, self.parser.store(value)),
                priority,
            )
            .await
            .ok_or(Error::NoResponse)
            .and_then(|cmd_res| match cmd_res.as_str() {
                "OK" => Ok(()),
                _ => Err(Error::Parse(cmd_res)),
            })
    }
}

pub struct Binding<T, P: Parser<T>, V: Var> {
    front: V,
    back: DeviceVariable<T, P>,
    value: Option<T>,
}
