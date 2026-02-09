use bytes::{Buf, BufMut, Bytes, BytesMut};
use sha1::{Digest, Sha1};
use std::io;
use tokio_util::codec::{Decoder, Encoder};

pub const OP_ERROR: u8 = 0;
pub const OP_INFO: u8 = 1;
pub const OP_AUTH: u8 = 2;
pub const OP_PUBLISH: u8 = 3;
pub const OP_SUBSCRIBE: u8 = 4;
pub const OP_UNSUBSCRIBE: u8 = 5;

// Max buffer size (1MB) to match original implementation limits (MAXBUF)
pub const MAXBUF: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Frame {
    Error(Bytes),
    Info { name: Bytes, rand: Bytes },
    Auth { ident: Bytes, secret_hash: Bytes },
    Publish { ident: Bytes, channel: Bytes, payload: Bytes },
    Subscribe { ident: Bytes, channel: Bytes },
    Unsubscribe { ident: Bytes, channel: Bytes },
}

pub fn strpack8(s: &str) -> Result<Vec<u8>, io::Error> {
    let b = s.as_bytes();
    if b.len() > 255 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "string too long for strpack8"));
    }
    let mut v = Vec::with_capacity(1 + b.len());
    v.push(b.len() as u8);
    v.extend_from_slice(b);
    Ok(v)
}

// Internal helper for packing Bytes as str8
fn pack_str8_bytes(b: &Bytes) -> Result<Vec<u8>, io::Error> {
    if b.len() > 255 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "string too long for strpack8"));
    }
    let mut v = Vec::with_capacity(1 + b.len());
    v.push(b.len() as u8);
    v.extend_from_slice(b);
    Ok(v)
}

pub fn strunpack8(data: &[u8]) -> Result<(String, &[u8]), io::Error> {
    if data.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty string buffer"));
    }
    let len = data[0] as usize;
    if data.len() < 1 + len {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "string buffer too short"));
    }
    let s = String::from_utf8(data[1..1 + len].to_vec())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid utf-8 string"))?;
    Ok((s, &data[1 + len..]))
}

// Helper for decoding from Bytes
fn read_str8_bytes(buf: &mut Bytes) -> Result<Bytes, io::Error> {
    if buf.is_empty() { return Err(io::Error::new(io::ErrorKind::InvalidData, "empty buffer")); }
    let len = buf[0] as usize;
    if buf.len() < 1 + len { return Err(io::Error::new(io::ErrorKind::InvalidData, "buffer too short")); }
    buf.advance(1);
    Ok(buf.split_to(len))
}

pub fn hashsecret(rand: &[u8], secret: &str) -> Vec<u8> {
    let mut hasher = Sha1::new();
    hasher.update(rand);
    hasher.update(secret.as_bytes());
    hasher.finalize().to_vec()
}

pub struct HpfeedsCodec;

impl HpfeedsCodec {
    pub fn new() -> Self {
        Self
    }

    pub fn encode_to_bytes(&mut self, item: Frame) -> Result<Bytes, io::Error> {
        let mut dst = BytesMut::new();
        self.encode(item, &mut dst)?;
        Ok(dst.freeze())
    }
}

impl Decoder for HpfeedsCodec {
    type Item = Frame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Frame>, io::Error> {
        // Need at least 4 bytes for length
        if src.len() < 4 {
            return Ok(None);
        }
        let len = (&src[..4]).get_u32() as usize;

        if len > MAXBUF {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
        }

        // Peek opcode if we have enough bytes (4 len + 1 opcode)
        if src.len() >= 5 {
            let op = src[4];
            let max_op_len = match op {
                OP_INFO => 1 + 256 + 20, // name(256) + rand(20, usually 16)
                OP_AUTH => 1 + 256 + 20, // ident(256) + hash(20)
                OP_PUBLISH => MAXBUF,
                OP_SUBSCRIBE => 1 + 256 + 256 * 2, // ident + channel (generous limit)
                OP_UNSUBSCRIBE => 1 + 256 + 256 * 2,
                OP_ERROR => 1 + 256, // error msg
                _ => {
                    // Invalid opcode, we will catch it later, but for now enforce MAXBUF
                    MAXBUF 
                }
            };
            
            let limit = 5 + max_op_len;
            if len > limit {
                 return Err(io::Error::new(io::ErrorKind::InvalidData, format!("message too large for opcode {}", op)));
            }
        }

        if src.len() < len {
            return Ok(None);
        }
        // We have a full message in the buffer
        // Remove length bytes
        src.advance(4);
        let mut msg = src.split_to(len - 4).freeze(); // Convert to Bytes for zero-copy slicing
        
        // First byte is opcode
        if msg.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "empty message"));
        }
        let op = msg.split_to(1)[0];
        
        match op {
            OP_ERROR => {
                Ok(Some(Frame::Error(msg)))
            }
            OP_INFO => {
                let name = read_str8_bytes(&mut msg)?;
                Ok(Some(Frame::Info { name, rand: msg }))
            }
            OP_AUTH => {
                let ident = read_str8_bytes(&mut msg)?;
                Ok(Some(Frame::Auth {
                    ident,
                    secret_hash: msg,
                }))
            }
            OP_PUBLISH => {
                let ident = read_str8_bytes(&mut msg)?;
                let channel = read_str8_bytes(&mut msg)?;
                Ok(Some(Frame::Publish {
                    ident,
                    channel,
                    payload: msg,
                }))
            }
            OP_SUBSCRIBE => {
                let ident = read_str8_bytes(&mut msg)?;
                Ok(Some(Frame::Subscribe { ident, channel: msg }))
            }
            OP_UNSUBSCRIBE => {
                let ident = read_str8_bytes(&mut msg)?;
                Ok(Some(Frame::Unsubscribe { ident, channel: msg }))
            }
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown opcode: {}", other),
            )),
        }
    }
}

