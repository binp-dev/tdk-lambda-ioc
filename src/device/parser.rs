use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

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
