/// Tools for working with Looking Glass "holographic" displays

use bytes::{Bytes, BytesMut, BufMut};
use hid;
use serde_json;
use serde_json::{Value, Error};
use std::time::Duration;
use std::str;

const LOOKING_GLASS_VID:u16 = 0x04D8;
const LOOKING_GLASS_PID:u16 = 0xEf7E;

/// For whatever reason, we only get 64-byte results from read(), but device reports 68 per page...
fn hid_multiread(handle: &mut hid::Handle) -> hid::Result<BytesMut> {
    let mut ret_buf = BytesMut::with_capacity(0);
    loop {
        // On Ubuntu 18.04, we'll either need 64 or 68 bytes, apparently
        // depending on whether libhidapi uses libusb or libhidraw.
        let mut this_read = BytesMut::with_capacity(128);
        this_read.resize(128, 0);

        match handle.data().read(&mut this_read, Duration::from_millis(10))? {
            Some(count) => {
                ret_buf.extend_from_slice(&this_read[..count]);
            },
            None => {
                break;
            },
        }
    }
    Ok(ret_buf)
}

/// Does a single write/read transaction to read a page of data from the LG's EEPROM
fn hid_query(handle: &mut hid::Handle, addr:u16) -> hid::Result<BytesMut> {
    let mut buf = BytesMut::with_capacity(512);

    // Flush the read buffer
    hid_multiread(handle)?;

    buf.put_u16_be(0); // First byte of this is HID "report ID"
    buf.put_u16_be(addr);
    buf.resize(68, 0); // Looking Glass needs a 68-Byte request, unclear why that is

    let count = handle.feature().send(&buf)?;
    if count != buf.len() {
        return Err(hid::Error::Write);
    }

    buf = hid_multiread(handle)?;

    // The first four bytes read should be the same as the four bytes written.
    // Note that the split_to() removes leading 4 bytes from buf.
    let confirm = [0, 0, (addr>>8 & 0xff) as u8, (addr & 0xFF) as u8];
    if buf.len() <= 4 ||
       buf.split_to(4) != Bytes::from(&confirm[..]) {
        println!("Confirm failed!");
        return Err(hid::Error::Read);
    }

    Ok(buf)
}

/// Extracts the JSON-formatted configuration string from LG's EEPROM via HID
fn get_json_string(candidate: hid::Device) -> hid::Result<String> {
    let mut handle = candidate.open()?;

    handle.blocking(true)?;

    // Data is organised in 64-byte pages.  Page 0 starts with 4B of length, followed by that many
    // bytes of JSON-formatted data.  First read will then have the length and some JSON data.
    let json_size_raw = hid_query(&mut handle, 0)?;

    // Can't see how to nicely turn a BytesMut in to a Buf to use get_u32_be()...
    let mut json_size = 0usize;
    for i in 0..4 {
        json_size <<= 8;
        json_size += json_size_raw[i] as usize;
    }

    // Keep the remaining bytes from page 0
    let mut json = Vec::from(&json_size_raw[4..]);

    // Then, read the remaining pages
    while json.len() < json_size {
        let last_page = (json.len() / 64) as u16;
        let this_read = hid_query(&mut handle, last_page + 1)?;

        json.extend_from_slice(&this_read);
    }

    json.truncate(json_size); // Lop off uninitialised garbage

    match str::from_utf8(&json) {
        Ok(yay) => return Ok(yay.to_string()),
        Err(e) => return Err(hid::Error::String(e.to_string()))
    };
}

pub struct LookingGlass {
    /// Serial number based on HID data, as string. This doesn't seem to be used as a serial number?
    pub hid_serial: String,
}

impl LookingGlass {
    fn new() -> LookingGlass {
        LookingGlass {
            hid_serial: String::new(),
        }
    }

    /// Returns a Vec of all Looking Glasses detected
    pub fn findall() -> hid::Result<Vec<LookingGlass>> {
        let mut glasses = Vec::new();

        let hid_manager = hid::init()?;

        for candidate in hid_manager.find(Some(LOOKING_GLASS_VID), Some(LOOKING_GLASS_PID)) {
            let mut glass = LookingGlass::new();

            if let Some(hid_serial) = candidate.serial_number() {
                glass.hid_serial = hid_serial;
            } else {
                return Err(hid::Error::String(
                    "Couldn't read HID serial number - might lack permissions?".to_string()));
            }

            let json_string = get_json_string(candidate)?;
            match serde_json::from_str::<Value>(&json_string) {
                Ok(json) => {
                    println!("DPI is {}", json["DPI"]["value"]);
                    println!("center is {}", json["center"]["value"]);
                    // TODO Use the strongly-typed technique
                    println!("All: {}", json)
                },
                Err(err) => return Err(hid::Error::String(err.to_string())),
            };

            glasses.push(glass);
        }

        Ok(glasses)
    }
}

#[cfg(test)]
mod tests {
    use crate::LookingGlass;

    #[test]
    fn can_find() {
        match LookingGlass::findall() {
            Ok(glasses) => {
                if glasses.is_empty() {
                    println!("Didn't find any Looking Glasses");
                }

                for glass in glasses {
                    println!("\t HID Serial\t {}", glass.hid_serial);
                }
            },
            Err(err) => {
                println!("Got error: {}", err);
                assert!(false);
            }
        }
    }
}
