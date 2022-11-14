use std::{
    fmt::Debug,
    ops::Deref,
    str::{FromStr, Utf8Error},
    string::ToString,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error<P> {
    #[error("Bytes to string failed")]
    Utf8(#[from] Utf8Error),
    #[error("Parse failed")]
    Parse(P),
}

pub trait ParseBytes {
    fn parse_bytes<T: FromStr>(&self) -> Result<T, Error<T::Err>>;
}
impl<B: Deref<Target = [u8]>> ParseBytes for B {
    fn parse_bytes<T: FromStr>(&self) -> Result<T, Error<T::Err>> {
        std::str::from_utf8(self)?
            .parse()
            .map_err(|e| Error::Parse(e))
    }
}

pub trait ToBytes {
    fn to_bytes(&self) -> Vec<u8>;
}
impl<T: ToString> ToBytes for T {
    fn to_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }
}

pub mod prelude {
    pub use super::{ParseBytes, ToBytes};
}
