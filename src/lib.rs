use regex::Regex;
use std::collections::HashMap;
use std::fs::{File, DirEntry};
use std::io::prelude::*;
use std::ops::RangeInclusive;
use std::str::FromStr;

// make the header a bit more easy to handle
const REPLACEMENTS: &'static [(&'static str, &'static str)] = &[
    ("PERIPHS_IO_MUX ", "PERIPHS_IO_MUX_BASE "),
    ("SLC_CONF0", "SLC_CONF0_REG"),
    ("SLC_INT_RAW", "SLC_INT_RAW_REG"),
    ("SLC_INT_STATUS", "SLC_INT_STATUS_REG"),
    ("SLC_INT_ENA", "SLC_INT_ENA_REG"),
    ("SLC_INT_CLR", "SLC_INT_CLR_REG"),
    ("SLC_RX_STATUS", "SLC_RX_STATUS_REG"),
    ("SLC_RX_FIFO_PUSH", "SLC_RX_FIFO_PUSH_REG"),
    ("SLC_TX_STATUS", "SLC_TX_STATUS_REG"),
    ("SLC_TX_FIFO_POP", "SLC_TX_FIFO_POP_REG"),
    ("SLC_RX_LINK", "SLC_RX_LINK_REG"),
    ("RTC_STORE0", "RTC_STORE0_REG"),
    ("RTC_STATE1", "RTC_STATE1_REG"),
    ("RTC_STATE2", "RTC_STATE2_REG"),
];

/* Regex's to find all the peripheral addresses */
pub const REG_BASE: &'static str = r"\#define[\s*]+(?:DR_REG|REG|PERIPHS)_(.*)_BASE(?:A?DDR)?[\s*]+0x([0-9a-fA-F]+)";
pub const REG_DEF: &'static str = r"\#define[\s*]+(?:PERIPHS_)?([^\s*]+)_(?:REG|ADDRESS|U)[\s*]+\((?:DR_REG|REG|PERIPHS)_(.*)_BASE(?:A?DDR)? \+ (.*)\)";
pub const REG_DEF_OFFSET: &'static str = r"\#define[\s*]+(?:PERIPHS_)?([^\s*]+)_(?:ADDRESS|U)[\s*]+(?:0x)?([0-9a-fA-F]+)";
pub const REG_DEF_INDEX: &'static str =
    r"\#define[\s*]+(?:PERIPHS_)?([^\s*]+)_(?:REG|ADDRESS|U)\(i\)[\s*]+\((?:DR_REG|REG|PERIPHS)_([0-9A-Za-z_]+)_BASE(?:A?DDR)?[\s*]*\(i\) \+ (.*?)\)";
pub const REG_DEFINE_MASK: &'static str =
    r"\#define[\s*]+(?:PERIPHS_)?([^\s*]+)[\s*]+\(?(0x[0-9a-fA-F]+|[0-9]+|\(?BIT\(?[0-9]+\)?)\)?\)?";
pub const REG_DEFINE_SHIFT: &'static str =
    r"\#define[\s*]+(?:PERIPHS_)?([^\s*]+)_(?:S|s)[\s*]+\(?(0x[0-9a-fA-F]+|[0-9]+)\)?";
pub const REG_DEFINE_SKIP: &'static str =
    r"\#define[\s*]+(?:PERIPHS_)?([^\s*]+)_(?:M|V)[\s*]+(\(|0x)";
pub const SINGLE_BIT: &'static str = r"BIT\(?([0-9]+)\)?";
pub const INTERRUPTS: &'static str =
    r"\#define[\s]ETS_([0-9A-Za-z_/]+)_SOURCE[\s]+([0-9]+)/\*\*<\s([0-9A-Za-z_/\s,]+)\*/";
pub const REG_IFDEF: &'static str = r"#ifn?def.*";
pub const REG_ENDIF: &'static str = r"#endif";

#[derive(Debug, Default, Clone)]
pub struct Peripheral {
    pub description: String,
    pub address: u32,
    pub registers: Vec<Register>,
}

#[derive(Clone, Debug, Default)]
pub struct Interrupt {
    pub name: String,
    pub description: Option<String>,
    pub value: u32,
}

