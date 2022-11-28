use crate::serial::{Commander, Priority};
use std::{fmt::Debug, fmt::Display, marker::PhantomData, str::FromStr, sync::Arc};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("No response from device")]
    NoResponse,
    #[error("Unexpected response: {0}")]
    Parse(String),
}

pub trait Parser<T> {
    fn load(&self, text: String) -> Result<T, String>;
    fn store(&self, value: T) -> String;
}

#[derive(Debug, Clone, Default)]
pub struct NumParser;
impl<T: FromStr + Display> Parser<T> for NumParser {
    fn load(&self, text: String) -> Result<T, String> {
        text.parse::<T>().map_err(|_| text)
    }
    fn store(&self, value: T) -> String {
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
    fn store(&self, value: u16) -> String {
        if value == 0 { "OFF" } else { "ON" }.to_string()
    }
}

#[derive(Debug, Clone, Default)]
pub struct StringParser;
impl Parser<String> for StringParser {
    fn load(&self, text: String) -> Result<String, String> {
        Ok(text)
    }
    fn store(&self, value: String) -> String {
        value
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

impl<T: Copy, P: Parser<T>> DeviceVariable<T, P> {
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

impl<T: Copy, P: Parser<T>> DeviceVariable<T, P> {
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
