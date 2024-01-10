use std::fs::File;
use std::io::{BufReader, Error as IoError, Read};
use std::path::Path;

use sha2::{Digest, Sha256};
use sha2::digest::consts::U32;
use sha2::digest::generic_array::GenericArray;
use sha3::Sha3_256;

macro_rules! throw {
    ($val:literal) => {
        return Err($val)?
    };
    ($($arg:tt)*) => {
        return Err(format!($($arg)*))?
    }
}

macro_rules! some_or_throw {
    ($option:expr, $msg:literal) => {
        if let Some(value) = $option {
            value
        } else {
            $crate::util::throw!($msg);
        }
    };
}

pub(crate) use throw;
pub(crate) use some_or_throw;

pub fn file_sha256<P: AsRef<Path>>(path: P) -> Result<String, IoError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let mut hasher = Sha256::new();
    let mut buffer = [0; 1024]; // Read 1024 bytes at a time

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }

        hasher.update(&buffer[..count]);
    }

    let result: GenericArray<u8, U32> = Digest::finalize(hasher);
    Ok(format!("{:x}", result))
}

pub fn sha3_256<T: AsRef<[u8]>>(input: T) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(input);
    let result = Digest::finalize(hasher);
    format!("{:x}", result)
}