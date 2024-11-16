use std::{array::TryFromSliceError, convert::Infallible, string::FromUtf8Error};

use derive_more::{Display, From};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Display, Debug, From)]
pub enum Error {
    InvalidFileAttributeCode,
    InvalidFailureReason,
    InvalidConnectionType,
    UnexpectedResponseCode,

    #[from]
    Io(std::io::Error),

    #[from]
    Iced(iced::Error),

    #[from]
    FromSlice(TryFromSliceError),

    #[from]
    FromUTF8(FromUtf8Error),

    #[from]
    FromInfallible(Infallible),
}

impl std::error::Error for Error {}