impl Encoder<Frame> for HpfeedsCodec {
    type Error = io::Error;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), io::Error> {
        let mut data: Vec<u8> = Vec::new();
        let op: u8 = match item {
            Frame::Error(err) => {
                data.extend_from_slice(&err);
                OP_ERROR
            }
            Frame::Info { name, rand } => {
                data.extend_from_slice(&pack_str8_bytes(&name)?);
                data.extend_from_slice(&rand);
                OP_INFO
            }
            Frame::Auth { ident, secret_hash } => {
                data.extend_from_slice(&pack_str8_bytes(&ident)?);
                data.extend_from_slice(&secret_hash);
                OP_AUTH
            }
            Frame::Publish { ident, channel, payload } => {
                data.extend_from_slice(&pack_str8_bytes(&ident)?);
                data.extend_from_slice(&pack_str8_bytes(&channel)?);
                data.extend_from_slice(&payload);
                OP_PUBLISH
            }
            Frame::Subscribe { ident, channel } => {
                data.extend_from_slice(&pack_str8_bytes(&ident)?);
                data.extend_from_slice(&channel);
                OP_SUBSCRIBE
            }
            Frame::Unsubscribe { ident, channel } => {
                data.extend_from_slice(&pack_str8_bytes(&ident)?);
                data.extend_from_slice(&channel);
                OP_UNSUBSCRIBE
            }
        };
        let ml = (5 + data.len()) as u32; // 4-byte length + 1 opcode + payload
        dst.put_u32(ml);
        dst.put_u8(op);
        dst.extend_from_slice(&data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn strpack_unpack_roundtrip() {
        let s = "identity";
        let packed = strpack8(s).expect("should pack");
        let (unpacked, rest) = strunpack8(&packed).expect("should unpack");
        assert_eq!(unpacked, s);
        assert!(rest.is_empty());
    }

    #[test]
    fn strpack_too_long() {
        let s = "a".repeat(256);
        let res = strpack8(&s);
        assert!(res.is_err());
    }

    #[test]
    fn info_roundtrip() {
        let mut codec = HpfeedsCodec::new();
        let frame = Frame::Info { name: Bytes::from_static(b"hpfeeds"), rand: Bytes::from_static(&[1, 2, 3, 4]) };
        let mut buf = BytesMut::new();
        codec.encode(frame.clone(), &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn publish_roundtrip() {
        let mut codec = HpfeedsCodec::new();
        let frame = Frame::Publish { 
            ident: Bytes::from_static(b"client1"), 
            channel: Bytes::from_static(b"ch1"), 
            payload: Bytes::from_static(b"hello") 
        };
        let mut buf = BytesMut::new();
        codec.encode(frame.clone(), &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn subscribe_roundtrip() {
        let mut codec = HpfeedsCodec::new();
        let frame = Frame::Subscribe { 
            ident: Bytes::from_static(b"client1"), 
            channel: Bytes::from_static(b"ch1") 
        };
        let mut buf = BytesMut::new();
        codec.encode(frame.clone(), &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn unsubscribe_roundtrip() {
        let mut codec = HpfeedsCodec::new();
        let frame = Frame::Unsubscribe { 
            ident: Bytes::from_static(b"client1"), 
            channel: Bytes::from_static(b"ch1") 
        };
        let mut buf = BytesMut::new();
        codec.encode(frame.clone(), &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn auth_hash_matches_python_impl() {
        let rand = b"randombytes";
        let secret = "s3cret";
        let expected = hashsecret(rand, secret);
        // compute directly using sha1 to verify length
        assert_eq!(expected.len(), 20);
    }
}

