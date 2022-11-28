use crate::serial::{Commander, Priority, SerialHandle};
use std::{fmt::Debug, fmt::Display, marker::PhantomData, str::FromStr, sync::Arc};
use thiserror::Error;

pub trait ParserBool: Parser<u16> + Default + Send + 'static {}
impl<P: Parser<u16> + Default + Send + 'static> ParserBool for P {}

pub struct DeviceVars<B: ParserBool> {
    /// Serial number
    pub sn: DeviceVariable<String, StringParser>,
    /// Output enabled
    pub out: DeviceVariable<u16, B>,
    /// Measured voltage
    pub mv: DeviceVariable<f64, NumParser>,
    /// Measured current
    pub mc: DeviceVariable<f64, NumParser>,
    /// Over-voltage protection
    pub ovp: DeviceVariable<f64, NumParser>,
    /// Under-voltage limit
    pub uvl: DeviceVariable<f64, NumParser>,
    /// Put voltage
    pub pv: DeviceVariable<f64, NumParser>,
    /// Put current
    pub pc: DeviceVariable<f64, NumParser>,
}

impl<B: ParserBool> DeviceVars<B> {
    pub fn new(cmdr: Arc<Commander>) -> Self {
        let this = Self {
            sn: DeviceVariable::new(cmdr.clone(), "SN"),
            out: DeviceVariable::new(cmdr.clone(), "OUT"),
            mv: DeviceVariable::new(cmdr.clone(), "MV"),
            mc: DeviceVariable::new(cmdr.clone(), "MC"),
            ovp: DeviceVariable::new(cmdr.clone(), "OVP"),
            uvl: DeviceVariable::new(cmdr.clone(), "UVL"),
            pv: DeviceVariable::new(cmdr.clone(), "PV"),
            pc: DeviceVariable::new(cmdr.clone(), "PC"),
        };
        drop(cmdr);
        this
    }
}

pub struct Device<B: ParserBool> {
    pub vars: DeviceVars<B>,
    pub handle: SerialHandle,
}

impl<B: ParserBool> Device<B> {
    pub fn new(handle: SerialHandle) -> Self {
        Self {
            vars: DeviceVars::new(handle.req.clone()),
            handle,
        }
    }
}

pub type DeviceOld = Device<BoolParser>;
pub type DeviceNew = Device<NumParser>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("No response from device")]
    NoResponse,
    #[error("Unexpected response: {0}")]
    Parse(String),
}

pub trait Parser<T>: Default {
    fn load(&self, text: String) -> Result<T, String>;
    fn store(&self, value: &T) -> String;
}

#[derive(Debug, Clone, Default)]
pub struct NumParser;
impl<T: FromStr + Display> Parser<T> for NumParser {
    fn load(&self, text: String) -> Result<T, String> {
        text.parse::<T>().map_err(|_| text)
    }
    fn store(&self, value: &T) -> String {
        format!("{}", value)
    }
}

#[derive(Debug, Clone, Default)]
pub struct BoolParser;
impl Parser<u16> for BoolParser {
    fn load(&self, text: String) -> Result<u16, String> {
        match text.as_str() {
            "OFF" => Ok(0),
            "ON" => Ok(1),
            _ => Err(text),
        }
    }
    fn store(&self, value: &u16) -> String {
        if *value == 0 { "OFF" } else { "ON" }.to_string()
    }
}

#[derive(Debug, Clone, Default)]
pub struct StringParser;
impl Parser<String> for StringParser {
    fn load(&self, text: String) -> Result<String, String> {
        Ok(text)
    }
    fn store(&self, value: &String) -> String {
        value.clone()
    }
}

/// Variable at the device side.
/// Can be read and written.
pub struct DeviceVariable<T, P: Parser<T>> {
    cmdr: Arc<Commander>,
    name: String,
    parser: P,
    _p: PhantomData<T>,
}

impl<T, P: Parser<T>> DeviceVariable<T, P> {
    pub fn new(cmdr: Arc<Commander>, name: &str) -> Self {
        Self {
            cmdr,
            name: String::from(name),
            parser: P::default(),
            _p: PhantomData,
        }
    }

    pub async fn read(&mut self, priority: Priority) -> Result<T, Error> {
        let cmd = format!("{}?", self.name);
        let cmd_res = self
            .cmdr
            .execute(cmd, priority)
            .await
            .ok_or(Error::NoResponse)?;
        self.parser.load(cmd_res).map_err(Error::Parse)
    }

    pub async fn write(&mut self, value: &T, priority: Priority) -> Result<(), Error> {
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