#[derive(Debug, Default, Clone)]
pub struct Register {
    /// Register Name
    pub name: String,
    /// Relative Address
    pub address: u32,
    /// Width
    pub width: u8,
    /// Description
    pub description: String,
    /// Reset Value
    pub reset_value: u64,
    /// Detailed description
    pub detailed_description: Option<String>,
    pub bit_fields: Vec<BitField>,
}

#[derive(Debug, Default, Clone)]
pub struct BitField {
    /// Field Name
    pub name: String,
    /// Bits
    pub bits: Bits,
    /// Type
    pub type_: Type,
    /// Reset Value
    pub reset_value: u32,
    /// Description
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum Bits {
    Single(u8),
    Range(RangeInclusive<u8>),
}

impl Default for Bits {
    fn default() -> Self {
        Bits::Single(0)
    }
}

use svd_parser::Access;

#[derive(Debug, Copy, Clone)]
pub enum Type {
    // ReadAsZero,
    ReadOnly,
    ReadWrite,
    WriteOnly,
    // ReadWriteSetOnly,
    // ReadableClearOnRead,
    // ReadableClearOnWrite,
    // WriteAsZero,
    // WriteToClear,
}

impl From<Type> for Access {
    fn from(t: Type) -> Self {
        match t {
            Type::ReadOnly => Access::ReadOnly,
            Type::ReadWrite => Access::ReadWrite,
            Type::WriteOnly => Access::WriteOnly,
        }
    }
}

impl Default for Type {
    fn default() -> Type {
        Type::ReadWrite
    }
}

impl FromStr for Type {
    type Err = String;

