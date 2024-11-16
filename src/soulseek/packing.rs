use std::net::Ipv4Addr;

use crate::error::Result;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

pub trait WriteSoulSeekBytes: WriteBytesExt {
    fn pack_u16(&mut self, n: u16) -> Result<()> {
        self.write_u16::<LittleEndian>(n)?;
        Ok(())
    }

    fn pack_u32(&mut self, n: u32) -> Result<()> {
        self.write_u32::<LittleEndian>(n)?;
        Ok(())
    }

    fn pack_bool(&mut self, b: bool) -> Result<()> {
        self.write_u8(b as u8)?;
        Ok(())
    }

    fn pack_string(&mut self, s: String) -> Result<()> {
        self.write_u32::<LittleEndian>(s.len() as u32)?;
        self.write_all(s.as_bytes())?;
        Ok(())
    }
}

impl<W: WriteBytesExt> WriteSoulSeekBytes for W {}

pub trait ReadSoulSeekBytes: ReadBytesExt {
    fn unpack_u16(&mut self) -> Result<u16> {
        let n = self.read_u16::<LittleEndian>()?;
        Ok(n)
    }

    fn unpack_u32(&mut self) -> Result<u32> {
        let n = self.read_u32::<LittleEndian>()?;
        Ok(n)
    }

    fn unpack_u64(&mut self) -> Result<u64> {
        let n = self.read_u64::<LittleEndian>()?;
        Ok(n)
    }

    fn unpack_bool(&mut self) -> Result<bool> {
        let b = self.read_u8()? == 1;
        Ok(b)
    }

    fn unpack_string(&mut self) -> Result<String> {
        let n = self.read_u32::<LittleEndian>()?;
        let mut buf = vec![0; n as usize];
        self.read_exact(&mut buf)?;
        Ok(String::from_utf8(buf)?)
    }

    fn unpack_ipv4(&mut self) -> Result<Ipv4Addr> {
        let n = self.read_u32::<LittleEndian>()?;
        Ok(Ipv4Addr::from(n))
    }
}

impl<R: ReadBytesExt> ReadSoulSeekBytes for R {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pack() -> Result<()> {
        const EXPECTED: [u8; 9] = [0x05, 0x00, 0x00, 0x00, 0x48, 0x65, 0x6c, 0x6c, 0x6f];

        let mut pack: Vec<u8> = vec![];
        pack.pack_string("Hello".into())?;

        assert_eq!(EXPECTED, pack.as_slice());
        Ok(())
    }

    #[tokio::test]
    async fn test_unpack() -> Result<()> {
        let expected: String = "Hello".into();
        let msg = vec![0x05, 0x00, 0x00, 0x00, 0x48, 0x65, 0x6c, 0x6c, 0x6f];

        let actual = msg.as_slice().unpack_string()?;

        assert_eq!(expected, actual);
        Ok(())
    }
}
