use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    str::FromStr,
};

pub trait Parser {
    type Item;
    fn load(&self, text: String) -> Result<Self::Item, String>;
    fn store(&self, value: Self::Item) -> String;
}

#[derive(Debug, Clone)]
pub struct NumParser<T: FromStr + Display> {
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
pub struct BoolParser {
    false_: String,
    true_: String,
}
impl BoolParser {
    pub fn new(false_: String, true_: String) -> Self {
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
pub struct StringParser;
impl Parser for StringParser {
    type Item = String;
    fn load(&self, text: String) -> Result<Self::Item, String> {
        Ok(text)
    }
    fn store(&self, value: Self::Item) -> String {
        value
    }
}
