use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct OscPacket {
    pub address: String,
    pub arguments: Vec<OscArgument>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OscArgument {
    Float(f32),
    Int(i32),
    Bool(bool),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OscError(String);

impl fmt::Display for OscError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub fn parse_packet(bytes: &[u8]) -> Result<OscPacket, OscError> {
    let (address, mut offset) = read_osc_string(bytes, 0)?;
    if !address.starts_with('/') {
        return Err(OscError("OSC address must start with /".into()));
    }

    let (types, next_offset) = read_osc_string(bytes, offset)?;
    offset = next_offset;
    if !types.starts_with(',') {
        return Err(OscError("OSC type tag must start with ,".into()));
    }

    let mut arguments = Vec::new();
    for tag in types[1..].chars() {
        match tag {
            'f' => {
                let (value, next) = read_f32(bytes, offset)?;
                offset = next;
                arguments.push(OscArgument::Float(value));
            }
            'i' => {
                let (value, next) = read_i32(bytes, offset)?;
                offset = next;
                arguments.push(OscArgument::Int(value));
            }
            's' => {
                let (value, next) = read_osc_string(bytes, offset)?;
                offset = next;
                arguments.push(OscArgument::String(value));
            }
            'T' => arguments.push(OscArgument::Bool(true)),
            'F' => arguments.push(OscArgument::Bool(false)),
            ignored => return Err(OscError(format!("unsupported OSC type tag: {ignored}"))),
        }
    }

    Ok(OscPacket { address, arguments })
}

pub fn encode_int_message(address: &str, value: i32) -> Result<Vec<u8>, OscError> {
    if !address.starts_with('/') {
        return Err(OscError("OSC address must start with /".into()));
    }

    let mut bytes = Vec::new();
    write_osc_string(&mut bytes, address);
    write_osc_string(&mut bytes, ",i");
    bytes.extend_from_slice(&value.to_be_bytes());
    Ok(bytes)
}

fn write_osc_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}

fn read_osc_string(bytes: &[u8], offset: usize) -> Result<(String, usize), OscError> {
    if offset >= bytes.len() {
        return Err(OscError("unexpected end of packet".into()));
    }

    let Some(relative_end) = bytes[offset..].iter().position(|byte| *byte == 0) else {
        return Err(OscError("unterminated OSC string".into()));
    };
    let end = offset + relative_end;
    let value = std::str::from_utf8(&bytes[offset..end])
        .map_err(|_| OscError("OSC string is not UTF-8".into()))?
        .to_string();
    Ok((value, align4(end + 1)))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<(i32, usize), OscError> {
    let chunk = read_4(bytes, offset)?;
    Ok((i32::from_be_bytes(chunk), offset + 4))
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<(f32, usize), OscError> {
    let chunk = read_4(bytes, offset)?;
    Ok((f32::from_bits(u32::from_be_bytes(chunk)), offset + 4))
}

fn read_4(bytes: &[u8], offset: usize) -> Result<[u8; 4], OscError> {
    bytes
        .get(offset..offset + 4)
        .ok_or_else(|| OscError("unexpected end of packet".into()))?
        .try_into()
        .map_err(|_| OscError("invalid 4-byte OSC value".into()))
}

const fn align4(value: usize) -> usize {
    (value + 3) & !3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_float_message() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"/haptics/left\0\0\0");
        bytes.extend_from_slice(b",f\0\0");
        bytes.extend_from_slice(&0.75_f32.to_bits().to_be_bytes());

        let packet = parse_packet(&bytes).unwrap();
        assert_eq!(packet.address, "/haptics/left");
        assert_eq!(packet.arguments, vec![OscArgument::Float(0.75)]);
    }

    #[test]
    fn parses_true_message_without_payload() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"/haptics/right\0\0");
        bytes.extend_from_slice(b",T\0\0");

        let packet = parse_packet(&bytes).unwrap();
        assert_eq!(packet.arguments, vec![OscArgument::Bool(true)]);
    }

    #[test]
    fn encodes_int_message() {
        let bytes = encode_int_message("/status/heartbeat", 3).unwrap();
        let packet = parse_packet(&bytes).unwrap();

        assert_eq!(packet.address, "/status/heartbeat");
        assert_eq!(packet.arguments, vec![OscArgument::Int(3)]);
    }
}
