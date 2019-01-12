# pluton
Tools for working with Looking Glass "holographic" displays

Pretty basic at the moment; can read calibration data from the Looking Glass' EEPROM based on work at https://github.com/lonetech/LookingGlass .

## Installing
Pluton depends on crate "hid", which in turn depends on libhidapi.  On Ubuntu 18.04: `#apt install libhidapi-dev`.

## Example

```
use pluton;

fn main() {
    for glass in pluton::LookingGlass::findall() {
        match glass {
            Ok(glass) => {
                println!("Serial {}:", glass.serial);
                println!("    {}x{} pixels, {} DPI", glass.screen_w, glass.screen_h, glass.dpi);
                println!("    pitch: {}", glass.pitch);
                println!("    slope: {}", glass.slope);
                println!("    center: {}", glass.center);
            },
            Err(error) => {
                println!("Had an error: {}", error.to_string());
            }
        }
    }
}
```
