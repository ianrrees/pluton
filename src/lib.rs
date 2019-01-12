/// Tools for working with Looking Glass "holographic" displays
/// Ian Rees 2019

use bytes::{Bytes, BytesMut, BufMut};
use hid;
#[macro_use] extern crate serde_derive;
use serde_json;

use std::{error, fmt, str, time};

const LOOKING_GLASS_VID:u16 = 0x04D8;
const LOOKING_GLASS_PID:u16 = 0xEf7E;

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

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    HIDError(String),
    ParseError(String)
}

pub type Result<T> = ::std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(error::Error::description(self))
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            Error::HIDError(ref err) => err,
            Error::ParseError(ref err) => err,
        }
    }
}

impl LookingGlass {
    /// Returns a Vec of all Looking Glass candidates: each is Result<LookingGlass>
    pub fn findall() -> Vec<Result<LookingGlass>> {
        let mut glasses = Vec::new();

        let hid_manager = match hid::init() {
            Ok(manager) => manager,
            Err(error) => {
                glasses.push(Err(Error::HIDError(error.to_string())));
                return glasses;
            }
        };

        for candidate in hid_manager.find(Some(LOOKING_GLASS_VID), Some(LOOKING_GLASS_PID)) {
            glasses.push(
                if let Some(_hid_serial) = candidate.serial_number() {
                    // Don't actually care about the serial reported to HID, as it is not unique

                    match get_json_string(candidate) {
                        Ok(string) => json_to_glass(string),
                        Err(error) => Err(Error::HIDError(error.to_string()))
                    }
                } else {
                    Err(Error::HIDError("Error reading - may lack permissions?".to_string()))
                }
            );
        }

        glasses
    }
}

/// Parses JSON config string and instantiates a LookingGlass as appropriate
fn json_to_glass(json_string: String) -> Result<LookingGlass>
{
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
                Ok(LookingGlass {
                    serial: config.serial,
                    pitch: config.pitch.value,
                    slope: config.slope.value,
                    center: config.center.value,
                    dpi: config.DPI.value,
                    screen_w: config.screenW.value as u32,
                    screen_h: config.screenH.value as u32,
                })
            } else {
                Err(Error::ParseError(format!("Don't know how to read config version {}...",
                    config.configVersion)))
            }
        },
        Err(error) => {
            Err(Error::ParseError(format!("Error parsing JSON: {}", error.to_string())))
        }
    }
}

/// For whatever reason, we only get 64-byte results from read(), but device reports 68 per page...
fn hid_multiread(handle: &mut hid::Handle) -> hid::Result<BytesMut> {
    let mut ret_buf = BytesMut::with_capacity(0);
    loop {
        // On Ubuntu 18.04, we'll either need 64 or 68 bytes, apparently (based on Python
        // experiments) depending on whether libhidapi uses libusb or libhidraw.
        let mut this_read = BytesMut::with_capacity(128);
        this_read.resize(128, 0);

        // Magic number warning: the read timeout is just a guess
        match handle.data().read(&mut this_read, time::Duration::from_millis(10))? {
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
        Ok(yay) => Ok(yay.to_string()),
        Err(error) => Err(hid::Error::String(error.to_string()))
    }
}


#[cfg(test)]
mod tests {
    use super::*; // Brings in private methods too

    #[test]
    fn can_find() {
        let glasses = LookingGlass::findall();
        if glasses.is_empty() {
            println!("Didn't find any Looking Glasses");
        } else {
            println!("Looking Glasses found:");
        }

        let mut index = 0;
        for glass in &glasses {
            index += 1;
            println!("{} of {}", index, glasses.len());

            match glass {
                Ok(glass) => {
                    println!("\t Serial: {}", glass.serial);
                    println!("\t {}x{} pixels, {} DPI", glass.screen_w, glass.screen_h, glass.dpi);
                    println!("\t pitch: {}", glass.pitch);
                    println!("\t slope: {}", glass.slope);
                    println!("\t center: {}", glass.center);
                },
                Err(error) => {
                    println!("\t Error: {}", error.to_string());
                    assert!(false);
                }
            }
        }
    }

    #[test]
    fn test_json_to_glass_v1() {
        let json = concat!(
            r#"{"configVersion":"1.0","serial":"00297","pitch":{"value":49.81804275512695},"#,
            r#""slope":{"value":5.044347763061523},"center":{"value":0.176902174949646},"#,
            r#""viewCone":{"value":40.0},"invView":{"value":1.0},"verticalAngle":{"value":0.0},"#,
            r#""DPI":{"value":338.0},"screenW":{"value":2560.0},"screenH":{"value":1600.0},"#,
            r#""flipImageX":{"value":0.0},"flipImageY":{"value":0.0},"flipSubp":{"value":0.0}}"#
            ).to_string();

        match json_to_glass(json) {
            Ok(glass) => {
                // Not sure that comparing for equality with floats here is a great idea...
                assert_eq!(glass.serial, "00297");
                assert_eq!(glass.pitch, 49.81804275512695);
                assert_eq!(glass.slope, 5.044347763061523);
                assert_eq!(glass.center, 0.176902174949646);
                assert_eq!(glass.dpi, 338.0);
                assert_eq!(glass.screen_w, 2560);
                assert_eq!(glass.screen_h, 1600);
            },
            Err(..) => {
                assert!(false);
            }
        }
    }
}
