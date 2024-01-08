use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use sha2::{Digest, Sha256};
use sha2::digest::consts::U32;
use sha2::digest::generic_array::GenericArray;

pub fn file_sha256<P: AsRef<Path>>(path: P) -> Result<String, std::io::Error> {
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