    fn from_str(s: &str) -> Result<Type, Self::Err> {
        Ok(match s {
            "RO" | "R/O" => Type::ReadOnly,
            "RW" | "R/W" => Type::ReadWrite,
            "WO" | "W/O" => Type::WriteOnly,
            _ => return Err(String::from("Invalid BitField type: ") + &String::from(s)),
        })
    }
}

enum State {
    FindReg,
    FindBitFieldMask(String, Register),
    FindBitFieldShift(String, Register, u32),
    FindBitFieldSkipShift(String, Register),
    AssumeFullRegister(String, Register),
    CheckEnd(String, Register),
    End(String, Register),
}

fn add_base_addr(header: &str, peripherals: &mut HashMap<String, Peripheral>) {
    let re_base = Regex::new(REG_BASE).unwrap();
    /* Peripheral base addresses */
    for captures in re_base.captures_iter(header) {
        let peripheral = &captures[1];
        let address = &captures[2];
        let mut p = Peripheral::default();
        p.address = u32::from_str_radix(address, 16).unwrap();
        p.description = peripheral.to_string();

        if !peripherals.contains_key(peripheral) {
            peripherals.insert(peripheral.to_string(), p);
        }
    }
}

pub fn parse_idf(path: &str) -> HashMap<String, Peripheral> {
    let mut peripherals = HashMap::new();
    let mut invalid_peripherals = vec![];
    let mut invalid_files = vec![];
    let mut invalid_registers = vec![];
    // let mut invalid_bit_fields = vec![];

    let mut interrupts = vec![];

    let filname = path.to_owned() + "eagle_soc.h";
    let re_reg = Regex::new(REG_DEF).unwrap();
    let re_reg_index = Regex::new(REG_DEF_INDEX).unwrap();
    let re_reg_offset = Regex::new(REG_DEF_OFFSET).unwrap();
    let re_reg_define = Regex::new(REG_DEFINE_MASK).unwrap();
    let re_reg_define_shift = Regex::new(REG_DEFINE_SHIFT).unwrap();
    let re_interrupts = Regex::new(INTERRUPTS).unwrap();
    let re_single_bit = Regex::new(SINGLE_BIT).unwrap();
    let re_reg_skip = Regex::new(REG_DEFINE_SKIP).unwrap();
    let re_ifdef = Regex::new(REG_IFDEF).unwrap();
    let re_endif = Regex::new(REG_ENDIF).unwrap();

    let soc_h = file_to_string(&filname);

    for captures in re_interrupts.captures_iter(soc_h.as_str()) {
        let name = &captures[1];
        let index = &captures[2];
        let desc = &captures[3];
        let intr = Interrupt {
            name: name.to_string(),
            description: Some(desc.to_string()),
            value: index.parse().unwrap(),
        };
        interrupts.push(intr);
        // println!("{:#?}", intr);
    }

    /*
       Theses are indexed, we seed these as they cannot be derived from the docs
       These blocks are identical, so we need to do some post processing to properly index
       and offset these
    */
    // peripherals.insert("I2C".to_string(), Peripheral::default());
    // peripherals.insert("SPI".to_string(), Peripheral::default());
    // peripherals.insert("TIMG".to_string(), Peripheral::default());
    // peripherals.insert("MCPWM".to_string(), Peripheral::default());
    // peripherals.insert("UHCI".to_string(), Peripheral::default());

    add_base_addr(&soc_h, &mut peripherals);

    std::fs::read_dir(path)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|f| f.path().to_str().unwrap().ends_with("_register.h") || f.file_name().to_str().unwrap() == "eagle_soc.h")
        .for_each(|f| {
            let name = f.path();
            let name = name.to_str().unwrap();
            // let mut buffer = vec![];
            let mut file_data = file_to_string(name);
            for (search, replace) in REPLACEMENTS {
                file_data = file_data.replace(search, replace);
            }

            add_base_addr(&file_data, &mut peripherals);

            // println!("Searching {}", name);
            let mut something_found = false;
            let mut state = State::FindReg;
            let mut in_ifdef = false;
            for (i, line) in file_data.lines().enumerate() {
                if re_ifdef.is_match(line) {
                    continue;
                } else if re_endif.is_match(line) {
                    continue;
                }

                loop {
                    match state {
                        State::FindReg => {
                            /* Normal register definitions */
                            if let Some(m) = re_reg.captures(line) {
                                let reg_name = &m[1];
                                let pname = &m[2];
                                let offset = &m[3].trim_start_matches("0x");
                                if reg_name.ends_with("(i)") {
                                    invalid_registers.push(reg_name.to_string());
                                    // some indexed still get through, ignore them
                                    break;
                                }
                                if let Ok(addr) = u32::from_str_radix(offset, 16) {
                                    let mut r = Register::default();
                                    r.description = reg_name.to_string();
                                    r.name = reg_name.to_string();
                                    r.address = addr;
                                    state = State::FindBitFieldMask(pname.to_string(), r);
                                } else {
                                    invalid_registers.push(reg_name.to_string());
                                }
                            } else if let Some(m) = re_reg_index.captures(line) {
                                let reg_name = &m[1];
                                let pname = &m[2];
                                let offset = &m[3].trim_start_matches("0x");

                                if let Ok(addr) = u32::from_str_radix(offset, 16) {
                                    let mut r = Register::default();
                                    r.name = reg_name.to_string();
                                    r.description = reg_name.to_string();
                                    r.address = addr;
                                    state = State::FindBitFieldMask(pname.to_string(), r);
                                } else {
                                    invalid_registers.push(reg_name.to_string());
                                }
                            } else if let Some(m) = re_reg_offset.captures(line) {
                                let reg_name = &m[1];
                                let offset = &m[2];
                                let pname = reg_name.split('_').next().unwrap();

                                if let Ok(addr) = u32::from_str_radix(offset, 16) {
                                    let mut r = Register::default();
                                    r.name = reg_name.to_string();
                                    r.description = reg_name.to_string();
                                    r.address = addr;
                                    state = State::FindBitFieldMask(pname.to_string(), r);
                                } else {
                                    invalid_registers.push(reg_name.to_string());
                                }
                            }
                            break; // next line
                        }
                        State::AssumeFullRegister(ref mut pname, ref mut reg) => {
                            something_found = true;
                            // assume full 32bit wide field
                            let bitfield = BitField {
                                name: "Register".to_string(),
                                bits: Bits::Range(0..=31),
                                ..Default::default()
                            };
                            reg.bit_fields.push(bitfield);

                            if let Some(p) = peripherals.get_mut(&pname.to_string()) {
                                p.registers.push(reg.clone());
                            } else {
                                invalid_peripherals.push(pname.to_string());
                            }
                            state = State::FindReg;
                        }
                        State::FindBitFieldMask(ref mut pname, ref mut reg) => {
                            if re_reg_skip.is_match(line) {
                                break;
                            }

                            if re_reg_offset.is_match(line) {
                                state = State::AssumeFullRegister(pname.clone(), reg.clone());
                                continue;
                            }
                            if let Some(m) = re_reg_define.captures(line) {
                                something_found = true;
                                let define_name = &m[1];
                                let value = &m[2].trim_start_matches("0x");

                                if let Some(m) = re_single_bit.captures(value) {
                                    if let Ok(mask_bit) = u8::from_str_radix(&m[1], 10) {
                                        let bitfield = BitField {
                                            name: define_name.to_string(),
                                            bits: Bits::Single(mask_bit),
                                            ..Default::default()
                                        };
                                        reg.bit_fields.push(bitfield);
                                        state = State::FindBitFieldSkipShift(pname.clone(), reg.clone());
                                        break;
                                    } else {
                                        println!("Failed to single bit match reg mask at {}:{}", name, i);
                                        state = State::FindReg;
                                    }
                                } else if let Ok(mask) = u32::from_str_radix(value, 16) {
                                    state = State::FindBitFieldShift(pname.clone(), reg.clone(), mask);
                                }
                            } else {
                                if reg.bit_fields.is_empty() {
                                    state = State::AssumeFullRegister(pname.clone(), reg.clone());
                                    continue;
                                } else {
                                    println!("Failed to match reg mask at {}:{}", name, i);
                                    state = State::End(pname.clone(), reg.clone());
                                }
                            }
                            break; // next line
                        }
                        State::FindBitFieldShift(ref mut pname, ref mut reg, ref mut mask) => {
                            if re_reg_skip.is_match(line) {
                                break;
                            }
                            if let Some(m) = re_reg_define_shift.captures(line) {
                                let define_name = &m[1];
                                let value = &m[2];

                                if let Ok(shift) = u8::from_str_radix(value, 10) {
                                    let bitfield = BitField {
                                        name: define_name.to_string(),
                                        bits: match mask.count_ones() {
                                            1 => Bits::Single(shift),
                                            bits => Bits::Range(shift..=shift + (bits - 1) as u8)
                                        },
                                        ..Default::default()
                                    };
                                    reg.bit_fields.push(bitfield);
                                    state = State::CheckEnd(pname.clone(), reg.clone())
                                }
                            } else {
                                if reg.bit_fields.is_empty() {
                                    state = State::AssumeFullRegister(pname.clone(), reg.clone());
                                    continue;
                                } else {
                                    println!("Failed to match reg shift at {}:{} ('{}')", name, i, line);
                                    state = State::End(pname.clone(), reg.clone());
                                }
                            }
                            break; // next line
                        }
                        State::FindBitFieldSkipShift(ref mut pname, ref mut reg) => {
                            state = State::CheckEnd(pname.clone(), reg.clone());
                            if re_reg_define_shift.is_match(line) {
                                break;
                            }
                        }
                        State::CheckEnd(ref mut pname, ref mut reg) => {
                            if line.is_empty() {
                                state = State::End(pname.clone(), reg.clone());
                                break;
                            } else if re_reg_define.is_match(line) {
                                // we've found the next bit field in the reg
                                state = State::FindBitFieldMask(pname.clone(), reg.clone());
                            } else {
                                break; // next line
                            }
                        }
                        State::End(ref mut pname, ref mut reg) => {
                            if let Some(p) = peripherals.get_mut(&pname.to_string()) {
                                p.registers.push(reg.clone());
                            } else {
                                // TODO indexed peripherals wont come up here
                                // println!("No periphal called {}", pname.to_string());
                                invalid_peripherals.push(pname.to_string());
                            }
                            state = State::FindReg;
                        }
                    }
                }
            }

            // log if nothing was parsed in this file
            if !something_found {
                invalid_files.push(String::from(name))
            }
        });

    println!("Parsed idf for peripherals information.");

    if invalid_files.len() > 0 {
        println!(
            "The following files contained no parsable information {:?}",
            invalid_files
        );
    }

    if invalid_peripherals.len() > 0 {
        println!(
            "The following peripherals failed to parse {:?}",
            invalid_peripherals
        );
    }

    if invalid_registers.len() > 0 {
        println!(
            "The following registers failed to parse {:?}",
            invalid_registers
        );
    }

    // if invalid_bit_fields.len() > 0 {
    //     println!(
    //         "The following bit_fields failed to parse {:?}",
    //         invalid_bit_fields
    //     );
    // }

    // println!("Interrupt information: {:#?}", interrupts);

    peripherals
}

fn file_to_string(fil: &str) -> String {
    let mut soc = File::open(fil).unwrap();
    let mut data = String::new();
    soc.read_to_string(&mut data).unwrap();
    data
}
