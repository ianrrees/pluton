/// Tools for working with Looking Glass "holographic" displays
/// Ian Rees 2019

use bytes::{Bytes, BytesMut, BufMut};
use hid;
#[macro_use] extern crate serde_derive;
use serde_json;

use std::time::Duration;
use std::str;

const LOOKING_GLASS_VID:u16 = 0x04D8;
const LOOKING_GLASS_PID:u16 = 0xEf7E;

/// For whatever reason, we only get 64-byte results from read(), but device reports 68 per page...
fn hid_multiread(handle: &mut hid::Handle) -> hid::Result<BytesMut> {
    let mut ret_buf = BytesMut::with_capacity(0);
    loop {
        // On Ubuntu 18.04, we'll either need 64 or 68 bytes, apparently (based on Python
        // experiments) depending on whether libhidapi uses libusb or libhidraw.
        let mut this_read = BytesMut::with_capacity(128);
        this_read.resize(128, 0);

        // Magic number warning: the read timeout is just a guess
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
    /// Serial number as reported from EEPROM, not HID.  Seems to be the "real one"
    pub serial: String,
    pub pitch: f32,
    pub slope: f32,
    pub center: f32,
    pub dpi: f32,
    pub screen_w: u32, // Width in pixels
    pub screen_h: u32, // Height in pixels
}

impl LookingGlass {
    pub fn new() -> Self {
        LookingGlass {
            serial: String::new(),
            pitch: 0.0,
            slope: 0.0,
            center: 0.0,
            dpi: 0.0,
            screen_w: 0,
            screen_h: 0,
        }
    }

    /// Returns a Vec of all Looking Glasses detected
    pub fn findall() -> hid::Result<Vec<LookingGlass>> {
        let mut glasses = Vec::new();

        let hid_manager = hid::init()?;

        for candidate in hid_manager.find(Some(LOOKING_GLASS_VID), Some(LOOKING_GLASS_PID)) {
            if let Some(_hid_serial) = candidate.serial_number() {
                // Don't actually care about the serial reported to HID, as it is not unique
            } else {
                return Err(hid::Error::String(
                    "Couldn't query HID device - might lack permissions?".to_string()));
            }

            let json_string = get_json_string(candidate)?;

            // TODO refactor most of the below logic out, so we can test it

            // Example json_string (newlines all added - none sent from Looking Glass):
            // {"configVersion":"1.0","serial":"00297","pitch":{"value":49.81804275512695},
            // "slope":{"value":5.044347763061523},"center":{"value":0.176902174949646},
            // "viewCone":{"value":40.0},"invView":{"value":1.0},"verticalAngle":{"value":0.0},
            // "DPI":{"value":338.0},"screenW":{"value":2560.0},"screenH":{"value":1600.0},
            // "flipImageX":{"value":0.0},"flipImageY":{"value":0.0},"flipSubp":{"value":0.0}}

            #[derive(Serialize, Deserialize)]
            struct JSONValueMap {
                value: f32
            }

            #[derive(Serialize, Deserialize)]
            #[allow(non_snake_case)]
            struct ConfigJSON {
                configVersion: String,
                serial: String,
                pitch: JSONValueMap,
                slope: JSONValueMap,
                center: JSONValueMap,
                viewCone: JSONValueMap,
                invView: JSONValueMap,
                verticalAngle: JSONValueMap,
                DPI: JSONValueMap,
                screenW: JSONValueMap,
                screenH: JSONValueMap,
                flipImageX: JSONValueMap,
                flipImageY: JSONValueMap,
                flipSubp: JSONValueMap,
            }

            match serde_json::from_str::<ConfigJSON>(&json_string) {
                Ok(config) => {
                    if config.configVersion == "1.0" {
                        glasses.push(LookingGlass {
                            serial: config.serial,
                            pitch: config.pitch.value,
                            slope: config.slope.value,
                            center: config.center.value,
                            dpi: config.DPI.value,
                            screen_w: config.screenW.value as u32,
                            screen_h: config.screenH.value as u32,
                        });
                    } else {
                        // TODO better error message here...
                        println!("Don't know how to read config version {}...", config.configVersion);
                        continue;
                    }
                },
                Err(err) => {
                    println!("Error parsing JSON: {}", err.to_string());
                    continue;
                }
            }
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

                let mut index = 0;
                for glass in &glasses {
                    index += 1;
                    println!("{} of {}", index, glasses.len());
                    println!("\t Serial: {}", glass.serial);
                    println!("\t {}x{} pixels, {} DPI", glass.screen_w, glass.screen_h, glass.dpi);
                    println!("\t pitch: {}", glass.pitch);
                    println!("\t slope: {}", glass.slope);
                    println!("\t center: {}", glass.center);
                }
            },
            Err(err) => {
                println!("Got error: {}", err);
                assert!(false);
            }
        }
    }
}
